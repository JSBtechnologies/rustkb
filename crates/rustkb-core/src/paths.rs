use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Filesystem locations used by rustkb.
///
/// * `root` — the knowledge root (the plugin/repo checkout containing `skills/`).
///   Resolved from `RUSTKB_ROOT`, then `CLAUDE_PLUGIN_ROOT`, then by walking up
///   from the current directory, then from the build-time source location.
/// * `data` — writable state: downloads, normalised docs, the search index.
///   Resolved from `RUSTKB_HOME`, else the platform data directory.
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub data: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let root = find_root().ok_or(Error::RootNotFound)?;
        let data = match env_path("RUSTKB_HOME").or_else(|| env_path("CLAUDE_PLUGIN_DATA")) {
            Some(home) => home,
            None => directories::ProjectDirs::from("dev", "rustkb", "rustkb")
                .ok_or(Error::NoDataDir)?
                .data_local_dir()
                .to_owned(),
        };
        Ok(Self { root, data })
    }

    pub fn new(root: impl Into<PathBuf>, data: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            data: data.into(),
        }
    }

    pub fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    pub fn catalog_file(&self) -> PathBuf {
        self.root
            .join("skills")
            .join("rust-ecosystem")
            .join("catalog.toml")
    }

    /// Raw downloads (zip, html, json.gz), safe to delete.
    pub fn cache_dir(&self) -> PathBuf {
        self.data.join("cache")
    }

    /// Normalised JSONL docs per source; the input to indexing.
    pub fn docs_dir(&self) -> PathBuf {
        self.data.join("docs")
    }

    pub fn index_dir(&self) -> PathBuf {
        self.data.join("index")
    }

    pub fn vectors_dir(&self) -> PathBuf {
        self.data.join("vectors")
    }

    pub fn state_file(&self) -> PathBuf {
        self.data.join("state.json")
    }
}

fn is_root(dir: &Path) -> bool {
    dir.join("skills").is_dir() && dir.join("docs").join("AUTHORING.md").is_file()
}

/// An env var as a path, ignoring empty values and unexpanded `${VAR}` placeholders
/// (which is what a plugin config passes through when run outside the plugin host).
fn env_path(var: &str) -> Option<PathBuf> {
    let value = std::env::var(var).ok()?;
    let value = value.trim();
    (!value.is_empty() && !value.contains("${")).then(|| PathBuf::from(value))
}

fn find_root() -> Option<PathBuf> {
    for var in ["RUSTKB_ROOT", "CLAUDE_PLUGIN_ROOT"] {
        if let Some(dir) = env_path(var)
            && is_root(&dir)
        {
            return Some(dir);
        }
    }
    if let Ok(cwd) = std::env::current_dir()
        && let Some(dir) = cwd.ancestors().find(|d| is_root(d))
    {
        return Some(dir.to_owned());
    }
    // Development fallback: the workspace this binary was built from.
    let built_from = Path::new(env!("CARGO_MANIFEST_DIR"));
    built_from
        .ancestors()
        .find(|d| is_root(d))
        .map(Path::to_owned)
}
