use std::fmt;

use serde::{Deserialize, Serialize};

/// Where a document came from. Determines ranking weight and how it is refreshed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Hand-written guidance from `skills/*/references/*.md`.
    Curated,
    /// Entry from the crate catalog (`skills/rust-ecosystem/catalog.toml`).
    Catalog,
    /// API item from a crate's rustdoc JSON (docs.rs).
    Rustdoc,
    /// Clippy lint documentation.
    Clippy,
    /// `RustSec` / OSV security advisory.
    Advisory,
    /// Rust release notes, one doc per release.
    Release,
}

impl Source {
    pub const ALL: [Self; 6] = [
        Self::Curated,
        Self::Catalog,
        Self::Rustdoc,
        Self::Clippy,
        Self::Advisory,
        Self::Release,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Curated => "curated",
            Self::Catalog => "catalog",
            Self::Rustdoc => "rustdoc",
            Self::Clippy => "clippy",
            Self::Advisory => "advisory",
            Self::Release => "release",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|src| src.as_str().eq_ignore_ascii_case(s))
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The unit of indexing and retrieval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    /// Globally unique, stable id, e.g. `curated:idioms/error-handling#err-01`,
    /// `rustdoc:tokio@1.53.1:tokio::sync::Mutex`, `clippy:unwrap_used`.
    pub id: String,
    pub source: Source,
    pub title: String,
    /// Item path (`tokio::sync::Mutex`), curated doc id, lint name, advisory id…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Parent item path, used to list children (methods of a type, items of a module).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "crate")]
    pub krate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Item kind (`struct`, `fn`), lint group, advisory severity, catalog tier…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// One-line summary shown in search results.
    #[serde(default)]
    pub summary: String,
    /// Full markdown body.
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Doc {
    pub fn new(
        id: impl Into<String>,
        source: Source,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            source,
            title: title.into(),
            path: None,
            parent: None,
            krate: None,
            version: None,
            kind: None,
            tags: Vec::new(),
            summary: String::new(),
            body: body.into(),
            url: None,
        }
    }
}
