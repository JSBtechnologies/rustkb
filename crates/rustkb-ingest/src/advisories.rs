//! Security advisories for crates.io from OSV (which mirrors the `RustSec` advisory DB).
//!
//! Bulk: the OSV `crates.io/all.zip` export, normalised into docs + structured records for
//! offline version matching. Live: the OSV query API for a specific `crate@version`.

use std::io::Read;
use std::time::Duration;

use anyhow::Context;
use rustkb_core::{Doc, Source};
use serde::{Deserialize, Serialize};

use crate::{Http, Result};

pub const BULK_URL: &str = "https://osv-vulnerabilities.storage.googleapis.com/crates.io/all.zip";
pub const QUERY_URL: &str = "https://api.osv.dev/v1/query";

/// A structured advisory, kept for offline `check`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Advisory {
    pub id: String,
    pub aliases: Vec<String>,
    pub krate: String,
    pub summary: String,
    /// `vulnerability`, `unmaintained`, `unsound`, `notice`…
    pub kind: String,
    pub severity: Option<String>,
    /// Semver ranges as `(introduced, fixed_or_last_affected)` event pairs.
    pub ranges: Vec<(Option<String>, Option<String>)>,
    pub patched: Vec<String>,
    pub published: Option<String>,
    pub withdrawn: bool,
    pub url: String,
}

pub fn fetch(http: &Http, ttl: Duration) -> Result<(Vec<Doc>, Vec<Advisory>)> {
    let path = http.fetch_cached(BULK_URL, "osv/crates.io-all.zip", ttl)?;
    let file = std::fs::File::open(&path)?;
    let mut zip = zip::ZipArchive::new(file).context("opening OSV zip")?;
    let mut docs = Vec::new();
    let mut advisories = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if !std::path::Path::new(entry.name())
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            continue;
        }
        let mut text = String::new();
        entry.read_to_string(&mut text)?;
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(file = entry.name(), error = %e, "skipping malformed OSV record");
                continue;
            }
        };
        for adv in from_osv(&value) {
            docs.push(adv.to_doc(&value));
            advisories.push(adv);
        }
    }
    anyhow::ensure!(!docs.is_empty(), "OSV export contained no advisories");
    // OSV mirrors most RustSec advisories as GHSA records too; keep one per issue and crate.
    let mut aliased = std::collections::HashSet::new();
    for a in advisories.iter().filter(|a| a.id.starts_with("RUSTSEC")) {
        for alias in &a.aliases {
            aliased.insert((alias.clone(), a.krate.clone()));
        }
    }
    let mut seen = std::collections::HashSet::new();
    let keep: Vec<bool> = advisories
        .iter()
        .map(|a| {
            !aliased.contains(&(a.id.clone(), a.krate.clone()))
                && seen.insert((a.id.clone(), a.krate.clone()))
        })
        .collect();
    let mut flags = keep.iter();
    docs.retain(|_| *flags.next().unwrap_or(&false));
    let mut flags = keep.iter();
    advisories.retain(|_| *flags.next().unwrap_or(&false));
    Ok((docs, advisories))
}

fn s(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// One OSV record can affect several crates; emit one advisory per crate.
pub fn from_osv(v: &serde_json::Value) -> Vec<Advisory> {
    let id = s(v, "id").unwrap_or_default();
    let aliases: Vec<String> = v
        .get("aliases")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let summary = s(v, "summary").unwrap_or_else(|| {
        s(v, "details")
            .map(|d| d.lines().next().unwrap_or_default().to_owned())
            .unwrap_or_default()
    });
    let db = v.get("database_specific");
    let kind = db
        .and_then(|d| d.get("informational"))
        .and_then(|i| i.as_str())
        .map_or_else(|| "vulnerability".to_owned(), str::to_owned);
    let severity = v
        .get("severity")
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
        .and_then(|s| s.get("score"))
        .and_then(|s| s.as_str())
        .map(str::to_owned)
        .or_else(|| {
            db.and_then(|d| d.get("cvss"))
                .and_then(|c| c.as_str())
                .map(str::to_owned)
        });
    let url = if id.starts_with("RUSTSEC") {
        format!("https://rustsec.org/advisories/{id}.html")
    } else {
        format!("https://osv.dev/vulnerability/{id}")
    };
    let withdrawn = v.get("withdrawn").is_some();

    let mut out = Vec::new();
    for affected in v
        .get("affected")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
    {
        let Some(pkg) = affected.get("package") else {
            continue;
        };
        if pkg.get("ecosystem").and_then(|e| e.as_str()) != Some("crates.io") {
            continue;
        }
        let krate = s(pkg, "name").unwrap_or_default();
        let mut ranges = Vec::new();
        let mut patched = Vec::new();
        for range in affected
            .get("ranges")
            .and_then(|r| r.as_array())
            .into_iter()
            .flatten()
        {
            let mut introduced = None;
            for ev in range
                .get("events")
                .and_then(|e| e.as_array())
                .into_iter()
                .flatten()
            {
                if let Some(i) = s(ev, "introduced") {
                    introduced = Some(i);
                } else if let Some(f) = s(ev, "fixed") {
                    patched.push(f.clone());
                    ranges.push((introduced.take(), Some(format!("<{f}"))));
                } else if let Some(l) = s(ev, "last_affected") {
                    ranges.push((introduced.take(), Some(format!("<={l}"))));
                }
            }
            if introduced.is_some() {
                ranges.push((introduced, None));
            }
        }
        // A record may list the same crate several times (one per range set): merge.
        if let Some(existing) = out.iter_mut().find(|a: &&mut Advisory| a.krate == krate) {
            existing.ranges.extend(ranges);
            existing.patched.extend(patched);
            continue;
        }
        out.push(Advisory {
            id: id.clone(),
            aliases: aliases.clone(),
            krate,
            summary: summary.clone(),
            // RustSec puts `informational`/`cvss` per affected package; fall back to top level.
            kind: affected
                .get("database_specific")
                .and_then(|d| d.get("informational"))
                .and_then(serde_json::Value::as_str)
                .map_or_else(|| kind.clone(), str::to_owned),
            severity: severity.clone().or_else(|| {
                affected
                    .get("database_specific")
                    .and_then(|d| d.get("cvss"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            }),
            ranges,
            patched,
            published: s(v, "published"),
            withdrawn,
            url: url.clone(),
        });
    }
    out
}

impl Advisory {
    /// Whether `version` falls in any affected range.
    pub fn affects(&self, version: &semver::Version) -> bool {
        if self.withdrawn {
            return false;
        }
        self.ranges.iter().any(|(introduced, end)| {
            let lower_ok = introduced
                .as_deref()
                .and_then(|i| semver::Version::parse(i).ok())
                .is_none_or(|i| version >= &i);
            let upper_ok = match end.as_deref() {
                None => true,
                Some(e) => {
                    if let Some(v) = e.strip_prefix("<=") {
                        semver::Version::parse(v).is_ok_and(|v| version <= &v)
                    } else if let Some(v) = e.strip_prefix('<') {
                        semver::Version::parse(v).is_ok_and(|v| version < &v)
                    } else {
                        true
                    }
                }
            };
            lower_ok && upper_ok
        })
    }

    fn to_doc(&self, raw: &serde_json::Value) -> Doc {
        let details = s(raw, "details").unwrap_or_default();
        let ranges: Vec<String> = self
            .ranges
            .iter()
            .map(|(i, e)| {
                format!(
                    ">={} {}",
                    i.as_deref().unwrap_or("0"),
                    e.as_deref().unwrap_or("(no fix)")
                )
            })
            .collect();
        let body = format!(
            "**{}** affecting crate `{}` ({}{})\n\nAffected: {}\n\nPatched: {}\n\nAliases: {}\n\n{}",
            self.id,
            self.krate,
            self.kind,
            self.severity
                .as_deref()
                .map(|s| format!(", severity {s}"))
                .unwrap_or_default(),
            if ranges.is_empty() {
                "all versions".into()
            } else {
                ranges.join("; ")
            },
            if self.patched.is_empty() {
                "none".into()
            } else {
                self.patched.join(", ")
            },
            if self.aliases.is_empty() {
                "none".into()
            } else {
                self.aliases.join(", ")
            },
            details
        );
        let mut d = Doc::new(
            format!("advisory:{}:{}", self.id, self.krate),
            Source::Advisory,
            format!("{}: {} ({})", self.id, self.summary, self.krate),
            body,
        );
        d.path = Some(self.id.clone());
        d.krate = Some(self.krate.clone());
        d.kind = Some(self.kind.clone());
        d.version.clone_from(&self.published);
        d.tags.clone_from(&self.aliases);
        d.tags.push(self.kind.clone());
        if self.withdrawn {
            d.tags.push("withdrawn".into());
        }
        d.summary.clone_from(&self.summary);
        d.url = Some(self.url.clone());
        d
    }
}

/// Offline check against stored advisories.
pub fn check_offline<'a>(
    advisories: &'a [Advisory],
    krate: &str,
    version: &str,
) -> Result<Vec<&'a Advisory>> {
    let v = semver::Version::parse(version)
        .with_context(|| format!("`{version}` is not a full semver version"))?;
    let name = krate.replace('_', "-").to_ascii_lowercase();
    let hits: Vec<&Advisory> = advisories
        .iter()
        .filter(|a| a.krate.replace('_', "-").to_ascii_lowercase() == name && a.affects(&v))
        .collect();
    Ok(dedupe(hits))
}

/// Drop duplicate ids and records that are aliases of another hit (GHSA mirrors of a
/// RUSTSEC advisory), preferring RUSTSEC ids.
pub fn dedupe<A: std::borrow::Borrow<Advisory>>(mut advs: Vec<A>) -> Vec<A> {
    advs.sort_by_key(|a| !a.borrow().id.starts_with("RUSTSEC"));
    let mut seen = std::collections::HashSet::new();
    advs.retain(|a| {
        let a = a.borrow();
        let fresh = seen.insert(a.id.clone());
        if fresh {
            seen.extend(a.aliases.iter().cloned());
        }
        fresh
    });
    advs
}

/// Live check via the OSV API (always current).
pub fn check_live(http: &Http, krate: &str, version: &str) -> Result<Vec<Advisory>> {
    let body = serde_json::json!({ "package": { "name": krate, "ecosystem": "crates.io" }, "version": version });
    let resp = http.post_json(QUERY_URL, &body)?;
    Ok(dedupe(
        resp.get("vulns")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .flat_map(from_osv)
            .filter(|a| a.krate.eq_ignore_ascii_case(krate) && !a.withdrawn)
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_osv_and_matches_versions() {
        let raw = serde_json::json!({
            "id": "RUSTSEC-2099-0001",
            "summary": "Use after free",
            "aliases": ["CVE-2099-1"],
            "affected": [{
                "package": {"name": "demo", "ecosystem": "crates.io"},
                "ranges": [{"type": "SEMVER", "events": [{"introduced": "0.0.0-0"}, {"fixed": "1.2.3"}, {"introduced": "2.0.0"}, {"fixed": "2.0.1"}]}],
                "database_specific": {"informational": "unsound", "cvss": null}
            }],
            "database_specific": {"license": "CC0-1.0"}
        });
        let advs = from_osv(&raw);
        assert_eq!(advs.len(), 1);
        let a = &advs[0];
        assert_eq!(a.kind, "unsound");
        assert!(a.affects(&"1.2.2".parse().expect("valid")));
        assert!(!a.affects(&"1.2.3".parse().expect("valid")));
        assert!(a.affects(&"2.0.0".parse().expect("valid")));
        assert!(!a.affects(&"2.0.1".parse().expect("valid")));
        assert_eq!(a.patched, ["1.2.3", "2.0.1"]);
    }
}
