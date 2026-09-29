use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Frontmatter of a curated reference file. See `docs/AUTHORING.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefMeta {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub area: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Stable Rust version the content was verified against.
    pub rust: String,
    #[serde(default)]
    pub edition: Option<String>,
    /// Recommended crates → `major.minor` verified against.
    #[serde(default)]
    pub crates: BTreeMap<String, String>,
    /// Date (YYYY-MM-DD) the content was last verified.
    pub verified: String,
    #[serde(default)]
    pub sources: Vec<String>,
}

/// Split a markdown document into its YAML frontmatter and body.
///
/// Returns `None` for the frontmatter when the file does not start with `---`.
pub fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text
        .strip_prefix("---")
        .and_then(|r| r.strip_prefix(['\n', '\r']))
    else {
        return (None, text);
    };
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let body = &rest[offset + line.len()..];
            return (Some(&rest[..offset]), body);
        }
        offset += line.len();
    }
    (None, text)
}

impl RefMeta {
    /// Parse the frontmatter of a reference file, returning metadata and body.
    pub fn parse(path: &Path, text: &str) -> Result<(Self, String)> {
        let (fm, body) = split_frontmatter(text);
        let fm = fm.ok_or_else(|| Error::Frontmatter {
            path: path.to_owned(),
            message: "missing `---` frontmatter block".into(),
        })?;
        let meta: Self = serde_saphyr::from_str(fm).map_err(|e| Error::Frontmatter {
            path: path.to_owned(),
            message: e.to_string(),
        })?;
        Ok((meta, body.to_owned()))
    }

    /// Validate semantic constraints serde cannot express.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if !matches!(
            self.area.as_str(),
            "idioms" | "architecture" | "ecosystem" | "security"
        ) {
            problems.push(format!(
                "area `{}` is not one of idioms|architecture|ecosystem|security",
                self.area
            ));
        }
        if !self.id.contains('/') {
            problems.push(format!("id `{}` should look like `<area>/<slug>`", self.id));
        }
        if jiff::civil::Date::strptime("%Y-%m-%d", &self.verified).is_err() {
            problems.push(format!(
                "verified `{}` is not a YYYY-MM-DD date",
                self.verified
            ));
        }
        if parse_minor_version(&self.rust).is_none() {
            problems.push(format!("rust `{}` is not a `1.NN` version", self.rust));
        }
        for (name, version) in &self.crates {
            if parse_req(version).is_none() {
                problems.push(format!(
                    "crate `{name}` version `{version}` is not `major.minor`"
                ));
            }
        }
        problems
    }
}

/// Parse `1.96` / `1.96.0` into the minor number.
pub(crate) fn parse_minor_version(s: &str) -> Option<u64> {
    let mut parts = s.trim().split('.');
    (parts.next()? == "1").then_some(())?;
    parts.next()?.parse().ok()
}

/// Parse a `major.minor[.patch]` string into a semver version (missing parts = 0).
pub(crate) fn parse_req(s: &str) -> Option<semver::Version> {
    let mut nums = s.trim().split('.').map(str::parse::<u64>);
    let major = nums.next()?.ok()?;
    let minor = nums.next().transpose().ok()?.unwrap_or(0);
    let patch = nums.next().transpose().ok()?.unwrap_or(0);
    Some(semver::Version::new(major, minor, patch))
}

/// True when `latest` is semver-incompatible with `verified` (Cargo's caret rules).
pub fn is_semver_breaking(verified: &str, latest: &str) -> Option<bool> {
    let v = parse_req(verified)?;
    let l = semver::Version::parse(latest)
        .ok()
        .or_else(|| parse_req(latest))?;
    let breaking = match (v.major, v.minor) {
        (0, 0) => l.major != 0 || l.minor != 0 || l.patch != v.patch,
        (0, minor) => l.major != 0 || l.minor != minor,
        (major, _) => l.major != major,
    };
    Some(breaking && l > v)
}

/// Rust minor version from a `1.NN` string, public for staleness checks.
pub fn rust_minor(s: &str) -> Option<u64> {
    parse_minor_version(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_frontmatter() {
        let (fm, body) = split_frontmatter("---\nid: a/b\n---\n# Title\n");
        assert_eq!(fm, Some("id: a/b\n"));
        assert_eq!(body, "# Title\n");
    }

    #[test]
    fn splits_crlf_frontmatter() {
        let (fm, body) = split_frontmatter("---\r\nid: a/b\r\n---\r\nbody");
        assert_eq!(fm, Some("id: a/b\r\n"));
        assert_eq!(body, "body");
    }

    #[test]
    fn no_frontmatter() {
        assert_eq!(split_frontmatter("# hi"), (None, "# hi"));
    }

    #[test]
    fn semver_breaking() {
        assert_eq!(is_semver_breaking("1.40", "1.53.1"), Some(false));
        assert_eq!(is_semver_breaking("1.40", "2.0.0"), Some(true));
        assert_eq!(is_semver_breaking("0.7", "0.8.1"), Some(true));
        assert_eq!(is_semver_breaking("0.7", "0.7.9"), Some(false));
        assert_eq!(is_semver_breaking("2.0", "1.9.0"), Some(false));
    }

    #[test]
    fn rust_minor_parses() {
        assert_eq!(rust_minor("1.96"), Some(96));
        assert_eq!(rust_minor("1.96.1"), Some(96));
        assert_eq!(rust_minor("2.0"), None);
    }
}
