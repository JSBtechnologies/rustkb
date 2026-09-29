//! Normalised docs on disk: `docs/<source>.jsonl` and `docs/rustdoc/<crate>@<version>.jsonl`.

use std::io::{BufRead, BufWriter, Write};
use std::path::{Path, PathBuf};

use rustkb_core::{Doc, Paths};

use crate::Result;

#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(paths: &Paths) -> Self {
        Self {
            dir: paths.docs_dir(),
        }
    }

    pub fn source_file(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.jsonl"))
    }

    pub fn rustdoc_file(&self, krate: &str, version: &str) -> PathBuf {
        self.dir
            .join("rustdoc")
            .join(format!("{krate}@{version}.jsonl"))
    }

    pub fn write(&self, path: &Path, docs: &[Doc]) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("jsonl.partial");
        {
            let mut w = BufWriter::new(std::fs::File::create(&tmp)?);
            for doc in docs {
                serde_json::to_writer(&mut w, doc)?;
                w.write_all(b"\n")?;
            }
            w.flush()?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn read(path: &Path) -> Result<Vec<Doc>> {
        let file = std::fs::File::open(path)?;
        std::io::BufReader::new(file)
            .lines()
            .filter(|l| l.as_ref().map_or(true, |l| !l.trim().is_empty()))
            .map(|line| Ok(serde_json::from_str(&line?)?))
            .collect()
    }

    /// Every stored doc (all remote sources + ingested rustdoc).
    pub fn read_all(&self) -> Result<Vec<Doc>> {
        let mut docs = Vec::new();
        if !self.dir.exists() {
            return Ok(docs);
        }
        for entry in walkdir::WalkDir::new(&self.dir)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.path().extension().is_some_and(|e| e == "jsonl") {
                docs.extend(Self::read(entry.path())?);
            }
        }
        Ok(docs)
    }

    /// `(crate, version)` pairs with stored rustdoc.
    pub fn rustdoc_versions(&self) -> Vec<(String, String)> {
        let Ok(entries) = std::fs::read_dir(self.dir.join("rustdoc")) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e
                    .file_name()
                    .to_string_lossy()
                    .strip_suffix(".jsonl")?
                    .to_owned();
                let (k, v) = name.split_once('@')?;
                Some((k.to_owned(), v.to_owned()))
            })
            .collect()
    }
}
