//! The knowledge-base service shared by the CLI and the MCP server.
//!
//! All methods are synchronous (tantivy + blocking HTTP); the MCP layer calls them on
//! tokio's blocking pool. Tool output is Markdown because that is what agents read best.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rustkb_core::{Catalog, CatalogEntry, Doc, Paths, Source, Tier};
use rustkb_index::{Index, SearchQuery};
use rustkb_ingest::advisories::{self, Advisory};
use rustkb_ingest::curated::{self, Report};
use rustkb_ingest::{Http, Store, clippy, cratesio, releases, rustdoc, stale};
use serde::{Deserialize, Serialize};

const ADVISORY_FILE: &str = "advisories.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    curated_fingerprint: u64,
    #[serde(default)]
    last_ingest: BTreeMap<String, String>,
    #[serde(default)]
    semantic: bool,
}

pub(crate) struct Kb {
    pub paths: Paths,
    index: RwLock<Arc<Index>>,
    pub http: Http,
    pub store: Store,
    pub catalog: Catalog,
    pub curated: Report,
}

impl std::fmt::Debug for Kb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kb")
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct IngestOptions {
    /// Sources to refresh; empty = all remote sources.
    pub only: Vec<String>,
    /// Extra `crate[@version]` specs to ingest rustdoc for.
    pub crates: Vec<String>,
    pub force: bool,
}

impl Kb {
    /// Open the knowledge base, (re)building the index when it is missing or the curated
    /// corpus changed since it was built. Never touches the network.
    pub(crate) fn open(paths: Paths) -> Result<Self> {
        let http = Http::new(paths.cache_dir())?;
        let store = Store::new(&paths);
        let catalog = Catalog::load(&paths.catalog_file())?;
        let curated = curated::load(&paths);
        for (path, err) in &curated.errors {
            tracing::warn!(path = %path.display(), %err, "curated doc problem");
        }

        let mut state = read_state(&paths);
        let fingerprint = fingerprint(&paths);
        let index = if Index::exists(&paths.index_dir()) {
            let index = Index::open(&paths.index_dir())?;
            if state.curated_fingerprint != fingerprint {
                tracing::info!("curated corpus changed; refreshing curated docs in index");
                let refreshed = curated::all_docs(&paths).and_then(|(docs, _)| {
                    index
                        .replace_sources(&[Source::Curated, Source::Catalog], &docs)
                        .map_err(anyhow::Error::from)
                });
                match refreshed {
                    Ok(()) => {
                        state.curated_fingerprint = fingerprint;
                        state.semantic = false; // vectors are stale
                        write_state(&paths, &state)?;
                    }
                    // Another process may hold the index; serve slightly stale curated docs.
                    Err(e) => tracing::warn!(error = %format!("{e:#}"), "curated refresh skipped"),
                }
            }
            index
        } else {
            tracing::info!("no index yet; building from curated corpus and stored docs");
            let docs = all_docs(&paths, &store)?;
            let index = Index::rebuild(&paths.index_dir(), docs)?;
            state.curated_fingerprint = fingerprint;
            state.semantic = false;
            write_state(&paths, &state)?;
            index
        };

        #[cfg(feature = "semantic")]
        let index = attach_embeddings(&paths, &store, index, &mut state)?;

        Ok(Self {
            paths,
            index: RwLock::new(Arc::new(index)),
            http,
            store,
            catalog,
            curated,
        })
    }

    /// Rebuild the whole index (and vectors) from curated files + stored docs.
    pub(crate) fn rebuild(paths: &Paths) -> Result<BTreeMap<String, u64>> {
        let store = Store::new(paths);
        let docs = all_docs(paths, &store)?;
        let mut counts = BTreeMap::new();
        for d in &docs {
            *counts.entry(d.source.as_str().to_owned()).or_insert(0) += 1;
        }
        #[cfg(feature = "semantic")]
        {
            rustkb_index::Embeddings::build(&paths.vectors_dir(), &docs)
                .map_err(|e| anyhow::anyhow!("building embeddings: {e}"))?;
        }
        Index::rebuild(&paths.index_dir(), docs)?;
        let mut state = read_state(paths);
        state.curated_fingerprint = fingerprint(paths);
        state.semantic = cfg!(feature = "semantic");
        write_state(paths, &state)?;
        Ok(counts)
    }

    /// Refresh remote sources into the store, then rebuild the index.
    pub(crate) fn ingest(paths: &Paths, opts: &IngestOptions) -> Result<Vec<String>> {
        let mut http = Http::new(paths.cache_dir())?;
        http.force = opts.force;
        let store = Store::new(paths);
        let catalog = Catalog::load(&paths.catalog_file())?;
        let want = |s: &str| opts.only.is_empty() || opts.only.iter().any(|o| o == s);
        let ttl = if opts.force {
            Duration::ZERO
        } else {
            rustkb_ingest::http::DAY
        };
        let mut log = Vec::new();
        let mut state = read_state(paths);
        let now = jiff::Timestamp::now().to_string();

        let mut run = |name: &str, f: &mut dyn FnMut() -> Result<String>| match f() {
            Ok(msg) => {
                state.last_ingest.insert(name.to_owned(), now.clone());
                log.push(format!("{name}: {msg}"));
            }
            Err(e) => log.push(format!("{name}: FAILED — {e:#}")),
        };

        if want("clippy") {
            run("clippy", &mut || {
                let docs = clippy::fetch(&http, ttl)?;
                store.write(&store.source_file("clippy"), &docs)?;
                Ok(format!("{} lints", docs.len()))
            });
        }
        if want("advisories") {
            run("advisories", &mut || {
                let (docs, advs) = advisories::fetch(&http, ttl)?;
                store.write(&store.source_file("advisory"), &docs)?;
                std::fs::write(paths.data.join(ADVISORY_FILE), serde_json::to_vec(&advs)?)?;
                Ok(format!("{} advisories", docs.len()))
            });
        }
        if want("releases") {
            run("releases", &mut || {
                let docs = releases::fetch(&http, ttl)?;
                store.write(&store.source_file("release"), &docs)?;
                Ok(format!("{} releases", docs.len()))
            });
        }
        if want("rustdoc") {
            let mut specs: Vec<(String, String)> = catalog
                .tracked()
                .map(|c| (c.name.clone(), "latest".to_owned()))
                .collect();
            specs.extend(opts.crates.iter().map(|s| match s.split_once('@') {
                Some((k, v)) => (k.to_owned(), v.to_owned()),
                None => (s.clone(), "latest".to_owned()),
            }));
            let (mut ok, mut failed) = (0, Vec::new());
            for (krate, version) in &specs {
                let cached = http.is_cached(&format!("rustdoc/{krate}-{version}.json.gz"), ttl);
                match ingest_rustdoc(&http, &store, krate, version) {
                    Ok(_) => ok += 1,
                    Err(e) => {
                        tracing::warn!(%krate, error = %format!("{e:#}"), "rustdoc ingest failed");
                        failed.push(krate.clone());
                    }
                }
                if !cached {
                    std::thread::sleep(Duration::from_millis(500)); // be polite to docs.rs
                }
            }
            state.last_ingest.insert("rustdoc".into(), now.clone());
            log.push(format!(
                "rustdoc: {ok} crates{}",
                if failed.is_empty() {
                    String::new()
                } else {
                    format!(", failed: {}", failed.join(", "))
                }
            ));
        }
        write_state(paths, &state)?;
        let counts = Self::rebuild(paths)?;
        log.push(format!("index rebuilt: {counts:?}"));
        Ok(log)
    }

    /// The live index, transparently switching to a newer generation after a rebuild
    /// (e.g. `rustkb ingest` run while this server is up).
    pub(crate) fn index(&self) -> Arc<Index> {
        let current = Arc::clone(&self.index.read().unwrap_or_else(PoisonError::into_inner));
        if current.is_current(&self.paths.index_dir()) {
            return current;
        }
        match Index::open(&self.paths.index_dir()) {
            Ok(fresh) => {
                #[cfg(feature = "semantic")]
                let fresh = match rustkb_index::Embeddings::load(&self.paths.vectors_dir()) {
                    Ok(Some(e)) => fresh.with_embeddings(e),
                    _ => fresh,
                };
                tracing::info!("switched to rebuilt index generation");
                let fresh = Arc::new(fresh);
                *self.index.write().unwrap_or_else(PoisonError::into_inner) = Arc::clone(&fresh);
                fresh
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not open new index generation; keeping current");
                current
            }
        }
    }

    // ---- queries -------------------------------------------------------------------

    pub(crate) fn search(
        &self,
        query: &str,
        sources: &[String],
        krate: Option<&str>,
        limit: usize,
    ) -> Result<String> {
        let sources = sources
            .iter()
            .map(|s| Source::parse(s).with_context(|| format!("unknown source `{s}` (use one of curated, catalog, rustdoc, clippy, advisory, release)")))
            .collect::<Result<Vec<_>>>()?;
        let hits = self.index().search(&SearchQuery {
            text: query.to_owned(),
            sources,
            krate: krate.map(str::to_owned),
            version: None,
            limit: limit.clamp(1, 50),
        })?;
        if hits.is_empty() {
            return Ok(format!(
                "No results for `{query}`. Try fewer or different terms, or drop filters."
            ));
        }
        let mut out = format!("{} results for `{query}`:\n", hits.len());
        for (i, h) in hits.iter().enumerate() {
            let d = &h.doc;
            let meta = [d.krate.as_deref(), d.version.as_deref(), d.kind.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            let _ = write!(
                out,
                "
{}. **{}** [{}{}]
   id: `{}`
",
                i + 1,
                d.title,
                d.source,
                if meta.is_empty() {
                    String::new()
                } else {
                    format!(" · {meta}")
                },
                d.id
            );
            let snippet = h.snippet.split_whitespace().collect::<Vec<_>>().join(" ");
            if !snippet.is_empty() {
                let _ = writeln!(out, "   {}", truncate(&snippet, 280));
            }
        }
        out.push_str("\nUse `get_doc` with an id for the full text.");
        Ok(out)
    }

    /// Full text of a doc id; curated file ids (`idioms/error-handling`) return the whole file.
    pub(crate) fn get_doc(&self, id: &str) -> Result<String> {
        if let Some(doc) = self.index().get(id)? {
            if doc.source == Source::Curated
                && !id.contains('#')
                && let Some(file) = curated::find_file(&self.curated, id)
            {
                return Ok(render_file(file));
            }
            return Ok(render_doc(&doc));
        }
        if let Some(file) = curated::find_file(&self.curated, id) {
            return Ok(render_file(file));
        }
        // Friendly fallbacks for bare names.
        for candidate in [
            format!("clippy:{}", id.trim_start_matches("clippy::")),
            format!("catalog:{id}"),
            format!("release:{id}"),
        ] {
            if let Some(doc) = self.index().get(&candidate)? {
                return Ok(render_doc(&doc));
            }
        }
        bail!("no document with id `{id}` — use `search` to find ids")
    }

    /// Versioned API docs for an item, fetching the crate's rustdoc from docs.rs on demand.
    pub(crate) fn get_item(
        &self,
        krate: &str,
        path: Option<&str>,
        version: Option<&str>,
    ) -> Result<String> {
        let mut versions = self.index().rustdoc_versions(krate)?;
        let need_fetch = match version {
            Some(v) => !versions.iter().any(|x| x == v),
            None => versions.is_empty(),
        };
        if need_fetch {
            let docs = ingest_rustdoc(&self.http, &self.store, krate, version.unwrap_or("latest"))?;
            self.index().upsert(&docs)?;
            versions = self.index().rustdoc_versions(krate)?;
        }
        let version = version
            .map(str::to_owned)
            .or_else(|| versions.first().cloned())
            .context("no rustdoc versions")?;
        let root = self
            .index()
            .list(Source::Rustdoc, Some(krate), 200)?
            .into_iter()
            .find(|d| d.kind.as_deref() == Some("crate") && d.version.as_deref() == Some(&version))
            .and_then(|d| d.path)
            .unwrap_or_else(|| krate.replace('-', "_"));

        let path = path
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .unwrap_or(&root);
        let candidates = [path.to_owned(), format!("{root}::{path}")];
        let found = candidates.iter().find_map(|p| {
            self.index()
                .find_path(Source::Rustdoc, Some(krate), p, Some(&version))
                .ok()
                .and_then(|v| v.into_iter().next())
        });

        let Some(doc) = found else {
            let hits = self.index().search(&SearchQuery {
                text: path.to_owned(),
                sources: vec![Source::Rustdoc],
                krate: Some(krate.to_owned()),
                version: Some(version.clone()),
                limit: 8,
            })?;
            let mut out = format!("`{path}` not found in {krate} {version}. Closest items:\n");
            for h in hits {
                let _ = writeln!(
                    out,
                    "- `{}` ({})",
                    h.doc.path.unwrap_or_default(),
                    h.doc.kind.unwrap_or_default()
                );
            }
            return Ok(out);
        };

        let mut out = render_doc(&doc);
        if let Some(p) = &doc.path {
            let children =
                self.index()
                    .children(Source::Rustdoc, Some(krate), p, Some(&version))?;
            if !children.is_empty() {
                out.push_str("\n\n## Members\n");
                for c in children.iter().take(150) {
                    let _ = writeln!(
                        out,
                        "- `{}` ({}){}",
                        c.title,
                        c.kind.as_deref().unwrap_or("?"),
                        if c.summary.is_empty() {
                            String::new()
                        } else {
                            format!(" — {}", truncate(&c.summary, 140))
                        }
                    );
                }
                if children.len() > 150 {
                    let _ = writeln!(out, "- … {} more", children.len() - 150);
                }
            }
        }
        if versions.len() > 1 {
            let _ = write!(
                out,
                "\n\nIndexed versions of {krate}: {}",
                versions.join(", ")
            );
        }
        Ok(out)
    }

    pub(crate) fn crate_info(&self, name: &str) -> String {
        let mut out = String::new();
        let entry = self.catalog.get(name);
        match cratesio::info(&self.http, name) {
            Ok(info) => {
                let _ = writeln!(out, "# {}\n", info.name);
                if let Some(d) = &info.description {
                    let _ = writeln!(out, "{d}\n");
                }
                let _ = writeln!(
                    out,
                    "- latest stable: **{}**",
                    info.max_stable_version.as_deref().unwrap_or("none")
                );
                if info.newest_version != info.max_stable_version {
                    let _ = writeln!(
                        out,
                        "- newest (pre-release): {}",
                        info.newest_version.as_deref().unwrap_or("?")
                    );
                }
                if info.newest_yanked {
                    let _ = writeln!(out, "- ⚠️ newest release is yanked");
                }
                if let Some(msrv) = &info.rust_version {
                    let _ = writeln!(out, "- MSRV (rust-version): {msrv}");
                }
                if let Some(l) = &info.license {
                    let _ = writeln!(out, "- license: {l}");
                }
                if let Some(u) = &info.last_release {
                    let _ = writeln!(out, "- last release: {}", u.get(..10).unwrap_or(u));
                }
                if let (Some(all), Some(recent)) = (info.downloads, info.recent_downloads) {
                    let _ = writeln!(
                        out,
                        "- downloads: {all} total, {recent} in the last 90 days"
                    );
                }
                if let Some(r) = &info.repository {
                    let _ = writeln!(out, "- repository: {r}");
                }
                if entry.is_some_and(|e| e.tier == Tier::Avoid) {
                    let _ = writeln!(
                        out,
                        "- ⚠️ **do not add** — superseded; see alternatives below"
                    );
                } else {
                    let _ = writeln!(out, "- add: `cargo add {}`", info.name);
                }
            }
            Err(e) => {
                let _ = writeln!(out, "# {name}\n\n(crates.io lookup failed: {e:#})");
            }
        }
        match entry {
            Some(c) => {
                let _ = write!(out, "\n## rustkb catalog: tier **{}** ({})\n\n", c.tier.as_str(), c.category);
                out.push_str(&c.to_doc().body);
            }
            None => out.push_str("\n_Not in the rustkb catalog — evaluate with the dependency policy (`get_doc ecosystem/dependency-policy`)._\n"),
        }
        let replacements: Vec<_> = self
            .catalog
            .replacements_for(name)
            .map(|c| format!("`{}` ({})", c.name, c.tier.as_str()))
            .collect();
        if !replacements.is_empty() {
            let _ = writeln!(out, "\n**Superseded by:** {}", replacements.join(", "));
        }
        if let Ok(advs) = self.advisories() {
            let norm = name.replace('_', "-").to_ascii_lowercase();
            let hits: Vec<_> = advs
                .iter()
                .filter(|a| a.krate.replace('_', "-").to_ascii_lowercase() == norm && !a.withdrawn)
                .collect();
            if !hits.is_empty() {
                let _ = writeln!(out, "\n## Advisories ({})", hits.len());
                for a in hits.iter().rev().take(10) {
                    let _ = writeln!(
                        out,
                        "- {} [{}] {} — patched: {}",
                        a.id,
                        a.kind,
                        a.summary,
                        if a.patched.is_empty() {
                            "none".into()
                        } else {
                            a.patched.join(", ")
                        }
                    );
                }
            }
        }
        out
    }

    pub(crate) fn recommend(
        &self,
        need: &str,
        category: Option<&str>,
        limit: usize,
    ) -> Result<String> {
        let limit = limit.clamp(1, 20);
        let text = match category {
            Some(c) => format!("{need} {c}"),
            None => need.to_owned(),
        };
        let hits = self.index().search(&SearchQuery {
            text,
            sources: vec![Source::Catalog],
            limit: 40,
            ..Default::default()
        })?;

        // Asking about a superseded crate? Lead with what replaces it, and nothing else.
        if let Some(old) = hits
            .first()
            .and_then(|h| self.catalog.get(&h.doc.title))
            .filter(|e| e.tier == Tier::Avoid && names_crate(need, &e.name))
        {
            return Ok(self.render_superseded(old));
        }

        let words: Vec<String> = need
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2)
            .map(str::to_ascii_lowercase)
            .collect();
        let top_score = hits.first().map_or(0.0, |h| h.score);
        // Re-rank: the ecosystem default for a category should beat a niche crate that merely
        // mentions the words; a category match is a strong signal of intent. Weak matches that
        // only share a common word with the query are dropped.
        let mut ranked: Vec<(f32, &CatalogEntry)> = hits
            .iter()
            .filter(|h| h.score >= top_score * 0.15)
            .filter_map(|h| self.catalog.get(&h.doc.title).map(|e| (h.score, e)))
            .map(|(score, e)| {
                let tier = match e.tier {
                    Tier::Default => 2.0,
                    Tier::Recommended => 1.25,
                    Tier::Situational => 0.8,
                    Tier::Avoid => 1.0,
                };
                let category = e.category.to_ascii_lowercase();
                let cat = if words.iter().any(|w| category.contains(w.as_str())) {
                    1.5
                } else {
                    1.0
                };
                (score * tier * cat, e)
            })
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        // Categories in order of their best match; within a category, the curated tier order
        // (default → recommended → situational) decides, so the ecosystem default leads.
        let mut category_rank: Vec<&str> = Vec::new();
        for (_, e) in &ranked {
            if !category_rank.contains(&e.category.as_str()) {
                category_rank.push(&e.category);
            }
        }
        let cat_pos = |e: &CatalogEntry| {
            category_rank
                .iter()
                .position(|c| *c == e.category)
                .unwrap_or(usize::MAX)
        };
        ranked.sort_by(|a, b| {
            cat_pos(a.1)
                .cmp(&cat_pos(b.1))
                .then(a.1.tier.cmp(&b.1.tier))
                .then(b.0.total_cmp(&a.0))
        });
        let (avoid, good): (Vec<_>, Vec<_>) = ranked
            .into_iter()
            .map(|(_, e)| e)
            .partition(|e| e.tier == Tier::Avoid);

        let mut out = format!("Crates for “{need}”:\n");
        if good.is_empty() && avoid.is_empty() {
            out.push_str("\nNo catalog match. Search crates.io/lib.rs and apply `get_doc ecosystem/dependency-policy`.\n");
        }
        for e in good.iter().take(limit) {
            let _ = write!(
                out,
                "\n- **{}** ({}, tier {}, v{}) — {}",
                e.name,
                e.category,
                e.tier.as_str(),
                e.version,
                e.summary
            );
            if !e.use_for.is_empty() {
                let _ = write!(out, "\n  use for: {}", e.use_for);
            }
            if !e.avoid_when.is_empty() {
                let _ = write!(out, "\n  avoid when: {}", e.avoid_when);
            }
        }
        if !avoid.is_empty() {
            out.push_str("\n\n**Do not use** (superseded/unmaintained):");
            for e in avoid.iter().take(5) {
                let _ = write!(
                    out,
                    "\n- ~~{}~~ → use {} — {}",
                    e.name,
                    e.alternatives.join(" / "),
                    e.summary
                );
            }
        }
        out.push_str("\n\nVerify current versions with `crate_info` before adding.");
        Ok(out)
    }

    fn render_superseded(&self, old: &CatalogEntry) -> String {
        let mut out = format!(
            "`{}` should not be used in new code: {}\n\n**Use instead:** {}\n",
            old.name,
            old.summary,
            old.alternatives.join(" / ")
        );
        let mut seen = std::collections::HashSet::new();
        let replacements = old
            .alternatives
            .iter()
            .filter_map(|a| self.catalog.get(a.split_whitespace().next().unwrap_or(a)))
            .chain(self.catalog.replacements_for(&old.name))
            .filter(|e| e.tier != Tier::Avoid && seen.insert(e.name.clone()));
        for e in replacements {
            let _ = write!(
                out,
                "\n- **{}** (tier {}, v{}) — {}",
                e.name,
                e.tier.as_str(),
                e.version,
                e.summary
            );
        }
        if !old.notes.is_empty() {
            let _ = write!(out, "\n\nNotes: {}", old.notes);
        }
        out.push_str("\n\nMigration details: `search` \"replace <crate>\" (ecosystem/deprecated-and-replacements).");
        out
    }

    /// Stored advisories, refreshed from OSV at most daily (falls back to stale data offline).
    pub(crate) fn advisories(&self) -> Result<Vec<Advisory>> {
        let file = self.paths.data.join(ADVISORY_FILE);
        let fresh = std::fs::metadata(&file)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < rustkb_ingest::http::DAY);
        if !fresh {
            match advisories::fetch(&self.http, rustkb_ingest::http::DAY) {
                Ok((_, advs)) => {
                    std::fs::create_dir_all(&self.paths.data)?;
                    std::fs::write(&file, serde_json::to_vec(&advs)?)?;
                    return Ok(advs);
                }
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), "advisory refresh failed; using cached data");
                }
            }
        }
        let bytes = std::fs::read(&file).context("no advisory data yet and download failed")?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Advisories for one `crate@version`, or every registry package in a Cargo.lock.
    pub(crate) fn check_advisories(
        &self,
        krate: Option<&str>,
        version: Option<&str>,
        lockfile: Option<&str>,
    ) -> Result<String> {
        let mut packages: Vec<(String, String)> = Vec::new();
        if let Some(lock) = lockfile {
            let text = std::fs::read_to_string(lock).with_context(|| format!("reading {lock}"))?;
            let parsed: toml::Value = toml::from_str(&text).context("parsing Cargo.lock")?;
            for p in parsed
                .get("package")
                .and_then(|p| p.as_array())
                .into_iter()
                .flatten()
            {
                let from_registry = p
                    .get("source")
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| s.starts_with("registry+") || s.starts_with("sparse+"));
                if let (true, Some(n), Some(v)) = (
                    from_registry,
                    p.get("name").and_then(|n| n.as_str()),
                    p.get("version").and_then(|v| v.as_str()),
                ) {
                    packages.push((n.to_owned(), v.to_owned()));
                }
            }
        }
        match (krate, version) {
            (Some(k), Some(v)) => packages.push((k.to_owned(), v.to_owned())),
            (Some(k), None) => {
                let latest = cratesio::info(&self.http, k)?
                    .max_stable_version
                    .context("crate has no stable release")?;
                packages.push((k.to_owned(), latest));
            }
            _ => {}
        }
        if packages.is_empty() {
            bail!("pass `crate` (+ optional `version`) or `lockfile`");
        }

        // Single package: live OSV query is authoritative; fall back to the local mirror.
        if packages.len() == 1 && lockfile.is_none() {
            let (k, v) = &packages[0];
            if let Ok(live) = advisories::check_live(&self.http, k, v) {
                return Ok(render_advisories(
                    &[(k.clone(), v.clone(), live)],
                    1,
                    "live OSV",
                ));
            }
        }
        let all = self.advisories()?;
        let mut affected = Vec::new();
        for (k, v) in &packages {
            let hits = advisories::check_offline(&all, k, v).unwrap_or_default();
            if !hits.is_empty() {
                affected.push((k.clone(), v.clone(), hits.into_iter().cloned().collect()));
            }
        }
        Ok(render_advisories(
            &affected,
            packages.len(),
            "RustSec/OSV mirror (≤1 day old)",
        ))
    }

    pub(crate) fn explain_lint(&self, name: &str) -> Result<String> {
        let name = name.trim().trim_start_matches("clippy::").replace('-', "_");
        if let Some(doc) = self.index().get(&format!("clippy:{name}"))? {
            return Ok(render_doc(&doc));
        }
        let total = self.index().list(Source::Clippy, None, 1)?.len();
        if total == 0 {
            bail!("clippy lints not ingested yet — run `rustkb ingest --only clippy`");
        }
        self.search(&name, &["clippy".into()], None, 5)
    }

    pub(crate) fn release(&self, version: Option<&str>, since: Option<&str>) -> Result<String> {
        let mut all = self.index().list(Source::Release, None, 1000)?;
        if all.is_empty() {
            bail!("release notes not ingested yet — run `rustkb ingest --only releases`");
        }
        let today = jiff::Zoned::now().date().to_string();
        let key = |d: &Doc| {
            d.version
                .as_deref()
                .and_then(|v| semver::Version::parse(v).ok())
        };
        all.retain(|d| d.kind.as_deref().is_none_or(|date| date <= today.as_str()));
        all.sort_by_key(|d| std::cmp::Reverse(key(d)));
        let normalize = |v: &str| {
            let v = v.trim().trim_start_matches('v');
            match v.matches('.').count() {
                1 => format!("{v}.0"),
                _ => v.to_owned(),
            }
        };
        if let Some(since) = since {
            let since =
                semver::Version::parse(&normalize(since)).context("`since` must look like 1.85")?;
            let mut newer: Vec<&Doc> = all
                .iter()
                .filter(|d| key(d).is_some_and(|v| v > since))
                .collect();
            newer.reverse();
            let mut out = format!("Rust releases after {since} ({}):\n", newer.len());
            for d in newer {
                let _ = write!(
                    out,
                    "\n## {} ({})\n{}\n",
                    d.title,
                    d.kind.as_deref().unwrap_or("?"),
                    highlights(&d.body, 25)
                );
            }
            return Ok(out);
        }
        let doc = match version
            .map(normalize)
            .filter(|v| v != "latest.0" && v != "latest")
        {
            Some(v) => all
                .iter()
                .find(|d| d.version.as_deref() == Some(v.as_str()))
                .with_context(|| format!("no release notes for {v}"))?,
            None => all.first().context("no releases")?,
        };
        Ok(render_doc(doc))
    }

    pub(crate) fn latest_rust(&self) -> Option<String> {
        let today = jiff::Zoned::now().date().to_string();
        let mut all = self.index().list(Source::Release, None, 1000).ok()?;
        all.retain(|d| d.kind.as_deref().is_none_or(|date| date <= today.as_str()));
        all.into_iter()
            .filter_map(|d| d.version.and_then(|v| semver::Version::parse(&v).ok()))
            .max()
            .map(|v| v.to_string())
    }

    /// Drift findings for curated docs and the catalog (hits crates.io, ≤1 req/s uncached).
    pub(crate) fn stale(&self, offline: bool) -> Vec<stale::Finding> {
        let names = stale::referenced_crates(&self.curated, &self.catalog);
        let crates: BTreeMap<String, cratesio::CrateInfo> = if offline {
            BTreeMap::new()
        } else {
            cratesio::info_many(&self.http, &names)
                .into_iter()
                .filter_map(|(n, r)| match r {
                    Ok(i) => Some((n, i)),
                    Err(e) => {
                        tracing::warn!(krate = %n, error = %format!("{e:#}"), "crates.io lookup failed");
                        None
                    }
                })
                .collect()
        };
        let advisories = if offline {
            std::fs::read(self.paths.data.join(ADVISORY_FILE))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default()
        } else {
            self.advisories().unwrap_or_default()
        };
        let latest_rust = self.latest_rust();
        stale::check(&stale::Inputs {
            report: &self.curated,
            catalog: &self.catalog,
            crates: &crates,
            advisories: &advisories,
            latest_rust: latest_rust.as_deref(),
            today: jiff::Zoned::now().date(),
            thresholds: stale::Thresholds::default(),
        })
    }

    pub(crate) fn status(&self) -> Result<String> {
        let state = read_state(&self.paths);
        let mut out = String::from("# rustkb status\n\n");
        let _ = writeln!(out, "- knowledge root: `{}`", self.paths.root.display());
        let _ = writeln!(out, "- data dir: `{}`", self.paths.data.display());
        let _ = writeln!(
            out,
            "- curated reference files: {} ({} problems)",
            self.curated.files.len(),
            self.curated.errors.len()
        );
        let _ = writeln!(
            out,
            "- catalog entries: {} ({} with tracked docs)",
            self.catalog.crates.len(),
            self.catalog.tracked().count()
        );
        let _ = writeln!(
            out,
            "- semantic search: {}",
            if cfg!(feature = "semantic") && state.semantic {
                "on"
            } else if cfg!(feature = "semantic") {
                "built without vectors (run `rustkb index`)"
            } else {
                "off (build with --features semantic)"
            }
        );
        if let Some(r) = self.latest_rust() {
            let _ = writeln!(out, "- latest stable Rust (per release notes): {r}");
        }
        out.push_str("\n## Indexed documents\n");
        for (source, n) in self.index().stats()? {
            let last = state.last_ingest.get(source.as_str()).or_else(|| {
                state.last_ingest.get(match source {
                    Source::Advisory => "advisories",
                    Source::Release => "releases",
                    _ => "",
                })
            });
            let _ = writeln!(
                out,
                "- {source}: {n}{}",
                last.map(|t| format!(" (refreshed {t})"))
                    .unwrap_or_default()
            );
        }
        let versions = self.store.rustdoc_versions();
        if !versions.is_empty() {
            let _ = writeln!(
                out,
                "\nrustdoc crates: {}",
                versions
                    .iter()
                    .map(|(k, v)| format!("{k}@{v}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !self.curated.errors.is_empty() {
            out.push_str("\n## Curated problems\n");
            for (p, e) in &self.curated.errors {
                let _ = writeln!(out, "- `{}`: {e}", p.display());
            }
        }
        Ok(out)
    }
}

fn all_docs(paths: &Paths, store: &Store) -> Result<Vec<Doc>> {
    let (mut docs, report) = curated::all_docs(paths)?;
    for (path, err) in &report.errors {
        tracing::warn!(path = %path.display(), %err, "curated doc problem");
    }
    docs.extend(store.read_all()?);
    Ok(docs)
}

fn ingest_rustdoc(http: &Http, store: &Store, krate: &str, version: &str) -> Result<Vec<Doc>> {
    let docs = rustdoc::fetch(http, krate, version)?;
    let actual = docs
        .first()
        .and_then(|d| d.version.clone())
        .unwrap_or_else(|| version.to_owned());
    // Keep at most the two newest versions of each crate on disk.
    let mut existing: Vec<String> = store
        .rustdoc_versions()
        .into_iter()
        .filter(|(k, _)| k == krate)
        .map(|(_, v)| v)
        .collect();
    existing.sort_by_key(|v| std::cmp::Reverse(semver::Version::parse(v).ok()));
    for old in existing.iter().filter(|v| **v != actual).skip(1) {
        let _ = std::fs::remove_file(store.rustdoc_file(krate, old));
    }
    store.write(&store.rustdoc_file(krate, &actual), &docs)?;
    Ok(docs)
}

#[cfg(feature = "semantic")]
fn attach_embeddings(
    paths: &Paths,
    store: &Store,
    index: Index,
    state: &mut State,
) -> Result<Index> {
    use rustkb_index::Embeddings;
    let emb = if state.semantic {
        Embeddings::load(&paths.vectors_dir()).map_err(|e| anyhow::anyhow!("{e}"))?
    } else {
        None
    };
    let emb = if let Some(e) = emb {
        Some(e)
    } else {
        tracing::info!("building embeddings (first run downloads a ~30 MB model)");
        match Embeddings::build(&paths.vectors_dir(), &all_docs(paths, store)?) {
            Ok(e) => {
                state.semantic = true;
                write_state(paths, state)?;
                Some(e)
            }
            Err(e) => {
                tracing::warn!(error = %e, "semantic search unavailable; continuing lexical-only");
                None
            }
        }
    };
    Ok(match emb {
        Some(e) => index.with_embeddings(e),
        None => index,
    })
}

fn read_state(paths: &Paths) -> State {
    std::fs::read(paths.state_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_state(paths: &Paths, state: &State) -> Result<()> {
    std::fs::create_dir_all(&paths.data)?;
    std::fs::write(paths.state_file(), serde_json::to_vec_pretty(state)?)?;
    Ok(())
}

/// Cheap change detector for the curated corpus: paths, sizes and mtimes under `skills/`.
fn fingerprint(paths: &Paths) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for e in walkdir::WalkDir::new(paths.skills_dir())
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
    {
        if e.file_type().is_file() {
            e.path().hash(&mut h);
            if let Ok(m) = e.metadata() {
                m.len().hash(&mut h);
                m.modified().ok().hash(&mut h);
            }
        }
    }
    h.finish()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

fn render_doc(d: &Doc) -> String {
    let mut out = format!("# {}\n\n", d.title);
    let mut meta = vec![format!("source: {}", d.source)];
    if let Some(k) = &d.krate {
        meta.push(format!("crate: {k}"));
    }
    if let Some(v) = &d.version {
        meta.push(format!("version: {v}"));
    }
    if let Some(k) = &d.kind {
        meta.push(format!("kind: {k}"));
    }
    if let Some(p) = &d.path {
        meta.push(format!("path: `{p}`"));
    }
    if let Some(u) = &d.url {
        meta.push(format!("url: {u}"));
    }
    let _ = write!(out, "_{}_\n\n{}", meta.join(" · "), d.body);
    out
}

fn render_file(f: &curated::RefFile) -> String {
    let m = &f.meta;
    let crates = m
        .crates
        .iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "# {}\n\n_id: `{}` · file: `{}` · verified {} on Rust {}{}_\n\n> {}\n{}",
        m.title,
        m.id,
        f.rel,
        m.verified,
        m.rust,
        if crates.is_empty() {
            String::new()
        } else {
            format!(" · crates: {crates}")
        },
        m.summary.trim(),
        f.body
    )
}

fn render_advisories(
    affected: &[(String, String, Vec<Advisory>)],
    checked: usize,
    via: &str,
) -> String {
    if affected.is_empty() {
        return format!("✅ No known advisories for {checked} package(s) (via {via}).");
    }
    let n: usize = affected.iter().map(|(_, _, a)| a.len()).sum();
    let mut out = format!(
        "⚠️ {n} advisories across {} of {checked} package(s) (via {via}):\n",
        affected.len()
    );
    for (k, v, advs) in affected {
        for a in advs {
            let _ = write!(
                out,
                "\n- **{k} {v}** — {} [{}{}]: {}\n  fix: {}\n  {}",
                a.id,
                a.kind,
                a.severity
                    .as_deref()
                    .map(|s| format!(", {s}"))
                    .unwrap_or_default(),
                a.summary,
                fix_hint(v, &a.patched),
                a.url
            );
        }
    }
    out
}

/// Whether `query` names `krate` (all of its `-`/`_`-separated parts appear as words).
fn names_crate(query: &str, krate: &str) -> bool {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_ascii_lowercase)
        .collect();
    krate
        .split(['-', '_'])
        .all(|part| words.iter().any(|w| w == &part.to_ascii_lowercase()))
}

/// The nearest patched release above `version` (what an upgrade should target).
fn fix_hint(version: &str, patched: &[String]) -> String {
    let current = semver::Version::parse(version).ok();
    let above: Vec<semver::Version> = patched
        .iter()
        .filter_map(|p| semver::Version::parse(p).ok())
        .filter(|p| current.as_ref().is_none_or(|c| p > c))
        .collect();
    match (above.iter().min(), above.iter().max()) {
        (Some(lo), Some(hi)) if lo != hi => {
            format!("nearest unaffected release {lo}; newest fix {hi}")
        }
        (Some(v), _) => format!("upgrade to >= {v}"),
        (None, _) if patched.is_empty() => "no patched version — replace the crate".into(),
        (None, _) => format!("patched in {}", patched.join(", ")),
    }
}

/// The most useful parts of a release: stabilised language features and APIs.
fn highlights(body: &str, max_lines: usize) -> String {
    let mut out = Vec::new();
    let mut keep = false;
    let mut lines = body.lines().peekable();
    while let Some(line) = lines.next() {
        if lines.peek().is_some_and(|n| n.starts_with("---")) {
            keep = matches!(
                line.trim(),
                "Language" | "Libraries" | "Stabilized APIs" | "Cargo" | "Compatibility Notes"
            );
            if keep {
                out.push(format!("**{}**", line.trim()));
            }
            lines.next();
            continue;
        }
        if keep && line.trim_start().starts_with("- ") {
            out.push(line.to_owned());
        }
        if out.len() >= max_lines {
            out.push("- …".into());
            break;
        }
    }
    out.join("\n")
}
