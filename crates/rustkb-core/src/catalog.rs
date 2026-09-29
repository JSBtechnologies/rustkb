use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Doc, Error, Result, Source};

/// How strongly the catalog recommends a crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// The ecosystem default for its category.
    Default,
    /// A strong choice, often the default for a sub-niche.
    Recommended,
    /// Right for specific situations only.
    Situational,
    /// Deprecated, unmaintained or superseded — do not add to new code.
    Avoid,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Recommended => "recommended",
            Self::Situational => "situational",
            Self::Avoid => "avoid",
        }
    }
}

/// One crate in `catalog.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub name: String,
    pub category: String,
    pub tier: Tier,
    /// `major.minor` verified against crates.io.
    pub version: String,
    pub summary: String,
    #[serde(default)]
    pub use_for: String,
    #[serde(default)]
    pub avoid_when: String,
    #[serde(default)]
    pub alternatives: Vec<String>,
    #[serde(default)]
    pub replaces: Vec<String>,
    #[serde(default)]
    pub notes: String,
    /// Ingest this crate's rustdoc JSON from docs.rs.
    #[serde(default)]
    pub track_docs: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(rename = "crate", default)]
    pub crates: Vec<CatalogEntry>,
}

impl Catalog {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = crate::read_to_string(path)?;
        toml::from_str(&text).map_err(|source| Error::Catalog {
            path: path.to_owned(),
            source,
        })
    }

    pub fn get(&self, name: &str) -> Option<&CatalogEntry> {
        let name = normalize(name);
        self.crates.iter().find(|c| normalize(&c.name) == name)
    }

    /// Entries that name `name` in their `replaces` list.
    pub fn replacements_for(&self, name: &str) -> impl Iterator<Item = &CatalogEntry> {
        let name = normalize(name);
        self.crates.iter().filter(move |c| {
            c.replaces
                .iter()
                .any(|r| normalize(r.split_whitespace().next().unwrap_or(r)) == name)
        })
    }

    pub fn tracked(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.crates.iter().filter(|c| c.track_docs)
    }

    pub fn to_docs(&self) -> Vec<Doc> {
        self.crates.iter().map(CatalogEntry::to_doc).collect()
    }
}

impl CatalogEntry {
    pub fn to_doc(&self) -> Doc {
        let mut body = format!(
            "**{}** ({}, tier: {}, verified {})\n\n{}\n",
            self.name,
            self.category,
            self.tier.as_str(),
            self.version,
            self.summary
        );
        let mut section = |label: &str, text: &str| {
            if !text.is_empty() {
                let _ = write!(body, "\n**{label}:** {text}\n");
            }
        };
        section("Use for", &self.use_for);
        section("Avoid when", &self.avoid_when);
        section("Alternatives", &self.alternatives.join(", "));
        section("Replaces", &self.replaces.join(", "));
        section("Notes", &self.notes);

        let mut doc = Doc::new(
            format!("catalog:{}", self.name),
            Source::Catalog,
            self.name.clone(),
            body,
        );
        doc.path = Some(self.name.clone());
        doc.krate = Some(self.name.clone());
        doc.version = Some(self.version.clone());
        doc.kind = Some(self.tier.as_str().to_owned());
        doc.tags = vec![self.category.clone()];
        doc.tags.extend(self.replaces.iter().cloned());
        doc.summary.clone_from(&self.summary);
        doc.url = Some(format!("https://crates.io/crates/{}", self.name));
        doc
    }
}

/// crates.io treats `-` and `_` as equivalent.
fn normalize(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace('_', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[crate]]
name = "tokio"
category = "async-runtime"
tier = "default"
version = "1.53"
summary = "Async runtime."
track_docs = true

[[crate]]
name = "lazy_static"
category = "concurrency"
tier = "avoid"
version = "1.5"
summary = "Superseded by std::sync::LazyLock."
alternatives = ["std::sync::LazyLock"]
"#;

    #[test]
    fn parses_and_queries() {
        let cat: Catalog = toml::from_str(SAMPLE).expect("valid catalog");
        assert_eq!(cat.crates.len(), 2);
        assert_eq!(cat.get("Lazy-Static").map(|c| c.tier), Some(Tier::Avoid));
        assert_eq!(cat.tracked().count(), 1);
        let doc = cat.crates[0].to_doc();
        assert_eq!(doc.id, "catalog:tokio");
        assert!(doc.body.contains("async-runtime"));
    }
}
