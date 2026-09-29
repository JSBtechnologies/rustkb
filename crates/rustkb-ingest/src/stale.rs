//! Drift detection: which curated guidance may no longer match reality?
//!
//! Signals, strongest first:
//! * a recommended crate is flagged unmaintained/unsound by `RustSec`;
//! * a recommended crate had a semver-incompatible release since the doc was verified;
//! * the doc was verified against a Rust release many minors ago;
//! * the doc hasn't been re-verified for a long time;
//! * a catalog crate looks abandoned (no release in years) or its newest release is yanked.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use rustkb_core::{Catalog, Tier, is_semver_breaking, rust_minor};
use serde::Serialize;

use crate::advisories::Advisory;
use crate::cratesio::CrateInfo;
use crate::curated::Report;

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub severity: Severity,
    /// Reference file (relative path) or `catalog.toml`.
    pub location: String,
    pub subject: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct Thresholds {
    pub rust_minors_behind: u64,
    pub max_age_days: i64,
    pub abandoned_after_days: i64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            rust_minors_behind: 6,
            max_age_days: 180,
            abandoned_after_days: 730,
        }
    }
}

/// Every crate name referenced by curated docs or the catalog.
pub fn referenced_crates(report: &Report, catalog: &Catalog) -> Vec<String> {
    let mut names: BTreeSet<String> = report
        .files
        .iter()
        .flat_map(|f| f.meta.crates.keys().cloned())
        .collect();
    names.extend(
        catalog
            .crates
            .iter()
            .filter(|c| c.category != "tool" || c.track_docs)
            .map(|c| c.name.clone()),
    );
    names.extend(
        catalog
            .crates
            .iter()
            .filter(|c| c.category == "tool")
            .map(|c| c.name.clone()),
    );
    names.into_iter().collect()
}

#[derive(Debug)]
pub struct Inputs<'a> {
    pub report: &'a Report,
    pub catalog: &'a Catalog,
    pub crates: &'a BTreeMap<String, CrateInfo>,
    pub advisories: &'a [Advisory],
    /// Latest stable Rust, e.g. `1.96.0`.
    pub latest_rust: Option<&'a str>,
    pub today: jiff::civil::Date,
    pub thresholds: Thresholds,
}

pub fn check(i: &Inputs<'_>) -> Vec<Finding> {
    let mut out = Vec::new();
    let unmaintained = flagged_crates(i.advisories);

    for f in &i.report.files {
        let m = &f.meta;
        for (name, verified) in &m.crates {
            if let Some((kind, id)) = unmaintained.get(&norm(name)) {
                out.push(Finding {
                    severity: Severity::High,
                    location: f.rel.clone(),
                    subject: name.clone(),
                    message: format!("recommends `{name}` but RustSec flags it {kind} ({id})"),
                });
            }
            let Some(latest) = i
                .crates
                .get(name)
                .and_then(|c| c.max_stable_version.as_deref())
            else {
                continue;
            };
            if is_semver_breaking(verified, latest) == Some(true) {
                out.push(Finding {
                    severity: Severity::High,
                    location: f.rel.clone(),
                    subject: name.clone(),
                    message: format!("verified against {name} {verified}, latest is {latest} (semver-incompatible)"),
                });
            }
        }
        if let (Some(latest), Some(verified)) =
            (i.latest_rust.and_then(rust_minor), rust_minor(&m.rust))
        {
            let behind = latest.saturating_sub(verified);
            if behind >= i.thresholds.rust_minors_behind {
                out.push(Finding {
                    severity: Severity::Medium,
                    location: f.rel.clone(),
                    subject: "rust".into(),
                    message: format!(
                        "verified on Rust 1.{verified}, now 1.{latest} ({behind} releases behind)"
                    ),
                });
            }
        }
        if let Ok(date) = jiff::civil::Date::strptime("%Y-%m-%d", &m.verified)
            && let Ok(span) = i.today.since(date)
        {
            let days = i64::from(span.get_days());
            if days > i.thresholds.max_age_days {
                out.push(Finding {
                    severity: Severity::Low,
                    location: f.rel.clone(),
                    subject: "verified".into(),
                    message: format!("last verified {} ({days} days ago)", m.verified),
                });
            }
        }
    }

    for c in &i.catalog.crates {
        let recommended = c.tier != Tier::Avoid;
        if recommended && let Some((kind, id)) = unmaintained.get(&norm(&c.name)) {
            out.push(Finding {
                severity: Severity::High,
                location: "catalog.toml".into(),
                subject: c.name.clone(),
                message: format!(
                    "tier `{}` but RustSec flags it {kind} ({id}) — demote to `avoid`?",
                    c.tier.as_str()
                ),
            });
        }
        let Some(info) = i.crates.get(&c.name) else {
            continue;
        };
        if let Some(latest) = info.max_stable_version.as_deref()
            && is_semver_breaking(&c.version, latest) == Some(true)
        {
            out.push(Finding {
                severity: if recommended {
                    Severity::High
                } else {
                    Severity::Low
                },
                location: "catalog.toml".into(),
                subject: c.name.clone(),
                message: format!(
                    "catalog says {}, latest is {latest} (semver-incompatible)",
                    c.version
                ),
            });
        }
        if recommended && info.newest_yanked {
            out.push(Finding {
                severity: Severity::Medium,
                location: "catalog.toml".into(),
                subject: c.name.clone(),
                message: format!(
                    "newest release {} is yanked",
                    info.newest_version.as_deref().unwrap_or("?")
                ),
            });
        }
        if recommended
            && let Some(updated) = info.last_release.as_deref().and_then(|u| u.get(..10))
            && let Ok(date) = jiff::civil::Date::strptime("%Y-%m-%d", updated)
            && let Ok(span) = i.today.since(date)
            && i64::from(span.get_days()) > i.thresholds.abandoned_after_days
        {
            out.push(Finding {
                severity: Severity::Medium,
                location: "catalog.toml".into(),
                subject: c.name.clone(),
                message: format!(
                    "no release since {updated} — check it is still maintained (or finished)"
                ),
            });
        }
    }

    out.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.location.cmp(&b.location))
    });
    out
}

/// Crates with a live (non-withdrawn) unmaintained/unsound advisory affecting *all* versions
/// (i.e. no patched release), keyed by normalised name.
fn flagged_crates(advisories: &[Advisory]) -> BTreeMap<String, (String, String)> {
    advisories
        .iter()
        .filter(|a| {
            !a.withdrawn
                && a.patched.is_empty()
                && matches!(a.kind.as_str(), "unmaintained" | "unsound")
        })
        .map(|a| (norm(&a.krate), (a.kind.clone(), a.id.clone())))
        .collect()
}

fn norm(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}

/// Markdown report suitable for a GitHub issue body.
pub fn to_markdown(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return "No drift detected. ✅\n".into();
    }
    let mut s = String::from("| Severity | Location | Subject | Finding |\n|---|---|---|---|\n");
    for f in findings {
        let _ = writeln!(
            s,
            "| {:?} | `{}` | `{}` | {} |",
            f.severity,
            f.location,
            f.subject,
            f.message.replace('|', "\\|")
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustkb_core::{CatalogEntry, RefMeta};

    fn info(name: &str, latest: &str, updated: &str) -> CrateInfo {
        CrateInfo {
            name: name.into(),
            max_stable_version: Some(latest.into()),
            newest_version: Some(latest.into()),
            description: None,
            repository: None,
            documentation: None,
            downloads: None,
            recent_downloads: None,
            updated_at: None,
            last_release: Some(updated.into()),
            created_at: None,
            newest_yanked: false,
            rust_version: None,
            license: None,
        }
    }

    #[test]
    fn detects_drift() {
        let meta = RefMeta {
            id: "idioms/x".into(),
            title: "X".into(),
            summary: "s".into(),
            area: "idioms".into(),
            tags: vec![],
            rust: "1.80".into(),
            edition: None,
            crates: [
                ("thiserror".to_owned(), "1.0".to_owned()),
                ("oldcrate".to_owned(), "0.3".to_owned()),
            ]
            .into(),
            verified: "2025-01-01".into(),
            sources: vec![],
        };
        let report = Report {
            files: vec![crate::curated::RefFile {
                path: "x.md".into(),
                rel: "skills/rust-idioms/references/x.md".into(),
                skill: "rust-idioms".into(),
                meta,
                body: String::new(),
            }],
            errors: vec![],
        };
        let catalog = Catalog {
            crates: vec![CatalogEntry {
                name: "oldcrate".into(),
                category: "misc".into(),
                tier: Tier::Recommended,
                version: "0.3".into(),
                summary: "s".into(),
                use_for: String::new(),
                avoid_when: String::new(),
                alternatives: vec![],
                replaces: vec![],
                notes: String::new(),
                track_docs: false,
            }],
        };
        let crates: BTreeMap<_, _> = [
            (
                "thiserror".to_owned(),
                info("thiserror", "2.0.21", "2026-09-01"),
            ),
            (
                "oldcrate".to_owned(),
                info("oldcrate", "0.3.9", "2021-01-01"),
            ),
        ]
        .into();
        let advisories = vec![Advisory {
            id: "RUSTSEC-2099-0002".into(),
            aliases: vec![],
            krate: "oldcrate".into(),
            summary: "unmaintained".into(),
            kind: "unmaintained".into(),
            severity: None,
            ranges: vec![],
            patched: vec![],
            published: None,
            withdrawn: false,
            url: String::new(),
        }];
        let findings = check(&Inputs {
            report: &report,
            catalog: &catalog,
            crates: &crates,
            advisories: &advisories,
            latest_rust: Some("1.96.0"),
            today: jiff::civil::date(2026, 9, 29),
            thresholds: Thresholds::default(),
        });
        let msgs: Vec<_> = findings.iter().map(|f| f.message.as_str()).collect();
        assert!(
            msgs.iter()
                .any(|m| m.contains("thiserror 1.0, latest is 2.0.21")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter()
                .any(|m| m.contains("RustSec flags it unmaintained")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.contains("16 releases behind")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter()
                .any(|m| m.contains("no release since 2021-01-01")),
            "{msgs:?}"
        );
        assert_eq!(findings[0].severity, Severity::High);
    }
}
