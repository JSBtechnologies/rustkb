//! `rustkb` — a versioned Rust knowledge base for coding agents (CLI + MCP server).

mod kb;
mod mcp;

use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use rustkb_core::Paths;

use crate::kb::{IngestOptions, Kb};

#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Knowledge root (directory containing `skills/`). Default: $`RUSTKB_ROOT`, the plugin root, or auto-discovery.
    #[arg(long, global = true)]
    root: Option<std::path::PathBuf>,
    /// Data directory for downloads and the index. Default: $`RUSTKB_HOME`, the plugin data dir, or the platform data dir.
    #[arg(long, global = true)]
    home: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the MCP server on stdio.
    Serve,
    /// Download/refresh upstream sources, then rebuild the index.
    Ingest {
        /// Only these sources: corpus, clippy, advisories, releases, rustdoc.
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Also ingest rustdoc for `crate` or `crate@version` (repeatable).
        #[arg(long = "crate")]
        crates: Vec<String>,
        /// Ignore caches and re-download everything.
        #[arg(long)]
        force: bool,
    },
    /// Rebuild the search index from curated files and stored docs (offline).
    Index,
    /// Search the knowledge base.
    Search {
        query: Vec<String>,
        #[arg(long, short, value_delimiter = ',')]
        source: Vec<String>,
        #[arg(long = "crate")]
        krate: Option<String>,
        #[arg(long, short = 'n', default_value_t = 8)]
        limit: usize,
    },
    /// Print a document by id.
    Doc { id: String },
    /// Show API docs for a crate item (fetches from docs.rs on demand).
    Item {
        #[arg(value_name = "CRATE")]
        krate: String,
        path: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },
    /// crates.io facts + catalog recommendation for a crate.
    Crate { name: String },
    /// Recommend crates for a need.
    Recommend { need: Vec<String> },
    /// Check advisories for `crate[@version]` or a Cargo.lock.
    Audit {
        /// `crate` or `crate@version`.
        spec: Option<String>,
        #[arg(long)]
        lockfile: Option<String>,
    },
    /// Explain a clippy lint.
    Lint { name: String },
    /// Rust release notes (latest, a version, or everything since a version).
    Release {
        version: Option<String>,
        #[arg(long)]
        since: Option<String>,
    },
    /// Report curated guidance that may have drifted from reality.
    Stale {
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
        /// Skip network lookups (only age/Rust-version checks).
        #[arg(long)]
        offline: bool,
        /// Exit non-zero when high-severity findings exist.
        #[arg(long)]
        strict: bool,
    },
    /// Validate curated frontmatter and the catalog (for CI).
    Validate,
    /// Show index and corpus status.
    Status,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum Format {
    Text,
    Json,
    Markdown,
}

fn main() -> ExitCode {
    // Logs go to stderr: stdout is the MCP transport.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("RUSTKB_LOG")
                .unwrap_or_else(|_| "rustkb=info,warn".into()),
        )
        .init();
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn paths(cli: &Cli) -> Result<Paths> {
    if let (Some(root), Some(home)) = (&cli.root, &cli.home) {
        return Ok(Paths::new(root, home));
    }
    let mut paths = Paths::discover()?;
    if let Some(root) = &cli.root {
        paths.root.clone_from(root);
        paths.managed_root = false;
    }
    if let Some(home) = &cli.home {
        paths.data.clone_from(home);
    }
    Ok(paths)
}

fn run(cli: Cli) -> Result<ExitCode> {
    let paths = paths(&cli)?;
    let print = |s: String| {
        println!("{s}");
        Ok(ExitCode::SUCCESS)
    };
    match cli.command {
        Command::Serve => {
            let kb = Kb::open(paths)?;
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(mcp::serve(kb))?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Ingest {
            only,
            crates,
            force,
        } => {
            for line in Kb::ingest(
                &paths,
                &IngestOptions {
                    only,
                    crates,
                    force,
                },
            )? {
                println!("{line}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Index => {
            let counts = Kb::rebuild(&paths)?;
            print(format!(
                "index rebuilt at {}: {counts:?}",
                paths.index_dir().display()
            ))
        }
        Command::Search {
            query,
            source,
            krate,
            limit,
        } => print(Kb::open(paths)?.search(&query.join(" "), &source, krate.as_deref(), limit)?),
        Command::Doc { id } => print(Kb::open(paths)?.get_doc(&id)?),
        Command::Item {
            krate,
            path,
            version,
        } => print(Kb::open(paths)?.get_item(&krate, path.as_deref(), version.as_deref())?),
        Command::Crate { name } => print(Kb::open(paths)?.crate_info(&name)),
        Command::Recommend { need } => {
            print(Kb::open(paths)?.recommend(&need.join(" "), None, 5)?)
        }
        Command::Audit { spec, lockfile } => {
            let (krate, version) = match spec
                .as_deref()
                .map(|s| s.split_once('@').map_or((s, None), |(k, v)| (k, Some(v))))
            {
                Some((k, v)) => (Some(k), v),
                None => (None, None),
            };
            print(Kb::open(paths)?.check_advisories(krate, version, lockfile.as_deref())?)
        }
        Command::Lint { name } => print(Kb::open(paths)?.explain_lint(&name)?),
        Command::Release { version, since } => {
            print(Kb::open(paths)?.release(version.as_deref(), since.as_deref())?)
        }
        Command::Stale {
            format,
            offline,
            strict,
        } => {
            let findings = Kb::open(paths)?.stale(offline);
            match format {
                Format::Json => println!("{}", serde_json::to_string_pretty(&findings)?),
                Format::Markdown => println!("{}", rustkb_ingest::stale::to_markdown(&findings)),
                Format::Text => {
                    if findings.is_empty() {
                        println!("No drift detected.");
                    }
                    for f in &findings {
                        println!(
                            "[{:?}] {} · {}: {}",
                            f.severity, f.location, f.subject, f.message
                        );
                    }
                }
            }
            let high = findings
                .iter()
                .any(|f| f.severity == rustkb_ingest::stale::Severity::High);
            Ok(if strict && high {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            })
        }
        Command::Validate => validate(&paths),
        Command::Status => print(Kb::open(paths)?.status()?),
    }
}

fn validate(paths: &Paths) -> Result<ExitCode> {
    let report = rustkb_ingest::curated::load(paths);
    let mut problems: Vec<String> = report
        .errors
        .iter()
        .map(|(p, e)| format!("{}: {e}", p.display()))
        .collect();
    match rustkb_core::Catalog::load(&paths.catalog_file()) {
        Ok(cat) => {
            let mut seen = std::collections::HashSet::new();
            for c in &cat.crates {
                if !seen.insert(c.name.to_ascii_lowercase().replace('_', "-")) {
                    problems.push(format!("catalog.toml: duplicate crate `{}`", c.name));
                }
                if c.tier == rustkb_core::Tier::Avoid && c.alternatives.is_empty() {
                    problems.push(format!(
                        "catalog.toml: `{}` is tier avoid but lists no alternatives",
                        c.name
                    ));
                }
            }
            for name in cat.maintained_checked.keys() {
                if cat.get(name).is_none() {
                    problems.push(format!(
                        "catalog.toml: [maintained_checked] names `{name}`, which is not a catalog crate"
                    ));
                }
            }
            println!("catalog: {} entries", cat.crates.len());
        }
        Err(e) => problems.push(e.to_string()),
    }
    for skill in std::fs::read_dir(paths.skills_dir())?.filter_map(Result::ok) {
        let file = skill.path().join("SKILL.md");
        match std::fs::read_to_string(&file) {
            Ok(text) => {
                let (fm, _) = rustkb_core::split_frontmatter(&text);
                let fm = fm.unwrap_or_default();
                for key in ["name:", "description:"] {
                    if !fm.lines().any(|l| l.starts_with(key)) {
                        problems.push(format!("{}: frontmatter missing `{key}`", file.display()));
                    }
                }
            }
            Err(_) => problems.push(format!("{}: missing", file.display())),
        }
    }
    println!("reference files: {}", report.files.len());
    if problems.is_empty() {
        println!("OK");
        Ok(ExitCode::SUCCESS)
    } else {
        for p in &problems {
            println!("✗ {p}");
        }
        Ok(ExitCode::FAILURE)
    }
}
