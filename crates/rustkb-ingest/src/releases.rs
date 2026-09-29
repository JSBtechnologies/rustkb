//! Rust release notes (`RELEASES.md`), one doc per release.

use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use rustkb_core::{Doc, Source};

use crate::{Http, Result};

pub const URL: &str = "https://raw.githubusercontent.com/rust-lang/rust/main/RELEASES.md";

static HEADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^Version (\d+\.\d+\.\d+)(?: \((\d{4}-\d{2}-\d{2})\))?\s*$").expect("valid regex")
});

pub fn fetch(http: &Http, ttl: Duration) -> Result<Vec<Doc>> {
    let path = http.fetch_cached(URL, "releases/RELEASES.md", ttl)?;
    Ok(parse(&std::fs::read_to_string(path)?))
}

pub fn parse(text: &str) -> Vec<Doc> {
    let mut docs = Vec::new();
    let mut current: Option<(String, Option<String>, String)> = None;
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(caps) = HEADING.captures(line) {
            // A heading is followed by a `====` underline.
            if lines.peek().is_some_and(|l| l.starts_with("===")) {
                lines.next();
                if let Some(done) = current.take() {
                    docs.push(to_doc(done));
                }
                current = Some((
                    caps[1].to_owned(),
                    caps.get(2).map(|m| m.as_str().to_owned()),
                    String::new(),
                ));
                continue;
            }
        }
        if let Some((_, _, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(done) = current {
        docs.push(to_doc(done));
    }
    docs
}

fn to_doc((version, date, body): (String, Option<String>, String)) -> Doc {
    // Reference-style links at the end of each section are noise for retrieval.
    let body: String = body
        .lines()
        .filter(|l| !(l.starts_with('[') && l.contains("]: http")))
        .collect::<Vec<_>>()
        .join("\n");
    let summary = format!(
        "Rust {version}{} release notes",
        date.as_deref()
            .map(|d| format!(" ({d})"))
            .unwrap_or_default()
    );
    let minor = version
        .rsplit_once('.')
        .map_or(version.as_str(), |(mm, _)| mm)
        .to_owned();
    let mut d = Doc::new(
        format!("release:{version}"),
        Source::Release,
        format!("Rust {version}"),
        body.trim(),
    );
    d.path = Some(version.clone());
    d.version = Some(version.clone());
    d.kind = date;
    d.tags = vec![minor, "release".into()];
    d.summary = summary;
    d.url = Some(format!("https://blog.rust-lang.org/releases/{version}"));
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_releases() {
        let text = "Version 1.96.0 (2026-05-28)\n==========================\n\nLanguage\n--------\n- Stabilized a thing\n\n[1]: https://x\n\nVersion 1.95.0 (2026-04-16)\n===========\n- other\n";
        let docs = parse(text);
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].id, "release:1.96.0");
        assert_eq!(docs[0].kind.as_deref(), Some("2026-05-28"));
        assert!(docs[0].body.contains("Stabilized"));
        assert!(!docs[0].body.contains("https://x"));
        assert_eq!(docs[0].tags[0], "1.96");
    }
}
