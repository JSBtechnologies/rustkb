//! MCP server exposing the knowledge base as tools (stdio transport).

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::kb::Kb;

const INSTRUCTIONS: &str = "\
rustkb: curated, versioned Rust knowledge for designing and building best-practice Rust projects.

Workflow:
- Before designing or writing Rust, `search` for guidance (e.g. \"error handling library\", \"workspace layout\", \"axum graceful shutdown\"). Curated guidance (source=curated) carries rule ids like ERR-01 you can cite.
- Before adding a dependency: `recommend_crates` for the need, then `crate_info` for the live version, maintenance and advisories. Never add a crate from memory.
- For exact, version-correct APIs: `get_item` (crate + path, e.g. tokio + sync::Mutex). Fetched from docs.rs on demand.
- Security: `check_advisories` for a crate@version or a whole Cargo.lock.
- `explain_lint` for clippy lints; `rust_release` for what changed in a Rust version (use `since` to see features newer than your training data).
- Skill overviews: get_doc curated:skill/rust-idioms | rust-architecture | rust-ecosystem | rust-security.";

#[derive(Debug, Clone)]
pub(crate) struct Server {
    kb: Arc<Kb>,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SearchArgs {
    /// Natural language or identifiers, e.g. "cancellation safety select", "`tokio::sync::Mutex`", "ERR-03".
    pub query: String,
    /// Restrict to sources: curated, catalog, rustdoc, clippy, advisory, release.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Restrict to one crate (applies to rustdoc/catalog/advisory docs).
    #[serde(rename = "crate", default)]
    pub krate: Option<String>,
    /// Max results (default 8, max 50).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct IdArgs {
    /// A doc id from `search` (e.g. `curated:idioms/error-handling#err-01-...`, `clippy:unwrap_used`),
    /// a curated file id (`idioms/error-handling`) for the whole file, or `curated:skill/<name>`.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ItemArgs {
    /// Crate name as on crates.io, e.g. "tokio", "`serde_json`".
    #[serde(rename = "crate")]
    pub krate: String,
    /// Item path, with or without the crate prefix: "`sync::Mutex`", "`tokio::spawn`", "`Value::as_str`". Omit for the crate root.
    #[serde(default)]
    pub path: Option<String>,
    /// Exact crate version (e.g. "1.47.1"). Omit for the latest indexed/published version.
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct CrateArgs {
    /// Crate name on crates.io.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RecommendArgs {
    /// What you need, e.g. "http client", "parse yaml config", "structured logging", "cli argument parsing".
    pub need: String,
    /// Optional catalog category to bias toward.
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct AdvisoryArgs {
    /// Crate name (checks `version`, or the latest release if omitted).
    #[serde(rename = "crate", default)]
    pub krate: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    /// Absolute path to a Cargo.lock (or a project directory containing one) to audit every
    /// crates.io dependency. Path and git dependencies are reported but not checked.
    #[serde(default)]
    pub lockfile: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct LintArgs {
    /// Clippy lint name, e.g. "`unwrap_used`" or "`clippy::needless_collect`".
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ReleaseArgs {
    /// A version like "1.85" / "1.85.0", or omit for the latest stable release.
    #[serde(default)]
    pub version: Option<String>,
    /// List highlights of every release after this version (e.g. "1.80").
    #[serde(default)]
    pub since: Option<String>,
}

type ToolResult = Result<String, String>;

impl Server {
    pub(crate) fn new(kb: Arc<Kb>) -> Self {
        Self {
            kb,
            tool_router: Self::tool_router(),
        }
    }

    async fn run<F>(&self, f: F) -> ToolResult
    where
        F: FnOnce(&Kb) -> anyhow::Result<String> + Send + 'static,
    {
        let kb = Arc::clone(&self.kb);
        match tokio::task::spawn_blocking(move || f(&kb)).await {
            Ok(Ok(text)) => Ok(text),
            Ok(Err(e)) => Err(format!("{e:#}")),
            Err(e) => Err(format!("internal error: {e}")),
        }
    }
}

#[tool_router(router = tool_router)]
impl Server {
    #[tool(
        description = "Search Rust guidance, crate catalog, API docs, clippy lints, advisories and release notes. Returns ranked ids + snippets; follow up with get_doc."
    )]
    async fn search(&self, Parameters(a): Parameters<SearchArgs>) -> ToolResult {
        self.run(move |kb| {
            kb.search(
                &a.query,
                &a.sources,
                a.krate.as_deref(),
                a.limit.unwrap_or(8),
            )
        })
        .await
    }

    #[tool(
        description = "Full text of a document by id (from search), or a whole curated guide by file id like 'idioms/error-handling'."
    )]
    async fn get_doc(&self, Parameters(a): Parameters<IdArgs>) -> ToolResult {
        self.run(move |kb| kb.get_doc(&a.id)).await
    }

    #[tool(
        description = "Version-accurate API docs for a crate item (signature, docs, members) from docs.rs rustdoc JSON; fetched on demand."
    )]
    async fn get_item(&self, Parameters(a): Parameters<ItemArgs>) -> ToolResult {
        self.run(move |kb| kb.get_item(&a.krate, a.path.as_deref(), a.version.as_deref()))
            .await
    }

    #[tool(
        description = "Live crates.io facts for a crate (latest version, MSRV, license, last release, downloads) plus rustkb's recommendation tier, replacements and advisories."
    )]
    async fn crate_info(&self, Parameters(a): Parameters<CrateArgs>) -> ToolResult {
        self.run(move |kb| Ok(kb.crate_info(&a.name))).await
    }

    #[tool(
        description = "Recommend crates for a need from the curated catalog, with tiers and when-to-avoid notes; also flags deprecated crates to avoid."
    )]
    async fn recommend_crates(&self, Parameters(a): Parameters<RecommendArgs>) -> ToolResult {
        self.run(move |kb| kb.recommend(&a.need, a.category.as_deref(), a.limit.unwrap_or(5)))
            .await
    }

    #[tool(
        description = "Check RustSec/OSV security advisories for crate@version or every registry package in a Cargo.lock."
    )]
    async fn check_advisories(&self, Parameters(a): Parameters<AdvisoryArgs>) -> ToolResult {
        self.run(move |kb| {
            kb.check_advisories(
                a.krate.as_deref(),
                a.version.as_deref(),
                a.lockfile.as_deref(),
            )
        })
        .await
    }

    #[tool(
        description = "Explain a clippy lint: what it catches, why, examples, configuration and how to enable it."
    )]
    async fn explain_lint(&self, Parameters(a): Parameters<LintArgs>) -> ToolResult {
        self.run(move |kb| kb.explain_lint(&a.name)).await
    }

    #[tool(
        description = "Rust release notes: one version, the latest, or highlights of everything newer than `since` (catch up on features newer than your training data)."
    )]
    async fn rust_release(&self, Parameters(a): Parameters<ReleaseArgs>) -> ToolResult {
        self.run(move |kb| kb.release(a.version.as_deref(), a.since.as_deref()))
            .await
    }

    #[tool(
        description = "Knowledge base status: indexed sources, freshness, curated corpus health."
    )]
    async fn kb_status(&self) -> ToolResult {
        self.run(Kb::status).await
    }
}

// rmcp's `#[tool_handler]` expands to async trait fns with no `.await` (flagged since clippy
// 1.98); `unknown_lints` keeps older toolchains from rejecting the lint name.
#[allow(unknown_lints, clippy::unused_async_trait_impl)]
#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("rustkb", env!("CARGO_PKG_VERSION"))
                    .with_title("Rust knowledge base"),
            )
            .with_instructions(INSTRUCTIONS)
    }
}

pub(crate) async fn serve(kb: Kb) -> anyhow::Result<()> {
    let server = Server::new(Arc::new(kb));
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
