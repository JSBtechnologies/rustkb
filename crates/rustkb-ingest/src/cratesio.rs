//! crates.io metadata (latest versions, maintenance signals), cached for a day.
//!
//! crates.io's crawler policy asks for ≤1 request/second and a descriptive User-Agent.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{Http, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrateInfo {
    pub name: String,
    pub max_stable_version: Option<String>,
    pub newest_version: Option<String>,
    pub description: Option<String>,
    pub repository: Option<String>,
    pub documentation: Option<String>,
    pub downloads: Option<u64>,
    pub recent_downloads: Option<u64>,
    /// When crates.io metadata last changed (NOT a release date).
    pub updated_at: Option<String>,
    /// Publication time of the most recent non-yanked release, any version line.
    #[serde(default)]
    pub last_release: Option<String>,
    pub created_at: Option<String>,
    /// Whether the newest version is yanked (usually a red flag).
    pub newest_yanked: bool,
    /// MSRV (`rust-version`) of the newest stable version, when declared.
    pub rust_version: Option<String>,
    pub license: Option<String>,
}

fn s(v: &serde_json::Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_owned)
}

pub fn info(http: &Http, name: &str) -> Result<CrateInfo> {
    let path = http.fetch_cached(
        &format!("https://crates.io/api/v1/crates/{name}"),
        &cache_name(name),
        crate::http::DAY,
    )?;
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let c = v
        .get("crate")
        .ok_or_else(|| anyhow::anyhow!("crate `{name}` not found on crates.io"))?;
    let max_stable = s(c, "max_stable_version");
    let versions = v.get("versions").and_then(|x| x.as_array());
    let find =
        |num: &Option<String>| versions.and_then(|vs| vs.iter().find(|ver| s(ver, "num") == *num));
    let newest = s(c, "newest_version");
    Ok(CrateInfo {
        name: s(c, "name").unwrap_or_else(|| name.to_owned()),
        description: s(c, "description").map(|d| d.trim().to_owned()),
        repository: s(c, "repository"),
        documentation: s(c, "documentation"),
        downloads: c.get("downloads").and_then(serde_json::Value::as_u64),
        recent_downloads: c
            .get("recent_downloads")
            .and_then(serde_json::Value::as_u64),
        updated_at: s(c, "updated_at"),
        last_release: versions.and_then(|vs| {
            vs.iter()
                .filter(|v| v.get("yanked").and_then(serde_json::Value::as_bool) != Some(true))
                .filter_map(|v| s(v, "created_at"))
                .max()
        }),
        created_at: s(c, "created_at"),
        newest_yanked: find(&newest)
            .and_then(|v| v.get("yanked"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        rust_version: find(&max_stable).and_then(|v| s(v, "rust_version")),
        license: find(&max_stable).and_then(|v| s(v, "license")),
        max_stable_version: max_stable,
        newest_version: newest,
    })
}

/// Fetch info for many crates at ≤1 req/s (cache hits are not throttled).
pub fn info_many(http: &Http, names: &[String]) -> Vec<(String, Result<CrateInfo>)> {
    let mut last = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    names
        .iter()
        .map(|name| {
            if !http.is_cached(&cache_name(name), crate::http::DAY) {
                let elapsed = last.elapsed();
                if elapsed < Duration::from_secs(1) {
                    std::thread::sleep(Duration::from_secs(1).saturating_sub(elapsed));
                }
                last = Instant::now();
            }
            (name.clone(), info(http, name))
        })
        .collect()
}

fn cache_name(name: &str) -> String {
    format!("cratesio/{name}.json")
}
