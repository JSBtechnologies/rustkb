//! Ingestion: turn upstream sources and the curated corpus into [`Doc`]s.
//!
//! Every remote source is downloaded into `cache/` and normalised into
//! `docs/<source>.jsonl`, so the index can be rebuilt offline and a failed
//! refresh never destroys the last good data.

pub mod advisories;
pub mod clippy;
pub mod corpus;
pub mod cratesio;
pub mod curated;
pub mod http;
pub mod releases;
pub mod rustdoc;
pub mod stale;
pub mod store;

pub use http::Http;
pub use store::Store;

pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;
