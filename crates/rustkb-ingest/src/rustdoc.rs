//! Versioned API docs from docs.rs rustdoc JSON.
//!
//! rustdoc's JSON format is unstable (`format_version` bumps often), so this parser is
//! deliberately loose: it reads `serde_json::Value` and only relies on long-stable
//! shapes. Items are discovered by walking the module tree from the crate root, which
//! yields the paths users actually import (re-exports included).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use anyhow::Context;
use rustkb_core::{Doc, Source};
use serde_json::Value;

use crate::{Http, Result};

/// Download (or reuse) rustdoc JSON for `krate@version` (`latest` allowed) and convert it.
pub fn fetch(http: &Http, krate: &str, version: &str) -> Result<Vec<Doc>> {
    let url = format!("https://docs.rs/crate/{krate}/{version}/json.gz");
    // Pinned versions never change; `latest` is re-checked daily.
    let ttl = if version == "latest" {
        crate::http::DAY
    } else {
        std::time::Duration::from_secs(u64::MAX / 4)
    };
    let path = http
        .fetch_cached(&url, &format!("rustdoc/{krate}-{version}.json.gz"), ttl)
        .with_context(|| format!("no rustdoc JSON on docs.rs for {krate}@{version} (docs.rs only has JSON for builds since 2025)"))?;
    let bytes = crate::http::maybe_gunzip(std::fs::read(path)?)?;
    let json: Value = serde_json::from_slice(&bytes).context("parsing rustdoc JSON")?;
    convert(krate, &json)
}

/// A module reachable via several re-export paths is documented under at most this many.
const MAX_MODULE_WALKS: u32 = 2;
/// Hard cap on items per crate; the largest real crates we track stay well below it.
const MAX_ITEMS: usize = 60_000;

struct Ctx<'a> {
    index: &'a serde_json::Map<String, Value>,
    krate: String,
    version: String,
    docs: Vec<Doc>,
    seen_paths: HashSet<String>,
    /// Type id → rendered trait names it implements (non-blanket, non-synthetic).
    trait_impls: HashMap<String, Vec<String>>,
    /// docs.rs URL of the item whose members are being emitted.
    parent_url: Option<String>,
    /// Module ids on the current walk path (re-export cycle detection).
    module_stack: Vec<String>,
    /// How many times each module id has been walked (canonical path + re-exports).
    module_walks: HashMap<String, u32>,
}

pub fn convert(krate: &str, json: &Value) -> Result<Vec<Doc>> {
    let index = json
        .get("index")
        .and_then(Value::as_object)
        .context("rustdoc JSON has no `index`")?;
    let root = id_str(json.get("root").context("rustdoc JSON has no `root`")?);
    let version = json
        .get("crate_version")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let crate_name = index
        .get(&root)
        .and_then(|r| r.get("name"))
        .and_then(Value::as_str)
        .unwrap_or(krate)
        .to_owned();

    let mut ctx = Ctx {
        index,
        krate: crate_name.clone(),
        version: version.clone(),
        docs: Vec::new(),
        seen_paths: HashSet::new(),
        trait_impls: HashMap::new(),
        parent_url: None,
        module_stack: Vec::new(),
        module_walks: HashMap::new(),
    };
    ctx.collect_trait_impls();
    ctx.walk_module(&root, &crate_name, 0);
    if ctx.docs.len() >= MAX_ITEMS {
        tracing::warn!(krate = %crate_name, "rustdoc item cap reached; output truncated");
    }
    tracing::info!(krate = %crate_name, %version, items = ctx.docs.len(), "converted rustdoc");
    Ok(ctx.docs)
}

fn id_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn inner_kind(item: &Value) -> Option<(&str, &Value)> {
    let inner = item.get("inner")?;
    match inner {
        Value::Object(map) => map.iter().next().map(|(k, v)| (k.as_str(), v)),
        Value::String(s) => Some((s.as_str(), &Value::Null)),
        _ => None,
    }
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

impl Ctx<'_> {
    fn item(&self, id: &str) -> Option<&Value> {
        self.index.get(id)
    }

    fn collect_trait_impls(&mut self) {
        for item in self.index.values() {
            let Some(("impl", imp)) = inner_kind(item) else {
                continue;
            };
            if imp.get("is_synthetic").and_then(Value::as_bool) == Some(true)
                || imp.get("blanket_impl").is_some_and(|b| !b.is_null())
            {
                continue;
            }
            let Some(tr) = imp.get("trait").filter(|t| !t.is_null()) else {
                continue;
            };
            let Some(for_id) = imp
                .get("for")
                .and_then(|f| f.get("resolved_path"))
                .and_then(|p| p.get("id"))
            else {
                continue;
            };
            let name = path_name(tr);
            let neg = if imp.get("is_negative").and_then(Value::as_bool) == Some(true) {
                "!"
            } else {
                ""
            };
            self.trait_impls
                .entry(id_str(for_id))
                .or_default()
                .push(format!("{neg}{name}"));
        }
    }

    fn walk_module(&mut self, id: &str, path: &str, depth: usize) {
        // Re-exports can form cycles (`mod a { pub use super::*; }`) or fan out
        // exponentially; each module is walked on at most two paths and never re-entered.
        let walks = self.module_walks.entry(id.to_owned()).or_insert(0);
        if depth > 12
            || *walks >= MAX_MODULE_WALKS
            || self.docs.len() >= MAX_ITEMS
            || self.module_stack.iter().any(|m| m == id)
            || !self.seen_paths.insert(format!("mod:{path}"))
        {
            return;
        }
        *walks += 1;
        self.module_stack.push(id.to_owned());
        self.walk_module_items(id, path, depth);
        self.module_stack.pop();
    }

    fn walk_module_items(&mut self, id: &str, path: &str, depth: usize) {
        let Some(item) = self.item(id).cloned() else {
            return;
        };
        self.emit(&item, id, path, "module");
        let Some(("module", module)) = inner_kind(&item) else {
            return;
        };
        for child in arr(module, "items") {
            let child_id = id_str(child);
            let Some(child_item) = self.item(&child_id).cloned() else {
                continue;
            };
            self.visit(&child_id, &child_item, path, depth);
        }
    }

    fn visit(&mut self, id: &str, item: &Value, parent: &str, depth: usize) {
        if !is_public(item) {
            return;
        }
        let Some((kind, inner)) = inner_kind(item) else {
            return;
        };
        if kind == "use" {
            self.visit_use(inner, parent, depth);
            return;
        }
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            return;
        };
        let path = format!("{parent}::{name}");
        if kind == "module" {
            self.walk_module(id, &path, depth + 1);
        } else {
            self.emit(item, id, &path, kind);
        }
    }

    /// `pub use` re-exports: document the target under the re-exported path.
    fn visit_use(&mut self, use_: &Value, parent: &str, depth: usize) {
        let Some(target) = use_.get("id").filter(|v| !v.is_null()).map(id_str) else {
            return;
        };
        let Some(target_item) = self.item(&target).cloned() else {
            return;
        };
        let is_glob = use_.get("is_glob").and_then(Value::as_bool) == Some(true);
        if is_glob {
            let walks = self.module_walks.entry(target.clone()).or_insert(0);
            if depth > 12 || *walks >= MAX_MODULE_WALKS || self.module_stack.contains(&target) {
                return;
            }
            *walks += 1;
            if let Some(("module", module)) = inner_kind(&target_item) {
                self.module_stack.push(target.clone());
                for child in arr(module, "items") {
                    let child_id = id_str(child);
                    if let Some(child_item) = self.item(&child_id).cloned() {
                        self.visit(&child_id, &child_item, parent, depth + 1);
                    }
                }
                self.module_stack.pop();
            }
            return;
        }
        let name = use_
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| target_item.get("name").and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned();
        let path = format!("{parent}::{name}");
        match inner_kind(&target_item) {
            Some(("module", _)) => self.walk_module(&target, &path, depth + 1),
            Some((kind, _)) => {
                let kind = kind.to_owned();
                self.emit(&target_item, &target, &path, &kind);
            }
            None => {}
        }
    }

    fn emit(&mut self, item: &Value, id: &str, path: &str, kind: &str) {
        if self.docs.len() >= MAX_ITEMS || !self.seen_paths.insert(path.to_owned()) {
            return;
        }
        let name = path.rsplit("::").next().unwrap_or(path);
        let parent = path.rsplit_once("::").map(|(p, _)| p.to_owned());
        let inner = inner_kind(item).map_or(&Value::Null, |(_, v)| v);
        let docs = item.get("docs").and_then(Value::as_str).unwrap_or_default();
        let signature = Self::signature(kind, name, item, inner)
            .replace("crate::", &format!("{}::", self.krate));

        let mut body = String::new();
        if !signature.is_empty() {
            let _ = write!(body, "```rust\n{signature}\n```\n\n");
        }
        if let Some(dep) = item.get("deprecation").filter(|d| !d.is_null()) {
            let note = dep.get("note").and_then(Value::as_str).unwrap_or("");
            let since = dep.get("since").and_then(Value::as_str).unwrap_or("?");
            let _ = write!(body, "**Deprecated** since {since}: {note}\n\n");
        }
        body.push_str(docs);

        let (kind_label, id_prefix) = match kind {
            "module" if parent.is_none() => ("crate", "crate"),
            k => (k, k),
        };
        let url = match (kind, &self.parent_url) {
            ("method" | "trait_method" | "assoc_type" | "assoc_const", Some(parent_url)) => {
                let anchor = match kind {
                    "method" => "method",
                    "trait_method" => "tymethod",
                    "assoc_type" => "associatedtype",
                    _ => "associatedconstant",
                };
                format!(
                    "{}#{anchor}.{name}",
                    parent_url.split('#').next().unwrap_or(parent_url)
                )
            }
            _ => docs_rs_url(&self.krate, &self.version, path, kind_label),
        };
        let saved = self.parent_url.replace(url.clone());
        body.push_str(&self.members_section(kind, inner, path, id));
        self.parent_url = saved;

        let mut d = Doc::new(
            format!("rustdoc:{}@{}:{id_prefix}:{path}", self.krate, self.version),
            Source::Rustdoc,
            name.to_owned(),
            body,
        );
        d.path = Some(path.to_owned());
        d.parent = parent;
        d.krate = Some(self.krate.clone());
        d.version = Some(self.version.clone());
        d.kind = Some(kind_label.to_owned());
        d.summary = first_sentence(docs);
        if item.get("deprecation").is_some_and(|d| !d.is_null()) {
            d.tags.push("deprecated".into());
        }
        d.url = Some(url);
        self.docs.push(d);
    }

    /// Methods of inherent impls / trait items become child docs; fields and variants are
    /// summarised inline in the parent body.
    fn members_section(&mut self, kind: &str, inner: &Value, path: &str, id: &str) -> String {
        let mut out = String::new();
        match kind {
            "struct" => {
                let fields = inner
                    .get("kind")
                    .and_then(|k| k.get("plain"))
                    .map(|p| arr(p, "fields").to_vec());
                let tuple = inner
                    .get("kind")
                    .and_then(|k| k.get("tuple"))
                    .and_then(Value::as_array)
                    .cloned();
                let lines: Vec<String> = fields
                    .unwrap_or_default()
                    .iter()
                    .chain(tuple.unwrap_or_default().iter())
                    .filter_map(|f| self.item(&id_str(f)).cloned())
                    .filter(is_public)
                    .map(|f| {
                        let n = f
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("_")
                            .to_owned();
                        let ty = inner_kind(&f)
                            .map(|(_, t)| render_type(t))
                            .unwrap_or_default();
                        format!("- `{n}: {ty}`")
                    })
                    .collect();
                if !lines.is_empty() {
                    let _ = write!(out, "\n\n## Fields\n{}", lines.join("\n"));
                }
                self.impl_methods(arr(inner, "impls").to_vec(), path, &mut out);
            }
            "enum" => {
                let lines: Vec<String> = arr(inner, "variants")
                    .iter()
                    .filter_map(|v| self.item(&id_str(v)).cloned())
                    .map(|v| {
                        let n = v
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("_")
                            .to_owned();
                        let doc = first_sentence(
                            v.get("docs").and_then(Value::as_str).unwrap_or_default(),
                        );
                        if doc.is_empty() {
                            format!("- `{n}`")
                        } else {
                            format!("- `{n}` — {doc}")
                        }
                    })
                    .collect();
                if !lines.is_empty() {
                    let _ = write!(out, "\n\n## Variants\n{}", lines.join("\n"));
                }
                self.impl_methods(arr(inner, "impls").to_vec(), path, &mut out);
            }
            "union" => self.impl_methods(arr(inner, "impls").to_vec(), path, &mut out),
            "trait" => {
                let mut names = Vec::new();
                for m in arr(inner, "items").to_vec() {
                    let mid = id_str(&m);
                    let Some(mi) = self.item(&mid).cloned() else {
                        continue;
                    };
                    let Some((mk, _)) = inner_kind(&mi) else {
                        continue;
                    };
                    let Some(n) = mi.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    names.push(format!("`{n}`"));
                    let mk = match mk {
                        "function" => "trait_method",
                        "assoc_type" => "assoc_type",
                        "assoc_const" => "assoc_const",
                        other => other,
                    }
                    .to_owned();
                    self.emit(&mi, &mid, &format!("{path}::{n}"), &mk);
                }
                if !names.is_empty() {
                    let _ = write!(out, "\n\n## Trait items\n{}", names.join(", "));
                }
                let implementors = arr(inner, "implementations").len();
                if implementors > 0 {
                    let _ = write!(
                        out,
                        "\n\nImplemented by {implementors} type(s) in this crate."
                    );
                }
            }
            _ => {}
        }
        if matches!(kind, "struct" | "enum" | "union")
            && let Some(traits) = self.trait_impls.get(id)
        {
            let mut traits = traits.clone();
            traits.sort();
            traits.dedup();
            let _ = write!(out, "\n\n## Implements\n{}", traits.join(", "));
        }
        out
    }

    fn impl_methods(&mut self, impls: Vec<Value>, type_path: &str, out: &mut String) {
        let mut methods = Vec::new();
        for imp_id in impls {
            let Some(imp_item) = self.item(&id_str(&imp_id)).cloned() else {
                continue;
            };
            let Some(("impl", imp)) = inner_kind(&imp_item) else {
                continue;
            };
            if imp.get("trait").is_some_and(|t| !t.is_null()) {
                continue; // trait impls are summarised under "Implements"
            }
            for m in arr(imp, "items").to_vec() {
                let mid = id_str(&m);
                let Some(mi) = self.item(&mid).cloned() else {
                    continue;
                };
                if !is_public(&mi) {
                    continue;
                }
                let Some((mk, _)) = inner_kind(&mi) else {
                    continue;
                };
                let Some(n) = mi.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let kind = if mk == "function" { "method" } else { mk }.to_owned();
                methods.push(format!("`{n}`"));
                self.emit(&mi, &mid, &format!("{type_path}::{n}"), &kind);
            }
        }
        if !methods.is_empty() {
            let _ = write!(out, "\n\n## Methods\n{}", methods.join(", "));
        }
    }

    fn signature(kind: &str, name: &str, item: &Value, inner: &Value) -> String {
        let vis = if matches!(kind, "trait_method" | "assoc_type" | "assoc_const") {
            ""
        } else {
            "pub "
        };
        match kind {
            "function" | "method" | "trait_method" => render_fn(vis, name, inner),
            "struct" => format!(
                "{vis}struct {name}{}",
                render_generics(inner.get("generics"))
            ),
            "enum" => format!("{vis}enum {name}{}", render_generics(inner.get("generics"))),
            "union" => format!(
                "{vis}union {name}{}",
                render_generics(inner.get("generics"))
            ),
            "trait" => {
                let unsafety = if inner.get("is_unsafe").and_then(Value::as_bool) == Some(true) {
                    "unsafe "
                } else {
                    ""
                };
                let bounds = render_bounds(arr(inner, "bounds"));
                let bounds = if bounds.is_empty() {
                    String::new()
                } else {
                    format!(": {bounds}")
                };
                format!(
                    "{vis}{unsafety}trait {name}{}{bounds}",
                    render_generics(inner.get("generics"))
                )
            }
            "type_alias" => format!(
                "{vis}type {name}{} = {};",
                render_generics(inner.get("generics")),
                inner.get("type").map(render_type).unwrap_or_default()
            ),
            "constant" => format!(
                "{vis}const {name}: {};",
                inner.get("type").map(render_type).unwrap_or_default()
            ),
            "static" => format!(
                "{vis}static {}{name}: {};",
                if inner.get("is_mutable").and_then(Value::as_bool) == Some(true) {
                    "mut "
                } else {
                    ""
                },
                inner.get("type").map(render_type).unwrap_or_default()
            ),
            "assoc_type" => format!("type {name};"),
            "assoc_const" => format!(
                "const {name}: {};",
                inner.get("type").map(render_type).unwrap_or_default()
            ),
            "macro" => inner
                .as_str()
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned(),
            "proc_macro" => {
                let k = inner.get("kind").and_then(Value::as_str).unwrap_or("bang");
                match k {
                    "derive" => format!("#[derive({name})]"),
                    "attr" => format!("#[{name}]"),
                    _ => format!("{name}!(…)"),
                }
            }
            _ => {
                let _ = item;
                String::new()
            }
        }
    }
}

fn is_public(item: &Value) -> bool {
    match item.get("visibility") {
        Some(Value::String(s)) => s == "public" || s == "default",
        _ => false,
    }
}

fn first_sentence(docs: &str) -> String {
    let para = docs
        .split("\n\n")
        .next()
        .unwrap_or_default()
        .replace('\n', " ");
    let end = para.find(". ").map_or(para.len(), |i| i + 1);
    para[..end].trim().chars().take(300).collect()
}

fn docs_rs_url(krate: &str, version: &str, path: &str, kind: &str) -> String {
    let segs: Vec<&str> = path.split("::").collect();
    let base = format!("https://docs.rs/{krate}/{version}");
    let (Some((last, dirs)), true) = (segs.split_last(), segs.len() > 1) else {
        return format!("{base}/{}/", segs.first().copied().unwrap_or(krate));
    };
    let dir = dirs.join("/");
    let prefix = match kind {
        "module" => return format!("{base}/{dir}/{last}/index.html"),
        "struct" => "struct",
        "enum" => "enum",
        "union" => "union",
        "trait" => "trait",
        "function" => "fn",
        "macro" | "proc_macro" => "macro",
        "type_alias" => "type",
        "constant" => "constant",
        "static" => "static",
        "method" | "trait_method" | "assoc_type" | "assoc_const" => {
            // Link to the parent type page with an anchor.
            let parent_dir = dirs
                .split_last()
                .map(|(_, d)| d.join("/"))
                .unwrap_or_default();
            let parent = dirs.last().copied().unwrap_or_default();
            let anchor = match kind {
                "method" => "method",
                "trait_method" => "tymethod",
                "assoc_type" => "associatedtype",
                _ => "associatedconstant",
            };
            return format!("{base}/{parent_dir}/?search={parent}#{anchor}.{last}");
        }
        _ => return format!("{base}/{dir}/?search={last}"),
    };
    format!("{base}/{dir}/{prefix}.{last}.html")
}

// ---- type rendering ----------------------------------------------------------------

fn path_name(p: &Value) -> String {
    let base = p
        .get("path")
        .or_else(|| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_owned();
    format!("{base}{}", render_generic_args(p.get("args")))
}

fn render_generic_args(args: Option<&Value>) -> String {
    let Some(args) = args.filter(|a| !a.is_null()) else {
        return String::new();
    };
    if let Some(ab) = args.get("angle_bracketed") {
        let mut parts: Vec<String> = arr(ab, "args")
            .iter()
            .map(|a| {
                if let Some(t) = a.get("type") {
                    render_type(t)
                } else if let Some(l) = a.get("lifetime").and_then(Value::as_str) {
                    l.to_owned()
                } else if let Some(c) = a.get("const") {
                    c.get("expr")
                        .and_then(Value::as_str)
                        .unwrap_or("_")
                        .to_owned()
                } else {
                    "_".to_owned()
                }
            })
            .collect();
        for c in arr(ab, "constraints").iter().chain(arr(ab, "bindings")) {
            let name = c.get("name").and_then(Value::as_str).unwrap_or("?");
            let binding = c.get("binding");
            if let Some(eq) = binding.and_then(|b| b.get("equality")) {
                let rhs = eq.get("type").map_or_else(|| render_type(eq), render_type);
                parts.push(format!("{name} = {rhs}"));
            } else if let Some(bounds) = binding
                .and_then(|b| b.get("constraint"))
                .and_then(Value::as_array)
            {
                parts.push(format!("{name}: {}", render_bounds(bounds)));
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("<{}>", parts.join(", "))
        }
    } else if let Some(p) = args.get("parenthesized") {
        let inputs: Vec<String> = arr(p, "inputs").iter().map(render_type).collect();
        let output = p
            .get("output")
            .filter(|o| !o.is_null())
            .map(|o| format!(" -> {}", render_type(o)))
            .unwrap_or_default();
        format!("({}){output}", inputs.join(", "))
    } else {
        String::new()
    }
}

fn render_bounds(bounds: &[Value]) -> String {
    bounds
        .iter()
        .map(|b| {
            if let Some(tb) = b.get("trait_bound") {
                let modifier = match tb.get("modifier").and_then(Value::as_str) {
                    Some("maybe") => "?",
                    Some("maybe_const") => "~const ",
                    _ => "",
                };
                format!(
                    "{modifier}{}",
                    tb.get("trait").map(path_name).unwrap_or_default()
                )
            } else if let Some(o) = b.get("outlives").and_then(Value::as_str) {
                o.to_owned()
            } else if b.get("use").is_some() {
                "use<..>".to_owned()
            } else {
                "?".to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

pub fn render_type(t: &Value) -> String {
    let Some((kind, v)) = (match t {
        Value::Object(m) => m.iter().next().map(|(k, v)| (k.as_str(), v)),
        _ => None,
    }) else {
        return "_".into();
    };
    match kind {
        "resolved_path" => path_name(v),
        "generic" | "primitive" => v.as_str().unwrap_or("?").to_owned(),
        "borrowed_ref" => {
            let lt = v
                .get("lifetime")
                .and_then(Value::as_str)
                .map(|l| format!("{l} "))
                .unwrap_or_default();
            let m = if v.get("is_mutable").and_then(Value::as_bool) == Some(true) {
                "mut "
            } else {
                ""
            };
            format!(
                "&{lt}{m}{}",
                v.get("type").map(render_type).unwrap_or_default()
            )
        }
        "raw_pointer" => {
            let m = if v.get("is_mutable").and_then(Value::as_bool) == Some(true) {
                "mut"
            } else {
                "const"
            };
            format!(
                "*{m} {}",
                v.get("type").map(render_type).unwrap_or_default()
            )
        }
        "slice" => format!("[{}]", render_type(v)),
        "array" => format!(
            "[{}; {}]",
            v.get("type").map(render_type).unwrap_or_default(),
            v.get("len").and_then(Value::as_str).unwrap_or("_")
        ),
        "tuple" => {
            let parts: Vec<String> = v
                .as_array()
                .map(|a| a.iter().map(render_type).collect())
                .unwrap_or_default();
            if parts.len() == 1 {
                format!("({},)", parts[0])
            } else {
                format!("({})", parts.join(", "))
            }
        }
        "impl_trait" => format!(
            "impl {}",
            render_bounds(v.as_array().map_or(&[], Vec::as_slice))
        ),
        "dyn_trait" => {
            let traits: Vec<String> = arr(v, "traits")
                .iter()
                .map(|p| p.get("trait").map(path_name).unwrap_or_default())
                .collect();
            let lt = v
                .get("lifetime")
                .and_then(Value::as_str)
                .map(|l| format!(" + {l}"))
                .unwrap_or_default();
            format!("dyn {}{lt}", traits.join(" + "))
        }
        "qualified_path" => {
            let name = v.get("name").and_then(Value::as_str).unwrap_or("?");
            let self_ty = v.get("self_type").map(render_type).unwrap_or_default();
            let trait_ = v
                .get("trait")
                .filter(|t| !t.is_null() && t.get("path").and_then(Value::as_str) != Some(""));
            match trait_ {
                Some(tr) => format!("<{self_ty} as {}>::{name}", path_name(tr)),
                None => format!("{self_ty}::{name}"),
            }
        }
        "function_pointer" => {
            let sig = v.get("sig").unwrap_or(v);
            let inputs: Vec<String> = arr(sig, "inputs")
                .iter()
                .map(|i| {
                    i.as_array()
                        .and_then(|p| p.get(1))
                        .map(render_type)
                        .unwrap_or_default()
                })
                .collect();
            let output = sig
                .get("output")
                .filter(|o| !o.is_null())
                .map(|o| format!(" -> {}", render_type(o)))
                .unwrap_or_default();
            format!("fn({}){output}", inputs.join(", "))
        }
        "pat" => v.get("type").map_or_else(|| "_".into(), render_type),
        _ => "_".into(),
    }
}

fn render_generics(g: Option<&Value>) -> String {
    let Some(g) = g else { return String::new() };
    let params: Vec<String> = arr(g, "params")
        .iter()
        .filter_map(|p| {
            let name = p.get("name").and_then(Value::as_str)?;
            let kind = p.get("kind")?;
            if let Some(ty) = kind.get("type") {
                if ty.get("is_synthetic").and_then(Value::as_bool) == Some(true) {
                    return None; // `impl Trait` in argument position
                }
                let bounds = render_bounds(arr(ty, "bounds"));
                Some(if bounds.is_empty() {
                    name.to_owned()
                } else {
                    format!("{name}: {bounds}")
                })
            } else if kind.get("lifetime").is_some() {
                Some(name.to_owned())
            } else if let Some(c) = kind.get("const") {
                Some(format!(
                    "const {name}: {}",
                    c.get("type").map(render_type).unwrap_or_default()
                ))
            } else {
                Some(name.to_owned())
            }
        })
        .collect();
    if params.is_empty() {
        String::new()
    } else {
        format!("<{}>", params.join(", "))
    }
}

fn render_where(g: Option<&Value>) -> String {
    let Some(g) = g else { return String::new() };
    let preds: Vec<String> = arr(g, "where_predicates")
        .iter()
        .filter_map(|p| {
            let bp = p.get("bound_predicate")?;
            let ty = bp.get("type").map(render_type)?;
            let bounds = render_bounds(arr(bp, "bounds"));
            (!bounds.is_empty()).then(|| format!("{ty}: {bounds}"))
        })
        .collect();
    if preds.is_empty() {
        String::new()
    } else {
        format!("\nwhere\n    {},", preds.join(",\n    "))
    }
}

fn render_fn(vis: &str, name: &str, inner: &Value) -> String {
    let header = inner.get("header");
    let flag = |k: &str| header.and_then(|h| h.get(k)).and_then(Value::as_bool) == Some(true);
    let mut quals = String::new();
    if flag("is_const") {
        quals.push_str("const ");
    }
    if flag("is_async") {
        quals.push_str("async ");
    }
    if flag("is_unsafe") {
        quals.push_str("unsafe ");
    }
    let sig = inner
        .get("sig")
        .or_else(|| inner.get("decl"))
        .unwrap_or(&Value::Null);
    let inputs: Vec<String> = arr(sig, "inputs")
        .iter()
        .map(|pair| {
            let p = pair.as_array().map_or(&[][..], Vec::as_slice);
            let n = p.first().and_then(Value::as_str).unwrap_or("_");
            let ty = p.get(1).map(render_type).unwrap_or_default();
            if n == "self" {
                match ty.as_str() {
                    "Self" => "self".to_owned(),
                    "&Self" => "&self".to_owned(),
                    "&mut Self" => "&mut self".to_owned(),
                    other => format!("self: {other}"),
                }
            } else {
                format!("{n}: {ty}")
            }
        })
        .collect();
    let output = sig
        .get("output")
        .filter(|o| !o.is_null())
        .map(|o| format!(" -> {}", render_type(o)))
        .unwrap_or_default();
    let generics = inner.get("generics");
    format!(
        "{vis}{quals}fn {name}{}({}){output}{}",
        render_generics(generics),
        inputs.join(", "),
        render_where(generics)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_types() {
        let t = json!({"borrowed_ref": {"lifetime": null, "is_mutable": true, "type": {"resolved_path": {"path": "Vec", "id": 1, "args": {"angle_bracketed": {"args": [{"type": {"primitive": "u8"}}], "constraints": []}}}}}});
        assert_eq!(render_type(&t), "&mut Vec<u8>");
        let t = json!({"impl_trait": [{"trait_bound": {"trait": {"path": "Fn", "id": 2, "args": {"parenthesized": {"inputs": [{"generic": "T"}], "output": {"primitive": "bool"}}}}, "generic_params": [], "modifier": "none"}}]});
        assert_eq!(render_type(&t), "impl Fn(T) -> bool");
    }

    #[test]
    fn survives_reexport_cycles() {
        // mod a { pub use crate::b; pub use super::*; }  mod b { pub use crate::a; } — cycles everywhere.
        let json = json!({
            "root": 0, "crate_version": "0.1.0",
            "index": {
                "0": {"name": "demo", "visibility": "public", "inner": {"module": {"items": [1, 2]}}},
                "1": {"name": "a", "visibility": "public", "inner": {"module": {"items": [3, 4, 7]}}},
                "2": {"name": "b", "visibility": "public", "inner": {"module": {"items": [5, 6]}}},
                "3": {"name": null, "visibility": "public", "inner": {"use": {"name": "b", "id": 2, "is_glob": false}}},
                "4": {"name": null, "visibility": "public", "inner": {"use": {"name": "", "id": 0, "is_glob": true}}},
                "5": {"name": null, "visibility": "public", "inner": {"use": {"name": "a", "id": 1, "is_glob": false}}},
                "6": {"name": null, "visibility": "public", "inner": {"use": {"name": "", "id": 1, "is_glob": true}}},
                "7": {"name": "f", "visibility": "public", "docs": "F.", "inner": {"function": {"sig": {"inputs": [], "output": null}, "generics": {"params": [], "where_predicates": []}, "header": {}}}}
            }
        });
        let docs = convert("demo", &json).expect("converts");
        assert!(
            docs.len() < 20,
            "cycle must not explode: {} docs",
            docs.len()
        );
        assert!(docs.iter().any(|d| d.path.as_deref() == Some("demo::a::f")));
    }

    #[test]
    fn converts_minimal_crate() {
        let json = json!({
            "root": 0,
            "crate_version": "1.2.3",
            "format_version": 61,
            "index": {
                "0": {"id": 0, "name": "demo", "visibility": "public", "docs": "Demo crate.", "inner": {"module": {"is_crate": true, "items": [1, 3], "is_stripped": false}}},
                "1": {"id": 1, "name": "Thing", "visibility": "public", "docs": "A thing. More text.", "deprecation": null,
                      "inner": {"struct": {"kind": {"plain": {"fields": [], "has_stripped_fields": true}}, "generics": {"params": [], "where_predicates": []}, "impls": [2]}}},
                "2": {"id": 2, "name": null, "visibility": "default", "docs": null, "inner": {"impl": {"trait": null, "for": {"resolved_path": {"path": "Thing", "id": 1, "args": null}}, "items": [4], "is_synthetic": false, "blanket_impl": null}}},
                "3": {"id": 3, "name": "helper", "visibility": "public", "docs": "Helps.", "inner": {"function": {"sig": {"inputs": [["x", {"primitive": "u32"}]], "output": {"primitive": "bool"}}, "generics": {"params": [], "where_predicates": []}, "header": {"is_async": true}}}},
                "4": {"id": 4, "name": "new", "visibility": "public", "docs": "Creates a thing.", "inner": {"function": {"sig": {"inputs": [], "output": {"generic": "Self"}}, "generics": {"params": [], "where_predicates": []}, "header": {}}}}
            }
        });
        let docs = convert("demo", &json).expect("converts");
        let paths: Vec<_> = docs.iter().filter_map(|d| d.path.as_deref()).collect();
        assert_eq!(
            paths,
            ["demo", "demo::Thing::new", "demo::Thing", "demo::helper"]
        );
        let helper = docs
            .iter()
            .find(|d| d.path.as_deref() == Some("demo::helper"))
            .expect("helper");
        assert!(
            helper
                .body
                .starts_with("```rust\npub async fn helper(x: u32) -> bool\n```"),
            "{}",
            helper.body
        );
        let new = docs.iter().find(|d| d.title == "new").expect("method");
        assert_eq!(new.parent.as_deref(), Some("demo::Thing"));
        assert_eq!(new.kind.as_deref(), Some("method"));
        let thing = docs.iter().find(|d| d.title == "Thing").expect("struct");
        assert_eq!(thing.summary, "A thing.");
        assert!(thing.body.contains("## Methods\n`new`"));
        assert_eq!(docs[0].kind.as_deref(), Some("crate"));
    }
}
