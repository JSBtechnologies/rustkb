//! The curated corpus: `skills/*/SKILL.md`, `skills/*/references/*.md` and the crate catalog.
//!
//! Reference files are split on `## ` headings: each section is a retrieval unit.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use rustkb_core::{Catalog, Doc, Paths, RefMeta, Source, split_frontmatter};

use crate::Result;

static RULE_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^#{3,4}\s+([A-Z][A-Z0-9]{1,9}-\d{2,3})\b").expect("valid regex")
});

/// A parsed reference file.
#[derive(Debug, Clone)]
pub struct RefFile {
    pub path: PathBuf,
    /// Path relative to the knowledge root, with `/` separators.
    pub rel: String,
    pub skill: String,
    pub meta: RefMeta,
    pub body: String,
}

/// Problems found while loading, keyed by file.
#[derive(Debug, Default)]
pub struct Report {
    pub files: Vec<RefFile>,
    pub errors: Vec<(PathBuf, String)>,
}

fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Load and validate every reference file.
pub fn load(paths: &Paths) -> Report {
    let mut report = Report::default();
    let mut seen_ids = std::collections::HashMap::<String, PathBuf>::new();
    let pattern = paths.skills_dir();
    let files = walkdir::WalkDir::new(&pattern)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| {
            e.path().extension().is_some_and(|x| x == "md")
                && e.path()
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|p| p == "references")
        });
    for entry in files {
        let path = entry.path().to_owned();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                report.errors.push((path, e.to_string()));
                continue;
            }
        };
        match RefMeta::parse(&path, &text) {
            Ok((meta, body)) => {
                for problem in meta.validate() {
                    report.errors.push((path.clone(), problem));
                }
                if let Some(first) = seen_ids.insert(meta.id.clone(), path.clone()) {
                    report.errors.push((
                        path.clone(),
                        format!("duplicate id `{}` (also in {})", meta.id, first.display()),
                    ));
                }
                let skill = path
                    .ancestors()
                    .nth(2)
                    .and_then(Path::file_name)
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let rel = rel_path(&paths.root, &path);
                report.files.push(RefFile {
                    path,
                    rel,
                    skill,
                    meta,
                    body,
                });
            }
            Err(e) => report.errors.push((path, e.to_string())),
        }
    }
    report
}

/// Split a markdown body into `(heading, section_text)` on `## ` headings, ignoring
/// headings inside fenced code blocks. Text before the first `##` becomes an "Overview".
pub fn sections(body: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut heading = String::from("Overview");
    let mut buf = String::new();
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        }
        if !in_fence && let Some(h) = line.strip_prefix("## ") {
            if !buf.trim().is_empty() {
                out.push((std::mem::take(&mut heading), std::mem::take(&mut buf)));
            }
            h.trim().clone_into(&mut heading);
            buf.clear();
            continue;
        }
        if !in_fence && line.starts_with("# ") && out.is_empty() && buf.trim().is_empty() {
            continue; // document title; already in frontmatter
        }
        buf.push_str(line);
        buf.push('\n');
    }
    if !buf.trim().is_empty() {
        out.push((heading, buf));
    }
    out
}

pub fn slug(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    s.trim_matches('-').to_owned()
}

impl RefFile {
    pub fn to_docs(&self) -> Vec<Doc> {
        let m = &self.meta;
        let mut used = std::collections::HashSet::new();
        sections(&self.body)
            .into_iter()
            .map(|(heading, text)| {
                let mut anchor = slug(&heading);
                while !used.insert(anchor.clone()) {
                    anchor.push('_');
                }
                let mut doc = Doc::new(
                    format!("curated:{}#{anchor}", m.id),
                    Source::Curated,
                    format!("{} — {}", m.title, heading),
                    text.trim().to_owned(),
                );
                doc.path = Some(m.id.clone());
                doc.kind = Some(m.area.clone());
                doc.tags.clone_from(&m.tags);
                doc.tags
                    .extend(RULE_ID.captures_iter(&text).map(|c| c[1].to_owned()));
                doc.tags.extend(m.crates.keys().cloned());
                m.summary.trim().clone_into(&mut doc.summary);
                doc.version = Some(m.rust.clone());
                doc.url = Some(format!("{}#{anchor}", self.rel));
                doc
            })
            .collect()
    }
}

/// `SKILL.md` files become one doc each: their core-rules list is high-signal.
pub fn skill_docs(paths: &Paths) -> Result<Vec<Doc>> {
    let mut docs = Vec::new();
    let Ok(entries) = std::fs::read_dir(paths.skills_dir()) else {
        return Ok(docs);
    };
    for entry in entries.filter_map(Result::ok) {
        let file = entry.path().join("SKILL.md");
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let (fm, body) = split_frontmatter(&text);
        let description = fm
            .and_then(|fm| fm.lines().find_map(|l| l.strip_prefix("description:")))
            .map(|d| d.trim().trim_matches(['"', '\'']).to_owned())
            .unwrap_or_default();
        let mut doc = Doc::new(
            format!("curated:skill/{name}"),
            Source::Curated,
            format!("Skill: {name}"),
            body.trim(),
        );
        doc.path = Some(format!("skill/{name}"));
        doc.kind = Some("skill".into());
        doc.summary = description;
        doc.tags = RULE_ID
            .captures_iter(body)
            .map(|c| c[1].to_owned())
            .collect();
        doc.url = Some(rel_path(&paths.root, &file));
        docs.push(doc);
    }
    Ok(docs)
}

/// All curated docs: skills, reference sections and catalog entries.
pub fn all_docs(paths: &Paths) -> Result<(Vec<Doc>, Report)> {
    let report = load(paths);
    let mut docs: Vec<Doc> = report.files.iter().flat_map(RefFile::to_docs).collect();
    docs.extend(skill_docs(paths)?);
    docs.extend(Catalog::load(&paths.catalog_file())?.to_docs());
    Ok((docs, report))
}

/// Find a reference file by its frontmatter id (or a unique suffix/slug of it).
pub fn find_file<'a>(report: &'a Report, id: &str) -> Option<&'a RefFile> {
    let id = id.trim_start_matches("curated:");
    let id = id.split('#').next().unwrap_or(id);
    report.files.iter().find(|f| f.meta.id == id).or_else(|| {
        let mut matches = report
            .files
            .iter()
            .filter(|f| f.meta.id.ends_with(id) || f.rel.ends_with(id));
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_sections_outside_fences() {
        let body = "# Title\nintro\n## A\ntext\n```rust\n## not a heading\n```\n## B\nmore\n";
        let s = sections(body);
        let names: Vec<_> = s.iter().map(|(h, _)| h.as_str()).collect();
        assert_eq!(names, ["Overview", "A", "B"]);
        assert!(s[1].1.contains("## not a heading"));
    }

    #[test]
    fn slugs() {
        assert_eq!(
            slug("ERR-01: Libraries expose typed errors"),
            "err-01-libraries-expose-typed-errors"
        );
        assert_eq!(
            slug("`Arc<Mutex<T>>` vs channels"),
            "arc-mutex-t-vs-channels"
        );
    }

    #[test]
    fn extracts_rule_ids() {
        let ids: Vec<_> = RULE_ID
            .captures_iter("### ERR-01: x\n#### ASYNC-12 y\n### not-a-rule")
            .map(|c| c[1].to_owned())
            .collect();
        assert_eq!(ids, ["ERR-01", "ASYNC-12"]);
    }
}
