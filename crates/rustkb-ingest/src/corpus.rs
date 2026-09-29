//! The curated corpus for installs without a local checkout (e.g. `cargo install rustkb`).
//!
//! Downloads the upstream repository archive and keeps only `skills/` and
//! `docs/AUTHORING.md`, so knowledge updates reach users without a new binary release.
//! Only ever writes to rustkb's own data directory — never to a user's checkout.

use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use anyhow::Context;

use crate::{Http, Result};

/// Override with `RUSTKB_CORPUS_URL` (any zip whose single top-level directory holds `skills/`).
pub const DEFAULT_URL: &str =
    "https://codeload.github.com/JSBtechnologies/rustkb/zip/refs/heads/main";

pub fn url() -> String {
    std::env::var("RUSTKB_CORPUS_URL").unwrap_or_else(|_| DEFAULT_URL.to_owned())
}

/// Download the corpus and atomically replace `dest` with it. Returns the number of files.
pub fn fetch(http: &Http, dest: &Path) -> Result<usize> {
    let url = url();
    tracing::info!(%url, "downloading curated corpus");
    let bytes = http.get_bytes(&url)?;
    let parent = dest.parent().context("corpus directory has no parent")?;
    std::fs::create_dir_all(parent)?;
    let staging = parent.join("corpus.partial");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    let count = extract(&bytes, &staging)?;
    anyhow::ensure!(
        staging.join("skills").is_dir() && staging.join("docs/AUTHORING.md").is_file(),
        "downloaded archive from {url} does not contain a rustkb corpus"
    );
    if dest.exists() {
        std::fs::remove_dir_all(dest)
            .with_context(|| format!("removing old corpus at {}", dest.display()))?;
    }
    std::fs::rename(&staging, dest)?;
    Ok(count)
}

/// Extract `skills/**` and `docs/AUTHORING.md` from a GitHub-style archive (one top-level dir).
fn extract(bytes: &[u8], dest: &Path) -> Result<usize> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("opening corpus archive")?;
    let mut count = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        // `enclosed_name` rejects absolute paths and `..` (zip-slip).
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let rel: PathBuf = name.components().skip(1).collect();
        let wanted = rel.starts_with("skills") || rel == Path::new("docs/AUTHORING.md");
        if !wanted || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            continue;
        }
        let out = dest.join(&rel);
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        std::fs::write(&out, buf)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn extracts_only_corpus_files() {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            for (name, body) in [
                ("rustkb-main/skills/a/SKILL.md", "skill"),
                ("rustkb-main/docs/AUTHORING.md", "spec"),
                ("rustkb-main/crates/x/src/lib.rs", "code"),
                ("rustkb-main/../evil.md", "nope"),
            ] {
                w.start_file(name, opts).expect("start");
                w.write_all(body.as_bytes()).expect("write");
            }
            w.finish().expect("finish");
        }
        let dir = std::env::temp_dir().join(format!("rustkb-corpus-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let n = extract(&buf, &dir).expect("extracts");
        assert_eq!(n, 2);
        assert!(dir.join("skills/a/SKILL.md").is_file());
        assert!(dir.join("docs/AUTHORING.md").is_file());
        assert!(!dir.join("crates").exists());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
