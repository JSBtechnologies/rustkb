---
id: architecture/cli-apps
title: Command-line applications
summary: >-
  How to structure a Rust CLI: clap derive types in their own module, a thin main that maps errors
  to exit codes, data on stdout vs diagnostics on stderr, broken-pipe handling, config precedence
  (flags > env > file > defaults), logging and progress output, and how to test it.
area: architecture
tags: [cli, clap, exit-code, stdout, stderr, anyhow, config, indicatif, progress, assert_cmd]
rust: "1.96"
edition: "2024"
crates:
  clap: "4.6"
  anyhow: "1.0"
  thiserror: "2.0"
  serde: "1.0"
  serde_json: "1.0"
  toml: "1.1"
  directories: "6.0"
  indicatif: "0.18"
  tracing: "0.1"
  tracing-subscriber: "0.3"
  assert_cmd: "2.2"
  predicates: "3.1"
  trycmd: "1.2"
  cargo-dist: "0.32"
verified: 2026-09-29
sources:
  - https://docs.rs/clap/4/clap/_derive/index.html
  - https://rust-cli.github.io/book/index.html
  - https://clig.dev/
  - https://no-color.org/
---

# Command-line applications

Default stack: `clap` (derive) for arguments, `anyhow` in `main`, `thiserror` for errors that need
their own exit code, sync code (no tokio unless the tool does concurrent network IO — ARCH-06).
The code below compiles and passes clippy pedantic on Rust 1.96.

## Structure

### CLI-01: Arguments are types in `cli.rs`; `main.rs` only dispatches

```text
src/
├── main.rs       # parse → run → map error to exit code
├── cli.rs        # #[derive(Parser)] types only
├── settings.rs   # merge flags/env/file/defaults (CLI-06)
└── commands/     # one module per subcommand when they grow: import.rs, list.rs
```

```rust
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Manage shop orders from the command line.
#[derive(Debug, Parser)]
#[command(version, about, propagate_version = true)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub(crate) global: GlobalArgs,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Args)]
pub(crate) struct GlobalArgs {
    /// Increase diagnostic output (-v, -vv).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub(crate) verbose: u8,

    /// Path to the config file.
    #[arg(long, env = "SHOP_CONFIG", global = true)]
    pub(crate) config: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Import orders from a file.
    Import {
        /// Input file.
        path: PathBuf,
        /// Validate only; write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// List orders.
    List {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum OutputFormat {
    Text,
    Json,
}
```

Doc comments become `--help` text. The `env = "…"` attribute needs clap's `env` feature, and
`--help` then shows `[env: SHOP_CONFIG=]` automatically. Never use `structopt` (merged into clap 3)
or clap's builder API for new code unless arguments are generated at runtime.

For tools that are also libraries, put the logic in `lib.rs` and keep the clap types in the binary
so the library does not depend on clap.

## Errors and exit codes

### CLI-02: `main` returns `ExitCode`; `run` returns `anyhow::Result`

```rust,ignore
fn main() -> ExitCode {
    let cli = Cli::parse(); // bad args: clap prints usage to stderr and exits with code 2
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if is_broken_pipe(&err) => ExitCode::SUCCESS, // `shop list | head` (CLI-05)
        Err(err) => {
            eprintln!("error: {err:#}"); // one line with the whole context chain
            let code = err.downcast_ref::<CliError>().map_or(1, CliError::exit_code);
            ExitCode::from(code)
        }
    }
}
```

`{err:#}` prints `error: reading orders.csv: No such file or directory (os error 2)`. For a very
small tool, `fn main() -> anyhow::Result<()>` is acceptable (it prints the Debug form with a
`Caused by:` list and exits 1), but you lose control of exit codes and formatting.

Never `unwrap()`/`expect()` on user input, files, network or env vars — a panic message with a
source location is not an error message. Never call `std::process::exit` deep in the code: it
skips destructors (unflushed buffers, temp-file cleanup). Return an error and let `main` decide.

### CLI-03: Document exit codes; give distinct codes only to actionable failures

| Code | Meaning |
|---|---|
| 0 | Success (including "nothing to do") |
| 1 | Generic failure |
| 2 | Usage error — clap does this for you |
| other | Only when scripts need to branch on it; document in `--help`/README |

Model special codes as a typed error and downcast in `main`:

```rust,ignore
#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("input file not found: {0}")]
    InputMissing(std::path::PathBuf),
}

impl CliError {
    fn exit_code(&self) -> u8 {
        match self {
            Self::InputMissing(_) => 66, // EX_NOINPUT from sysexits.h
        }
    }
}
```

## Output

### CLI-04: stdout is for data; stderr is for everything else

- **stdout**: the result the user asked for — the thing that gets piped into `jq`, `grep`, a file.
- **stderr**: errors, warnings, progress bars, logs, "Processing 10 files…" chatter, prompts.
- Offer `--format json` (or `--json`) for machine-readable output; keep the JSON schema stable.
- Be quiet by default; `-v` adds detail, `-q` removes non-essential stderr output.

Configure tracing to write to **stderr**: `tracing_subscriber::fmt` writes to stdout by default.

```rust,ignore
let level = match cli.global.verbose {
    0 => "warn",
    1 => "info",
    2 => "debug",
    _ => "trace",
};
tracing_subscriber::fmt()
    .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level)))
    .with_writer(std::io::stderr)
    .init();
```

### CLI-05: Write through a locked, buffered stdout and handle broken pipes

Rust ignores `SIGPIPE`, so when the reader goes away (`tool | head -1`) writes return
`ErrorKind::BrokenPipe` — and `println!` **panics** on that error. For data output, lock stdout
once, buffer it, use `writeln!` and propagate errors; treat a broken pipe as success in `main`.

```rust,ignore
let mut out = BufWriter::new(io::stdout().lock());
for item in items {
    writeln!(out, "{item}")?;
}
out.flush()?;

fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain()
        .filter_map(|e| e.downcast_ref::<io::Error>())
        .any(|e| e.kind() == io::ErrorKind::BrokenPipe)
}
```

Unbuffered `println!` in a loop also re-locks and flushes per line: slow for large outputs. The
recommended `clippy::print_stdout` lint (TPL-01) steers code to this pattern; a tool that really
wants `println!` can `#![expect(clippy::print_stdout, reason = "…")]` in `main.rs`.

### CLI-06: Config precedence: flags > env vars > config file > defaults

clap already merges flags and env vars (`#[arg(long, env = "…")]`). Merge the file and defaults
yourself, field by field, into one resolved `Settings`:

```rust,ignore
/// Every field optional: the file only overrides defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    api_url: Option<String>,
    parallelism: Option<usize>,
}

pub(crate) fn resolve(o: Overrides) -> anyhow::Result<Settings> {
    let file = match (&o.config, default_config_path()) {
        (Some(explicit), _) => load(explicit)?, // user asked for it: must exist
        (None, Some(default)) if default.exists() => load(&default)?,
        (None, _) => FileConfig::default(), // no file is fine
    };
    Ok(Settings {
        api_url: o.api_url.or(file.api_url).unwrap_or_else(|| "https://api.example.com".into()),
        parallelism: o.parallelism.or(file.parallelism).unwrap_or(4),
    })
}

/// `~/.config/shop/config.toml` on Linux, the platform equivalent on macOS/Windows.
fn default_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "example", "shop")
        .map(|d| d.config_dir().join("config.toml"))
}
```

An explicitly passed `--config` that does not exist is an error; a missing default file is not.
Never hard-code `~/.myapp` or `%APPDATA%` paths. Services use the `config` crate instead
(`configuration.md`).

### CLI-07: Progress and colour only when a human is watching

- Progress: `indicatif::ProgressBar` draws to stderr and hides itself automatically when stderr is
  not a terminal (or `TERM=dumb`) — so piped/CI output stays clean. Call `finish_and_clear()` (or
  `finish_with_message`) when done.
- Colour: clap colours its own help. For your output, check `std::io::IsTerminal` on the stream
  you write to and honour `NO_COLOR`; offer `--color auto|always|never` for tools used in CI.
- Never print progress to stdout, and never use `\r` animations when not attached to a terminal.

## Testing and distribution

### CLI-08: Test the parser, the logic, and the binary separately

```rust,ignore
#[test]
fn cli_definition_is_valid() {
    use clap::CommandFactory;
    Cli::command().debug_assert(); // catches conflicting flags/short options at test time
}
```

- Logic: unit tests on `run`-level functions in the library, with in-memory inputs.
- End-to-end: `assert_cmd` runs the built binary and asserts on stdout/stderr/exit code
  (`Command::cargo_bin("shop")?.arg("list").assert().success().stdout(predicates::str::contains("A-1"))`),
  or `trycmd` for many cases as snapshot files.

### CLI-09: Ship binaries with cargo-dist; keep `cargo install` working

For end users, generate release builds and installers with `cargo-dist` (`dist init`), which writes
the GitHub release workflow for you (`ci-cd-release.md` CI-09). Keep the package `cargo install`-able
(`publish = true`, no path-only dependencies) if you publish to crates.io.

## Checklist

### CLI-10: CLI readiness checklist

- [ ] clap derive types in `cli.rs`; `Cli::command().debug_assert()` test.
- [ ] `main` → `ExitCode`; `run` → `anyhow::Result`; `error: {err:#}` on stderr.
- [ ] Documented exit codes; typed error for any code other than 0/1/2.
- [ ] Data on stdout (buffered, `writeln!`, broken pipe = success); everything else on stderr.
- [ ] `--format json` for scriptable output.
- [ ] Flags > env > file > defaults; platform config dirs via `directories`.
- [ ] Tracing subscriber writes to stderr; `-v/-q` map to levels.
- [ ] Progress bars only on a TTY; `NO_COLOR` honoured.
- [ ] No tokio unless the tool does concurrent network IO.
