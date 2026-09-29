//! Hybrid search over rustkb [`Doc`]s.
//!
//! Lexical retrieval uses tantivy with a code-aware tokenizer (keeps `spawn_blocking`,
//! `ERR-01` and `Arc` intact) plus exact-path fields for structured lookups. With the
//! `semantic` feature, a static embedding model (model2vec) adds dense retrieval and the
//! two rankings are fused with reciprocal rank fusion.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rustkb_core::{Doc, Source};
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::snippet::SnippetGenerator;
use tantivy::tokenizer::{LowerCaser, RegexTokenizer, TextAnalyzer};
use tantivy::{IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

#[cfg(feature = "semantic")]
mod semantic;
#[cfg(feature = "semantic")]
pub use semantic::{Embeddings, embeddable};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Tantivy(#[from] tantivy::TantivyError),
    #[error(transparent)]
    OpenDirectory(#[from] tantivy::directory::error::OpenDirectoryError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt stored document: {0}")]
    Json(#[from] serde_json::Error),
    #[error("embedding error: {0}")]
    Embedding(String),
    #[error("index not built yet at {0} — run `rustkb index`")]
    Missing(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

const CODE_TOKENIZER: &str = "rustkb_code";
const WRITER_HEAP: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
struct Fields {
    id: Field,
    source: Field,
    krate: Field,
    version: Field,
    kind: Field,
    path_exact: Field,
    parent_exact: Field,
    title: Field,
    path: Field,
    tags: Field,
    summary: Field,
    body: Field,
    json: Field,
}

fn schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let code = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(CODE_TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let prose = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("en_stem")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let fields = Fields {
        id: b.add_text_field("id", STRING | STORED),
        source: b.add_text_field("source", STRING),
        krate: b.add_text_field("crate", STRING),
        version: b.add_text_field("version", STRING),
        kind: b.add_text_field("kind", STRING),
        path_exact: b.add_text_field("path_exact", STRING),
        parent_exact: b.add_text_field("parent_exact", STRING),
        title: b.add_text_field("title", code.clone()),
        path: b.add_text_field("path", code.clone()),
        tags: b.add_text_field("tags", code),
        summary: b.add_text_field("summary", prose.clone()),
        // Body is stored separately in `json`; index it for search + snippets.
        body: b.add_text_field("body", prose.set_stored()),
        json: b.add_bytes_field("json", STORED),
    };
    (b.build(), fields)
}

fn register_tokenizers(index: &tantivy::Index) -> Result<()> {
    // Identifiers, paths segments and rule ids (`ERR-01`) stay whole.
    let tokenizer = RegexTokenizer::new(r"[A-Za-z0-9_]+(?:-[0-9]+)?")?;
    index.tokenizers().register(
        CODE_TOKENIZER,
        TextAnalyzer::builder(tokenizer).filter(LowerCaser).build(),
    );
    Ok(())
}

/// Search request.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub text: String,
    /// Restrict to these sources (empty = all).
    pub sources: Vec<Source>,
    pub krate: Option<String>,
    pub version: Option<String>,
    pub limit: usize,
}

/// A ranked search result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Hit {
    pub score: f32,
    pub snippet: String,
    #[serde(flatten)]
    pub doc: Doc,
}

/// Document counts per source.
pub type Stats = Vec<(Source, u64)>;

pub struct Index {
    index: tantivy::Index,
    reader: IndexReader,
    fields: Fields,
    generation: String,
    /// Serialises writes within this process; tantivy's lockfile serialises across processes.
    write_lock: Mutex<()>,
    #[cfg(feature = "semantic")]
    embeddings: Option<Embeddings>,
}

impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

/// The index directory holds generations (`gen-<n>/`) and a `CURRENT` file naming the
/// live one. Rebuilds never delete a directory another process may have memory-mapped
/// (which fails on Windows); they switch `CURRENT` atomically and garbage-collect old
/// generations best-effort.
fn current_generation(root: &Path) -> Option<String> {
    let name = std::fs::read_to_string(root.join("CURRENT"))
        .ok()?
        .trim()
        .to_owned();
    root.join(&name).join("meta.json").is_file().then_some(name)
}

impl Index {
    pub fn exists(root: &Path) -> bool {
        current_generation(root).is_some()
    }

    pub fn open(root: &Path) -> Result<Self> {
        let generation =
            current_generation(root).ok_or_else(|| Error::Missing(root.display().to_string()))?;
        let index = tantivy::Index::open_in_dir(root.join(&generation))?;
        Self::from_tantivy(index, generation)
    }

    /// Whether this handle still points at the live generation under `root`.
    pub fn is_current(&self, root: &Path) -> bool {
        current_generation(root).as_deref() == Some(self.generation.as_str())
    }

    /// Build a fresh generation from `docs` and make it the live one.
    pub fn rebuild(root: &Path, docs: impl IntoIterator<Item = Doc>) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let generation = format!("gen-{nanos}-{}", std::process::id());
        let dir = root.join(&generation);
        std::fs::create_dir_all(&dir)?;
        let (schema, _) = schema();
        let index = tantivy::Index::create_in_dir(&dir, schema)?;
        let this = Self::from_tantivy(index, generation.clone())?;
        {
            let mut writer = this.index.writer::<TantivyDocument>(WRITER_HEAP)?;
            // Ids are unique by contract; drop accidental duplicates (last one wins).
            let mut unique: HashMap<String, Doc> = HashMap::new();
            for doc in docs {
                unique.insert(doc.id.clone(), doc);
            }
            for doc in unique.into_values() {
                writer.add_document(this.to_tantivy(&doc)?)?;
            }
            writer.commit()?;
            writer.wait_merging_threads()?;
        }
        this.reader.reload()?;

        let tmp = root.join("CURRENT.tmp");
        std::fs::write(&tmp, &generation)?;
        std::fs::rename(&tmp, root.join("CURRENT"))?;
        for entry in std::fs::read_dir(root)?.filter_map(std::result::Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != generation && entry.path().is_dir() {
                // Fails harmlessly while another process still has it open.
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        Ok(this)
    }

    fn from_tantivy(index: tantivy::Index, generation: String) -> Result<Self> {
        register_tokenizers(&index)?;
        let (_, fields) = schema();
        // Picks up commits made by other processes (e.g. another MCP server fetching rustdoc).
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        Ok(Self {
            index,
            reader,
            fields,
            generation,
            write_lock: Mutex::new(()),
            #[cfg(feature = "semantic")]
            embeddings: None,
        })
    }

    #[cfg(feature = "semantic")]
    #[must_use]
    pub fn with_embeddings(mut self, embeddings: Embeddings) -> Self {
        self.embeddings = Some(embeddings);
        self
    }

    fn to_tantivy(&self, doc: &Doc) -> Result<TantivyDocument> {
        let f = &self.fields;
        let mut t = TantivyDocument::new();
        t.add_text(f.id, &doc.id);
        t.add_text(f.source, doc.source.as_str());
        if let Some(k) = &doc.krate {
            t.add_text(f.krate, normalize_crate(k));
        }
        if let Some(v) = &doc.version {
            t.add_text(f.version, v);
        }
        if let Some(k) = &doc.kind {
            t.add_text(f.kind, k);
        }
        if let Some(p) = &doc.path {
            t.add_text(f.path_exact, p.to_ascii_lowercase());
            t.add_text(f.path, p);
        }
        if let Some(p) = &doc.parent {
            t.add_text(f.parent_exact, p.to_ascii_lowercase());
        }
        t.add_text(f.title, &doc.title);
        for tag in &doc.tags {
            t.add_text(f.tags, tag);
        }
        t.add_text(f.summary, &doc.summary);
        t.add_text(f.body, &doc.body);
        t.add_bytes(f.json, &serde_json::to_vec(doc)?);
        Ok(t)
    }

    fn doc_from_stored(&self, t: &TantivyDocument) -> Result<Doc> {
        let bytes = t
            .get_first(self.fields.json)
            .and_then(|v| v.as_bytes())
            .unwrap_or_default();
        Ok(serde_json::from_slice(bytes)?)
    }

    /// Run `f` with a short-lived writer, commit, and release the lock. Waits up to ~10 s
    /// for another process holding the index lock.
    fn write<T>(
        &self,
        f: impl FnOnce(&mut IndexWriter<TantivyDocument>) -> Result<T>,
    ) -> Result<T> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut attempts = 0;
        let mut writer = loop {
            match self.index.writer::<TantivyDocument>(WRITER_HEAP) {
                Ok(w) => break w,
                Err(tantivy::TantivyError::LockFailure(..)) if attempts < 100 => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => return Err(e.into()),
            }
        };
        let out = f(&mut writer)?;
        writer.commit()?;
        writer.wait_merging_threads()?;
        self.reader.reload()?;
        Ok(out)
    }

    /// Insert or replace docs (by id) and make them visible to searches.
    pub fn upsert(&self, docs: &[Doc]) -> Result<()> {
        self.write(|writer| {
            for doc in docs {
                writer.delete_term(Term::from_field_text(self.fields.id, &doc.id));
                writer.add_document(self.to_tantivy(doc)?)?;
            }
            Ok(())
        })
    }

    /// Remove every doc matching `source` (and `krate`/`version`, if given).
    pub fn delete_where(
        &self,
        source: Source,
        krate: Option<&str>,
        version: Option<&str>,
    ) -> Result<()> {
        self.write(|writer| {
            writer.delete_query(self.filter_query(&[source], krate, version, None))?;
            Ok(())
        })
    }

    /// Atomically replace every doc from `sources` with `docs` (one commit).
    pub fn replace_sources(&self, sources: &[Source], docs: &[Doc]) -> Result<()> {
        self.write(|writer| {
            for s in sources {
                writer.delete_query(self.filter_query(&[*s], None, None, None))?;
            }
            for doc in docs {
                writer.add_document(self.to_tantivy(doc)?)?;
            }
            Ok(())
        })
    }

    fn term(field: Field, value: &str) -> Box<dyn Query> {
        Box::new(TermQuery::new(
            Term::from_field_text(field, value),
            IndexRecordOption::Basic,
        ))
    }

    fn filter_query(
        &self,
        sources: &[Source],
        krate: Option<&str>,
        version: Option<&str>,
        main: Option<Box<dyn Query>>,
    ) -> Box<dyn Query> {
        let f = &self.fields;
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        if let Some(main) = main {
            clauses.push((Occur::Must, main));
        }
        if !sources.is_empty() {
            let any: Vec<(Occur, Box<dyn Query>)> = sources
                .iter()
                .map(|s| (Occur::Should, Self::term(f.source, s.as_str())))
                .collect();
            clauses.push((Occur::Must, Box::new(BooleanQuery::new(any))));
        }
        if let Some(k) = krate {
            clauses.push((Occur::Must, Self::term(f.krate, &normalize_crate(k))));
        }
        if let Some(v) = version {
            clauses.push((Occur::Must, Self::term(f.version, v)));
        }
        if clauses.iter().all(|(o, _)| *o != Occur::Must) {
            clauses.push((Occur::Must, Box::new(tantivy::query::AllQuery)));
        }
        Box::new(BooleanQuery::new(clauses))
    }

    fn collect(&self, query: &dyn Query, limit: usize) -> Result<Vec<(f32, Doc, TantivyDocument)>> {
        let searcher = self.reader.searcher();
        let top = searcher.search(query, &TopDocs::with_limit(limit.max(1)).order_by_score())?;
        top.into_iter()
            .map(|(score, addr)| {
                let t: TantivyDocument = searcher.doc(addr)?;
                Ok((score, self.doc_from_stored(&t)?, t))
            })
            .collect()
    }

    /// Ranked full-text (and, with `semantic`, hybrid) search.
    pub fn search(&self, q: &SearchQuery) -> Result<Vec<Hit>> {
        let f = &self.fields;
        let limit = if q.limit == 0 { 10 } else { q.limit };
        let mut parser = QueryParser::for_index(
            &self.index,
            vec![f.title, f.path, f.tags, f.summary, f.body],
        );
        parser.set_field_boost(f.title, 4.0);
        parser.set_field_boost(f.path, 3.0);
        parser.set_field_boost(f.tags, 2.5);
        parser.set_field_boost(f.summary, 1.5);
        let (text_query, _errors) = parser.parse_query_lenient(&sanitize_query(&q.text));
        let query = self.filter_query(
            &q.sources,
            q.krate.as_deref(),
            q.version.as_deref(),
            Some(text_query),
        );

        // Over-fetch so source weighting and fusion can reorder.
        let candidates = self.collect(query.as_ref(), limit * 4)?;
        let searcher = self.reader.searcher();
        let snippets = SnippetGenerator::create(&searcher, query.as_ref(), f.body).ok();

        let mut hits: Vec<Hit> = candidates
            .into_iter()
            .map(|(score, doc, t)| {
                let snippet = snippets
                    .as_ref()
                    .map(|g| g.snippet_from_doc(&t).fragment().to_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| doc.summary.clone());
                Hit {
                    score: score * source_weight(doc.source),
                    snippet,
                    doc,
                }
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));

        // Identifier-style queries (paths, snake_case, CamelCase, rule ids) are answered best
        // by exact lexical matches; dense retrieval would dilute them.
        #[cfg(feature = "semantic")]
        if let Some(emb) = &self.embeddings
            && !looks_like_code(&q.text)
        {
            hits = self.fuse_semantic(emb, q, hits)?;
        }

        hits.truncate(limit);
        Ok(hits)
    }

    #[cfg(feature = "semantic")]
    fn fuse_semantic(
        &self,
        emb: &Embeddings,
        q: &SearchQuery,
        lexical: Vec<Hit>,
    ) -> Result<Vec<Hit>> {
        const K: f32 = 60.0;
        let dense = emb.search(&q.text, q.limit.max(10) * 4)?;
        let mut fused: HashMap<String, (f32, Option<Hit>)> = HashMap::new();
        for (rank, hit) in lexical.into_iter().enumerate() {
            let entry = fused.entry(hit.doc.id.clone()).or_default();
            entry.0 += 1.0 / (K + rank as f32 + 1.0);
            entry.1 = Some(hit);
        }
        for (rank, (id, _sim)) in dense.into_iter().enumerate() {
            let entry = fused.entry(id).or_default();
            entry.0 += 1.0 / (K + rank as f32 + 1.0);
        }
        let mut out = Vec::with_capacity(fused.len());
        for (id, (score, hit)) in fused {
            let hit = match hit {
                Some(h) => h,
                None => match self.get(&id)? {
                    Some(doc) if matches_filters(&doc, q) => Hit {
                        score: 0.0,
                        snippet: doc.summary.clone(),
                        doc,
                    },
                    _ => continue,
                },
            };
            // Same source priors as lexical ranking, so fused lists keep guidance on top.
            let score = score * source_weight(hit.doc.source);
            out.push(Hit { score, ..hit });
        }
        out.sort_by(|a, b| b.score.total_cmp(&a.score));
        Ok(out)
    }

    /// Fetch one doc by id.
    pub fn get(&self, id: &str) -> Result<Option<Doc>> {
        let q = Self::term(self.fields.id, id);
        Ok(self
            .collect(q.as_ref(), 1)?
            .into_iter()
            .next()
            .map(|(_, d, _)| d))
    }

    /// All docs whose exact item path matches (case-insensitive), newest version first.
    pub fn find_path(
        &self,
        source: Source,
        krate: Option<&str>,
        path: &str,
        version: Option<&str>,
    ) -> Result<Vec<Doc>> {
        let main = Self::term(self.fields.path_exact, &path.to_ascii_lowercase());
        let q = self.filter_query(&[source], krate, version, Some(main));
        let mut docs: Vec<Doc> = self
            .collect(q.as_ref(), 50)?
            .into_iter()
            .map(|(_, d, _)| d)
            .collect();
        sort_newest_first(&mut docs);
        Ok(docs)
    }

    /// Children of an item (methods of a type, items of a module).
    pub fn children(
        &self,
        source: Source,
        krate: Option<&str>,
        parent: &str,
        version: Option<&str>,
    ) -> Result<Vec<Doc>> {
        let main = Self::term(self.fields.parent_exact, &parent.to_ascii_lowercase());
        let q = self.filter_query(&[source], krate, version, Some(main));
        let mut docs: Vec<Doc> = self
            .collect(q.as_ref(), 500)?
            .into_iter()
            .map(|(_, d, _)| d)
            .collect();
        docs.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.title.cmp(&b.title)));
        Ok(docs)
    }

    /// All docs from `source`, optionally for one crate.
    pub fn list(&self, source: Source, krate: Option<&str>, limit: usize) -> Result<Vec<Doc>> {
        let q = self.filter_query(&[source], krate, None, None);
        Ok(self
            .collect(q.as_ref(), limit)?
            .into_iter()
            .map(|(_, d, _)| d)
            .collect())
    }

    /// Versions of `krate` with indexed rustdoc, newest first.
    pub fn rustdoc_versions(&self, krate: &str) -> Result<Vec<String>> {
        let main = Self::term(self.fields.kind, "crate");
        let q = self.filter_query(&[Source::Rustdoc], Some(krate), None, Some(main));
        let mut docs: Vec<Doc> = self
            .collect(q.as_ref(), 50)?
            .into_iter()
            .map(|(_, d, _)| d)
            .collect();
        sort_newest_first(&mut docs);
        Ok(docs.into_iter().filter_map(|d| d.version).collect())
    }

    pub fn stats(&self) -> Result<Stats> {
        let searcher = self.reader.searcher();
        Source::ALL
            .into_iter()
            .map(|s| {
                let q = Self::term(self.fields.source, s.as_str());
                Ok((
                    s,
                    searcher.search(q.as_ref(), &tantivy::collector::Count)? as u64,
                ))
            })
            .collect()
    }
}

#[cfg(feature = "semantic")]
fn matches_filters(doc: &Doc, q: &SearchQuery) -> bool {
    (q.sources.is_empty() || q.sources.contains(&doc.source))
        && q.krate
            .as_deref()
            .is_none_or(|k| doc.krate.as_deref().map(normalize_crate) == Some(normalize_crate(k)))
        && q.version
            .as_deref()
            .is_none_or(|v| doc.version.as_deref() == Some(v))
}

/// Heuristic: does this query name code rather than describe a concept?
#[cfg_attr(not(feature = "semantic"), allow(dead_code))]
fn looks_like_code(text: &str) -> bool {
    let t = text.trim();
    if ["::", "#[", "!(", "()", "<", "&"]
        .iter()
        .any(|m| t.contains(m))
    {
        return true;
    }
    let words: Vec<&str> = t.split_whitespace().collect();
    let codeish = |w: &&str| {
        w.contains('_')
            || w.chars().skip(1).any(char::is_uppercase)
            || (w.contains('-')
                && w.chars().any(|c| c.is_ascii_digit())
                && w.chars().next().is_some_and(char::is_uppercase))
    };
    words.len() <= 2 && (words.len() == 1 || words.iter().any(codeish))
}

/// Guidance should outrank raw API text for conceptual questions.
fn source_weight(source: Source) -> f32 {
    match source {
        Source::Curated => 1.6,
        Source::Catalog => 1.3,
        Source::Clippy | Source::Rustdoc => 1.0,
        Source::Advisory | Source::Release => 0.8,
    }
}

fn normalize_crate(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace('-', "_")
}

/// Turn free text / Rust paths into something the lenient parser handles well:
/// `tokio::sync::Mutex` → `tokio sync Mutex`, strip syntax characters.
fn sanitize_query(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' || c == '"' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .filter(|w| !matches!(*w, "AND" | "OR" | "NOT" | "-"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn sort_newest_first(docs: &mut [Doc]) {
    let key = |d: &Doc| {
        d.version
            .as_deref()
            .and_then(|v| semver::Version::parse(v).ok())
    };
    docs.sort_by_key(|d| std::cmp::Reverse(key(d)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: &str, source: Source, title: &str, body: &str) -> Doc {
        let mut d = Doc::new(id, source, title, body);
        d.summary = title.to_owned();
        d
    }

    #[test]
    fn code_query_detection() {
        for q in [
            "tokio::sync::Mutex",
            "spawn_blocking",
            "HashMap",
            "ERR-03",
            "#[non_exhaustive]",
            "Arc<Mutex<T>>",
        ] {
            assert!(looks_like_code(q), "{q}");
        }
        for q in [
            "how should I structure a plugin system",
            "sharing state between threads",
            "error handling",
        ] {
            assert!(!looks_like_code(q), "{q}");
        }
    }

    #[test]
    fn search_and_lookup() -> Result<()> {
        let dir = tempdir();
        let mut item = doc(
            "rustdoc:tokio@1.53.1:tokio::task::spawn_blocking",
            Source::Rustdoc,
            "spawn_blocking",
            "Runs the provided closure on a thread where blocking is acceptable.",
        );
        item.krate = Some("tokio".into());
        item.version = Some("1.53.1".into());
        item.path = Some("tokio::task::spawn_blocking".into());
        item.parent = Some("tokio::task".into());
        let guide = doc(
            "curated:idioms/async#blocking",
            Source::Curated,
            "Never block the async runtime",
            "Use spawn_blocking for blocking or CPU-heavy work inside async code. ASYNC-03.",
        );
        let index = Index::rebuild(&dir, [item, guide])?;

        let hits = index.search(&SearchQuery {
            text: "tokio::task::spawn_blocking".into(),
            limit: 5,
            ..Default::default()
        })?;
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].doc.source,
            Source::Rustdoc,
            "exact identifier match wins"
        );

        let hits = index.search(&SearchQuery {
            text: "blocking in async code".into(),
            limit: 5,
            ..Default::default()
        })?;
        assert_eq!(
            hits[0].doc.source,
            Source::Curated,
            "curated guidance wins conceptual queries"
        );

        let only_api = index.search(&SearchQuery {
            text: "blocking".into(),
            sources: vec![Source::Rustdoc],
            krate: Some("tokio".into()),
            limit: 5,
            ..Default::default()
        })?;
        assert_eq!(only_api.len(), 1);

        let found = index.find_path(
            Source::Rustdoc,
            Some("tokio"),
            "Tokio::Task::spawn_blocking",
            None,
        )?;
        assert_eq!(found.len(), 1);
        assert_eq!(
            index
                .children(Source::Rustdoc, Some("tokio"), "tokio::task", None)?
                .len(),
            1
        );

        let rule = index.search(&SearchQuery {
            text: "ASYNC-03".into(),
            limit: 5,
            ..Default::default()
        })?;
        assert_eq!(rule[0].doc.id, "curated:idioms/async#blocking");

        index.delete_where(Source::Rustdoc, Some("tokio"), None)?;
        assert!(
            index
                .get("rustdoc:tokio@1.53.1:tokio::task::spawn_blocking")?
                .is_none()
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rustkb-index-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }
}
