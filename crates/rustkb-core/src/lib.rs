//! Core data model shared by the rustkb indexer, ingesters and server.
//!
//! Everything rustkb knows is normalised into [`Doc`]s: curated guidance sections,
//! crate catalog entries, rustdoc items, clippy lints, security advisories and
//! release notes. Each doc carries enough version metadata to detect drift.

mod catalog;
mod doc;
mod frontmatter;
mod paths;

pub use catalog::{Catalog, CatalogEntry, Tier};
pub use doc::{Doc, Source};
pub use frontmatter::{RefMeta, is_semver_breaking, rust_minor, split_frontmatter};
pub use paths::Paths;

/// Errors produced while loading rustkb's own inputs.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid frontmatter in {path}: {message}")]
    Frontmatter {
        path: std::path::PathBuf,
        message: String,
    },
    #[error("invalid catalog {path}: {source}")]
    Catalog {
        path: std::path::PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error(
        "could not locate the rustkb knowledge root (set RUSTKB_ROOT to the directory containing `skills/`)"
    )]
    RootNotFound,
    #[error("could not determine a data directory (set RUSTKB_HOME)")]
    NoDataDir,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) fn read_to_string(path: &std::path::Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}
