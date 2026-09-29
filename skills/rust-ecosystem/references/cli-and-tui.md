---
id: ecosystem/cli-and-tui
title: CLI, terminal UX and TUI crates
summary: >-
  clap (derive) for argument parsing, prompts/progress/colour crates that respect TTYs,
  platform directories, and ratatui + crossterm for full-screen terminal UIs.
area: ecosystem
tags: [cli, clap, arguments, terminal, color, progress, prompts, tui, ratatui, crossterm, directories]
rust: "1.96"
edition: "2024"
crates:
  clap: "4.6"
  clap_complete: "4.6"
  clap_mangen: "0.3"
  anyhow: "1.0"
  indicatif: "0.18"
  dialoguer: "0.12"
  inquire: "0.9"
  owo-colors: "4.4"
  anstream: "1.0"
  directories: "6.0"
  dirs: "7.0"
  ratatui: "0.30"
  crossterm: "0.29"
  comfy-table: "8.0"
  human-panic: "2.0"
  lexopt: "0.3"
  argh: "0.1"
  color-eyre: "0.6"
  miette: "7.6"
  assert_cmd: "2.2"
  cursive: "0.21"
  rustyline: "18.0"
  reedline: "0.52"
  tabled: "0.22"
  trycmd: "1.2"
  snapbox: "1.2"
  predicates: "3.1"
  tempfile: "3.27"
  ratatui-core: "0.1"
verified: 2026-09-29
sources:
  - https://docs.rs/clap/latest/clap/_derive/index.html
  - https://ratatui.rs/
  - https://rustsec.org/advisories/RUSTSEC-2022-0104.html
  - https://rustsec.org/advisories/RUSTSEC-2023-0049.html
---

# CLI, terminal UX and TUI crates

CLI application structure (exit codes, config precedence, stdout vs stderr contracts) is in
`rust-architecture`. This file picks crates.

## CLI-01: Default argument parser is clap with derive

Default: `clap` (4.6) with `features = ["derive"]` (add `"env"` for env-var fallbacks).
`structopt` was merged into clap 3 and is in maintenance mode (RUSTSEC-2022-0104) — never add it.
clap 2.x/3.x builder code from memory (`App::new`, `Arg::with_name`) doesn't compile on 4.x.

```toml
clap = { version = "4.6", features = ["derive", "env"] }
anyhow = "1.0"
```

```rust
use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};

/// Sync files to a remote bucket.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Increase log verbosity (-v, -vv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// API token (falls back to the environment)
    #[arg(long, env = "MYTOOL_TOKEN", hide_env_values = true)]
    token: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Upload a directory
    Push {
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = Mode::Incremental)]
        mode: Mode,
    },
    /// Show status
    Status,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Mode {
    Full,
    Incremental,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Push { path, mode } => {
            let meta = std::fs::metadata(&path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            println!("pushing {} bytes ({mode:?})", meta.len());
        }
        Command::Status => println!("ok"),
    }
    Ok(())
}
```

- Doc comments become help text; `#[command(version, about)]` pulls from Cargo.toml.
- Use `PathBuf` (not `String`) for paths, `ValueEnum` for fixed choices, `ArgAction::Count`
  for `-vvv`.
- Shell completions: `clap_complete` (4.6); man pages: `clap_mangen`
  (0.3) — generate them in `build.rs` or an `xtask`.
- Alternatives only for binary-size/compile-time constrained tools: `lexopt`
  (0.3, minimal, hand-rolled loop) or `argh` (0.1). `gumdrop` is unmaintained
  (RUSTSEC, 2026-07).

## CLI-02: Errors and exit behaviour in CLIs

Default: `fn main() -> anyhow::Result<()>` with `.context(...)` on fallible steps. Returning
`Err` from `main` prints the error chain and exits with code 1.

- Prettier reports with span traces: `color-eyre` (0.6).
- Diagnostics pointing into user input files (config validators, compilers): `miette`
  (7.6).
- End-user tools: `human-panic` (2.0) turns panics into a friendly message and a
  crash-report file.
- Specific exit codes: return `std::process::ExitCode` from `main` (after printing the error
  yourself) rather than calling `std::process::exit` deep in the code (skips destructors).

## CLI-03: Colour and TTY detection

Default: detect TTYs with `std::io::IsTerminal` (stable since 1.70). `atty` is unmaintained
and unsound (RUSTSEC-2024-0375); `is-terminal` is just a polyfill for the std trait.

- Colouring: `owo-colors` (4.4) — zero-allocation, no global state; use its
  `if_supports_color` (feature `supports-colors`) to respect `NO_COLOR`/pipes.
- Auto-stripping ANSI when output isn't a terminal: `anstream` (1.0) (what clap uses).
- `ansi_term` is unmaintained (RUSTSEC-2021-0139); `colored` works but relies on global
  state — prefer the two above for new code.
- Never emit colour codes when stdout is not a terminal or `NO_COLOR` is set.

## CLI-04: Progress, prompts and tables

| Need | Crate |
|---|---|
| Progress bars / spinners | `indicatif` (0.18) — hide when not a TTY (it does by default on non-terminals) |
| Confirm / select / password prompts | `dialoguer` (0.12) or `inquire` (0.9) |
| Line editing / REPL history | `rustyline` or `reedline` |
| Tables in terminal output | `comfy-table` (8.0) or `tabled` |

Every interactive prompt needs a non-interactive path (`--yes`, flags, env vars) so the tool
works in CI and scripts. Write progress to stderr; keep stdout for data.

## CLI-05: Where to store config, cache and data

Default: `directories` (6.0) `ProjectDirs::from("com", "Org", "app")` for
per-app config/cache/data paths that follow XDG on Linux, Known Folders on Windows and
Apple conventions on macOS. `directories` and its lower-level sibling `dirs` (7.0) are
now developed on Codeberg; the old GitHub repository of `directories` is archived, which
does not mean the crate is abandoned.

- Just the home directory: `std::env::home_dir()` (fixed in 1.85, un-deprecated in 1.87).
- Never hard-code `~/.myapp` or `%APPDATA%` paths.
- Temporary files: `tempfile` (secure creation, auto-cleanup).

## CLI-06: Full-screen TUIs: ratatui + crossterm

Default: `ratatui` (0.30) with its default `crossterm` backend. `tui` (tui-rs) is
unmaintained (RUSTSEC-2023-0049); ratatui is its community continuation.

```rust
use ratatui::crossterm::event::{self, Event, KeyCode};
use ratatui::{
    DefaultTerminal, Frame,
    widgets::{Block, Paragraph},
};

fn main() -> std::io::Result<()> {
    // Sets up raw mode + alternate screen, runs the closure, restores the terminal (also on panic)
    ratatui::run(app)
}

fn app(terminal: &mut DefaultTerminal) -> std::io::Result<()> {
    loop {
        terminal.draw(render)?;
        if let Event::Key(key) = event::read()?
            && key.code == KeyCode::Char('q')
        {
            return Ok(());
        }
    }
}

fn render(frame: &mut Frame) {
    let text = Paragraph::new("Press q to quit").block(Block::bordered().title("demo"));
    frame.render_widget(text, frame.area());
}
```

- Use crossterm through `ratatui::crossterm` so both use the same crossterm version; adding
  a separate `crossterm` dependency with a different major produces confusing type mismatches.
  Add `crossterm` (0.29) directly only for non-TUI terminal control.
- ratatui 0.30 split into `ratatui-core`, `ratatui-widgets` and backend crates; widget
  libraries should depend on `ratatui-core`, apps on `ratatui`.
- For an async app, read events with crossterm's `EventStream` (feature `event-stream`) in a
  `tokio::select!` loop.
- `cursive` is an alternative with a retained, callback-driven model; pick ratatui unless you
  specifically want that model.

## CLI-07: Testing CLIs

Default: `assert_cmd` (2.2) + `predicates` for black-box tests of the binary, and
`insta` (`assert_snapshot!` of stdout) for large outputs; `trycmd`/`snapbox` for
Markdown-described CLI test cases. See `testing-and-benchmarking.md`.
