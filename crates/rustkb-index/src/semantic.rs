//! Dense retrieval with a static (model2vec) embedding model.
//!
//! Static embeddings are a lookup + mean-pool: no ONNX runtime, no GPU, thousands of
//! docs per second on a laptop. Vectors are kept in a flat, L2-normalised matrix and
//! searched by brute force, which is fast enough for the ~10⁴ docs we embed
//! (rustdoc items are excluded: they are found by identifier, not meaning).

use std::io::{Read, Write};
use std::path::Path;

use model2vec_rs::model::StaticModel;
use rustkb_core::{Doc, Source};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Default model; override with `RUSTKB_EMBED_MODEL` (any model2vec HF repo or local dir).
pub(crate) const DEFAULT_MODEL: &str = "minishlab/potion-base-8M";

pub struct Embeddings {
    model: StaticModel,
    meta: Meta,
    matrix: Vec<f32>,
}

impl std::fmt::Debug for Embeddings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Embeddings")
            .field("model", &self.meta.model)
            .field("count", &self.meta.ids.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Meta {
    model: String,
    dim: usize,
    ids: Vec<String>,
}

fn model_id() -> String {
    std::env::var("RUSTKB_EMBED_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned())
}

fn load_model(id: &str) -> Result<StaticModel> {
    StaticModel::from_pretrained(id, None, Some(true), None)
        .map_err(|e| Error::Embedding(format!("loading {id}: {e:#}")))
}

/// Whether a doc is worth embedding: conceptual content. API items and advisories are found
/// by identifier/crate name, and advisories' sheer number would swamp dense results.
pub fn embeddable(doc: &Doc) -> bool {
    !matches!(doc.source, Source::Rustdoc | Source::Advisory)
}

fn text_for(doc: &Doc) -> String {
    let body: String = doc.body.chars().take(2000).collect();
    format!(
        "{}\n{}\n{}\n{}",
        doc.title,
        doc.tags.join(" "),
        doc.summary,
        body
    )
}

impl Embeddings {
    /// Embed `docs` and persist vectors to `dir`.
    pub fn build<'a>(dir: &Path, docs: impl IntoIterator<Item = &'a Doc>) -> Result<Self> {
        let id = model_id();
        let model = load_model(&id)?;
        let docs: Vec<&Doc> = docs.into_iter().filter(|d| embeddable(d)).collect();
        let texts: Vec<String> = docs.iter().map(|d| text_for(d)).collect();
        let vectors = model.encode_with_args(&texts, Some(256), 512);
        let dim = vectors.first().map_or(0, Vec::len);
        let mut matrix = Vec::with_capacity(dim * vectors.len());
        for mut v in vectors {
            normalize(&mut v);
            matrix.extend_from_slice(&v);
        }
        let meta = Meta {
            model: id,
            dim,
            ids: docs.iter().map(|d| d.id.clone()).collect(),
        };

        std::fs::create_dir_all(dir)?;
        // Matrix first, meta last (both via rename) so readers never see a mismatched pair.
        {
            let mut f = std::io::BufWriter::new(std::fs::File::create(dir.join("matrix.f32.tmp"))?);
            for x in &matrix {
                f.write_all(&x.to_le_bytes())?;
            }
            f.flush()?;
        }
        std::fs::rename(dir.join("matrix.f32.tmp"), dir.join("matrix.f32"))?;
        std::fs::write(dir.join("meta.json.tmp"), serde_json::to_vec(&meta)?)?;
        std::fs::rename(dir.join("meta.json.tmp"), dir.join("meta.json"))?;
        Ok(Self {
            model,
            meta,
            matrix,
        })
    }

    /// Load persisted vectors; `None` if none were built.
    pub fn load(dir: &Path) -> Result<Option<Self>> {
        let meta_path = dir.join("meta.json");
        if !meta_path.is_file() {
            return Ok(None);
        }
        let meta: Meta = serde_json::from_slice(&std::fs::read(meta_path)?)?;
        let mut bytes = Vec::new();
        std::fs::File::open(dir.join("matrix.f32"))?.read_to_end(&mut bytes)?;
        let matrix: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        if matrix.len() != meta.dim * meta.ids.len() {
            return Err(Error::Embedding(
                "vector matrix size mismatch; rebuild with `rustkb index`".into(),
            ));
        }
        let model = load_model(&meta.model)?;
        Ok(Some(Self {
            model,
            meta,
            matrix,
        }))
    }

    /// Top-`k` doc ids by cosine similarity.
    pub fn search(&self, query: &str, k: usize) -> Result<Vec<(String, f32)>> {
        let dim = self.meta.dim;
        if dim == 0 {
            return Ok(Vec::new());
        }
        let mut q = self.model.encode_single(query);
        normalize(&mut q);
        let mut scored: Vec<(usize, f32)> = self
            .matrix
            .chunks_exact(dim)
            .enumerate()
            .map(|(i, row)| (i, row.iter().zip(&q).map(|(a, b)| a * b).sum()))
            .collect();
        let k = k.min(scored.len());
        if k == 0 {
            return Ok(Vec::new());
        }
        scored.select_nth_unstable_by(k - 1, |a, b| b.1.total_cmp(&a.1));
        scored.truncate(k);
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        Ok(scored
            .into_iter()
            .map(|(i, s)| (self.meta.ids[i].clone(), s))
            .collect())
    }

    pub fn len(&self) -> usize {
        self.meta.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.meta.ids.is_empty()
    }
}

fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}
