//! Clippy lint documentation, scraped from the published lint list.

use std::fmt::Write as _;
use std::time::Duration;

use rustkb_core::{Doc, Source};
use scraper::{ElementRef, Html, Node, Selector};

use crate::{Http, Result};

pub const URL: &str = "https://rust-lang.github.io/rust-clippy/stable/index.html";

pub fn fetch(http: &Http, ttl: Duration) -> Result<Vec<Doc>> {
    let path = http.fetch_cached(URL, "clippy/stable.html", ttl)?;
    parse(&std::fs::read_to_string(path)?)
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector is valid")
}

pub fn parse(html: &str) -> Result<Vec<Doc>> {
    let doc = Html::parse_document(html);
    let article = sel("article[id]");
    let group = sel(".lint-group");
    let level = sel(".lint-level");
    let body = sel(".lint-doc-md");
    let version = sel(".label-version");
    let applicability = sel(".applicability");

    let text_of = |el: &ElementRef<'_>, s: &Selector| {
        el.select(s)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_owned())
            .unwrap_or_default()
    };

    let mut out = Vec::new();
    for a in doc.select(&article) {
        let Some(name) = a.value().attr("id") else {
            continue;
        };
        let group = text_of(&a, &group);
        let level = text_of(&a, &level);
        let since = text_of(&a, &version);
        let applicability = text_of(&a, &applicability);
        let md = a
            .select(&body)
            .next()
            .map(|e| html_to_markdown(&e))
            .unwrap_or_default();
        let summary = md
            .split("### What it does")
            .nth(1)
            .and_then(|s| s.split("\n### ").next())
            .map(|s| s.trim().replace('\n', " "))
            .unwrap_or_default();

        let header = format!(
            "`clippy::{name}` — group: **{group}**, default level: **{level}**, added in {since}, applicability: {applicability}\n\n\
             Enable in Cargo.toml: `[lints.clippy] {name} = \"warn\"`\n\n"
        );
        let mut d = Doc::new(
            format!("clippy:{name}"),
            Source::Clippy,
            format!("clippy::{name}"),
            header + &md,
        );
        d.path = Some(name.to_owned());
        d.kind = Some(group.clone());
        d.version = (!since.is_empty()).then_some(since);
        d.tags = vec![group, level, "lint".into(), "clippy".into()];
        d.summary = summary;
        d.url = Some(format!("{URL}#{name}"));
        out.push(d);
    }
    anyhow::ensure!(
        !out.is_empty(),
        "no lints found — clippy page layout changed?"
    );
    Ok(out)
}

/// Minimal HTML → Markdown for rustdoc/clippy-style content.
pub fn html_to_markdown(el: &ElementRef<'_>) -> String {
    let mut out = String::new();
    walk(el, &mut out, 0);
    // Collapse 3+ newlines.
    let mut cleaned = String::with_capacity(out.len());
    let mut newlines = 0;
    for c in out.chars() {
        if c == '\n' {
            newlines += 1;
            if newlines > 2 {
                continue;
            }
        } else {
            newlines = 0;
        }
        cleaned.push(c);
    }
    cleaned.trim().to_owned()
}

fn walk(el: &ElementRef<'_>, out: &mut String, list_depth: usize) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) => {
                let Some(child_el) = ElementRef::wrap(child) else {
                    continue;
                };
                match e.name() {
                    "h1" | "h2" | "h3" | "h4" | "h5" => {
                        let level = e.name()[1..].parse::<usize>().unwrap_or(3);
                        let _ = write!(out, "\n\n{} ", "#".repeat(level));
                        walk(&child_el, out, list_depth);
                        out.push_str("\n\n");
                    }
                    "p" => {
                        out.push_str("\n\n");
                        walk(&child_el, out, list_depth);
                        out.push_str("\n\n");
                    }
                    "pre" => {
                        let lang = child_el
                            .select(&sel("code"))
                            .next()
                            .and_then(|c| c.value().attr("class"))
                            .and_then(|c| {
                                c.split_whitespace()
                                    .find_map(|k| k.strip_prefix("language-"))
                            })
                            .unwrap_or("rust")
                            .to_owned();
                        let code: String = child_el.text().collect();
                        let _ = write!(out, "\n\n```{lang}\n{}\n```\n\n", code.trim_end());
                    }
                    "code" => {
                        out.push('`');
                        out.push_str(&child_el.text().collect::<String>());
                        out.push('`');
                    }
                    "li" => {
                        out.push('\n');
                        out.push_str(&"  ".repeat(list_depth.saturating_sub(1)));
                        out.push_str("- ");
                        walk(&child_el, out, list_depth);
                    }
                    "ul" | "ol" => {
                        walk(&child_el, out, list_depth + 1);
                        out.push('\n');
                    }
                    "br" => out.push('\n'),
                    "strong" | "b" => {
                        out.push_str("**");
                        walk(&child_el, out, list_depth);
                        out.push_str("**");
                    }
                    "em" | "i" => {
                        out.push('*');
                        walk(&child_el, out, list_depth);
                        out.push('*');
                    }
                    "script" | "style" | "input" | "label" => {}
                    _ => walk(&child_el, out, list_depth),
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_article() {
        let html = r#"<html><body><article id="unwrap_used"><h2><span class="label lint-group group-restriction">restriction</span> <span class="label lint-level level-allow">allow</span></h2>
<div class="lint-docs"><div class="lint-doc-md"><h3>What it does</h3><p>Checks for <code>.unwrap()</code> calls.</p><h3>Example</h3><pre><code class="language-rust">x.unwrap();</code></pre></div>
<div class="lint-additional-info">Added in: <span class="label label-version">1.45.0</span></div></div></article></body></html>"#;
        let docs = parse(html).expect("parses");
        assert_eq!(docs.len(), 1);
        let d = &docs[0];
        assert_eq!(d.id, "clippy:unwrap_used");
        assert_eq!(d.kind.as_deref(), Some("restriction"));
        assert_eq!(d.version.as_deref(), Some("1.45.0"));
        assert_eq!(d.summary, "Checks for `.unwrap()` calls.");
        assert!(d.body.contains("```rust\nx.unwrap();\n```"));
    }
}
