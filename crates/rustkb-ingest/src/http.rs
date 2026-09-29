//! Polite HTTP with an on-disk cache.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Context;

use crate::Result;

const USER_AGENT: &str = concat!(
    "rustkb/",
    env!("CARGO_PKG_VERSION"),
    " (Rust knowledge base for coding agents)"
);

#[derive(Debug, Clone)]
pub struct Http {
    client: reqwest::blocking::Client,
    cache_dir: PathBuf,
    /// Ignore cached files and always re-download.
    pub force: bool,
}

impl Http {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            client,
            cache_dir: cache_dir.into(),
            force: false,
        })
    }

    /// Download `url` into `cache/<name>`, reusing a cached copy younger than `ttl`.
    pub fn fetch_cached(&self, url: &str, name: &str, ttl: Duration) -> Result<PathBuf> {
        let path = self.cache_dir.join(name);
        if !self.force && is_fresh(&path, ttl) {
            tracing::debug!(%url, "cache hit");
            return Ok(path);
        }
        tracing::info!(%url, "downloading");
        let bytes = self.get_bytes(url)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Write-then-rename so a failed download never leaves a truncated cache file.
        let tmp = path.with_extension("partial");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }

    /// Whether `cache/<name>` exists and is younger than `ttl` (and `force` is off).
    pub fn is_cached(&self, name: &str, ttl: Duration) -> bool {
        !self.force && is_fresh(&self.cache_dir.join(name), ttl)
    }

    pub fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self
            .client
            .get(url)
            .send()
            .with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("GET {url}: HTTP {status}");
        }
        Ok(resp.bytes()?.to_vec())
    }

    pub fn get_json(&self, url: &str) -> Result<serde_json::Value> {
        serde_json::from_slice(&self.get_bytes(url)?)
            .with_context(|| format!("parsing JSON from {url}"))
    }

    pub fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let resp = self
            .client
            .post(url)
            .json(body)
            .send()
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("POST {url}: HTTP {status}");
        }
        Ok(resp.json()?)
    }
}

fn is_fresh(path: &Path, ttl: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < ttl)
}

/// Decompress gzip if the bytes carry the gzip magic number, else return them as-is.
pub fn maybe_gunzip(bytes: Vec<u8>) -> Result<Vec<u8>> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes.as_slice()).read_to_end(&mut out)?;
        Ok(out)
    } else {
        Ok(bytes)
    }
}

pub const DAY: Duration = Duration::from_secs(24 * 60 * 60);
