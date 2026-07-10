use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, process::Command};

use regex::Regex;

use crate::cli::CommandRegistrar;
use crate::config::api_docs_metadata::{
    append_module_notes, ensure_single_trailing_newline, module_description,
};
use crate::foundation::Error;
use crate::support::generated_manifest::{
    clean_manifest_files, create_generated_dir_all, safe_manifest_path_with_extension,
    write_generated_file, write_manifest,
};
use crate::support::CommandId;

const DOCS_API_COMMAND: CommandId = CommandId::new("docs:api");
const API_DOCS_MANIFEST: &str = ".foundry-api-docs-manifest.json";

pub(crate) fn docs_api_cli_registrar() -> CommandRegistrar {
    Arc::new(|registry| {
        registry.command(
            DOCS_API_COMMAND,
            clap::Command::new(DOCS_API_COMMAND.as_str().to_string())
                .about("Generate API surface docs from cargo doc HTML output")
                .arg(
                    clap::Arg::new("path")
                        .long("path")
                        .value_name("DIR")
                        .default_value("docs/api")
                        .help("Directory to write the API docs to"),
                ),
            |invocation| async move {
                let dir = invocation
                    .matches()
                    .get_one::<String>("path")
                    .map(|s| s.as_str())
                    .unwrap_or("docs/api");

                generate_api_docs(dir)?;
                Ok(())
            },
        )?;
        Ok(())
    })
}

fn generate_api_docs(output_dir: &str) -> Result<(), Error> {
    // Run cargo doc --no-deps
    println!("Running cargo doc --no-deps...");
    let status = Command::new("cargo")
        .args(["doc", "--no-deps", "--quiet"])
        .status()
        .map_err(|e| Error::message(format!("failed to run cargo doc: {e}")))?;
    if !status.success() {
        return Err(Error::message("cargo doc failed"));
    }

    // Locate the doc output
    let doc_root = find_doc_root()?;

    // Discover modules
    let mut all_modules = BTreeMap::new();
    discover_modules(&doc_root, "", &mut all_modules);

    if all_modules.is_empty() {
        return Err(Error::message("no modules found in cargo doc output"));
    }

    // Auto-group by top-level module
    let skip = ["prelude"];
    let groups = auto_group_modules(&all_modules, &skip);

    // Prepare output directories
    let out = PathBuf::from(output_dir);
    create_generated_dir_all(&out, Path::new("modules"))?;
    let planned_files = planned_api_doc_files(&groups);
    clean_api_docs_manifest_files(&out, &planned_files)?;

    let mut index_entries: Vec<(String, String, usize)> = Vec::new();
    let mut output_files = BTreeSet::new();

    // Write root.md
    if let Some(root_paths) = groups.get("") {
        let mut content = String::new();
        writeln!(content, "# Foundry Crate Root").unwrap();
        writeln!(content).unwrap();
        writeln!(content, "Derive macros and re-exports at the crate root.").unwrap();
        writeln!(content).unwrap();
        writeln!(content, "[Back to index](index.md)").unwrap();
        writeln!(content).unwrap();

        for mod_path in root_paths {
            if let Some(items) = all_modules.get(mod_path.as_str()) {
                let filtered = filter_root_items(items);
                let section = format_module(&doc_root, mod_path, &filtered);
                if !section.is_empty() {
                    writeln!(content, "```rust").unwrap();
                    content.push_str(&section);
                    writeln!(content, "```").unwrap();
                }
            }
        }

        let lines = content.lines().count();
        let file = "root.md";
        write_generated_file(&out, Path::new(file), &content)?;
        output_files.insert(file.to_string());
        index_entries.push((
            "root".into(),
            "Crate root: derive macros, re-exports".into(),
            lines,
        ));
    }

    // Write module files
    for (group_key, mod_paths) in &groups {
        if group_key.is_empty() {
            continue;
        }

        let desc = module_description(group_key).to_string();
        let mut content = String::new();
        writeln!(content, "# {group_key}").unwrap();
        if !desc.is_empty() {
            writeln!(content).unwrap();
            writeln!(content, "{desc}").unwrap();
        }
        writeln!(content).unwrap();
        writeln!(content, "[Back to index](../index.md)").unwrap();
        writeln!(content).unwrap();

        let mut has_content = false;
        for mod_path in mod_paths {
            let Some(items) = all_modules.get(mod_path.as_str()) else {
                continue;
            };
            let display = format!("foundry::{mod_path}");
            let section = format_module(&doc_root, mod_path, items);
            if !section.is_empty() {
                writeln!(content, "## {display}").unwrap();
                writeln!(content).unwrap();
                writeln!(content, "```rust").unwrap();
                content.push_str(&section);
                writeln!(content, "```").unwrap();
                writeln!(content).unwrap();
                has_content = true;
            }
        }

        if has_content {
            append_module_notes(group_key, &mut content);
            ensure_single_trailing_newline(&mut content);
            let lines = content.lines().count();
            let file = format!("modules/{group_key}.md");
            write_generated_file(&out, Path::new(&file), &content)?;
            output_files.insert(file);
            index_entries.push((group_key.to_string(), desc, lines));
        }
    }

    // Sort: root first, then alphabetical
    index_entries.sort_by(|a, b| match (a.0.as_str(), b.0.as_str()) {
        ("root", _) => std::cmp::Ordering::Less,
        (_, "root") => std::cmp::Ordering::Greater,
        _ => a.0.cmp(&b.0),
    });

    // Write index.md
    let total_lines: usize = index_entries.iter().map(|(_, _, l)| l).sum();
    let mut index = String::new();
    writeln!(index, "# Foundry API Surface").unwrap();
    writeln!(index).unwrap();
    writeln!(
        index,
        "> Auto-generated from `cargo doc`. Regenerate: `docs:api`"
    )
    .unwrap();
    writeln!(index).unwrap();
    writeln!(
        index,
        "Each file documents one module's public API (structs, enums, traits, functions)."
    )
    .unwrap();
    writeln!(
        index,
        "Load only the file you need — don't read them all at once."
    )
    .unwrap();
    writeln!(index).unwrap();
    writeln!(
        index,
        "For import stability and compatibility expectations, see [Public API Contract](public-api-contract.md)."
    )
    .unwrap();
    writeln!(index).unwrap();
    writeln!(index, "| Module | Description | Size |").unwrap();
    writeln!(index, "|--------|-------------|------|").unwrap();

    for (stem, desc, lines) in &index_entries {
        let link = if stem == "root" {
            format!("[{stem}](root.md)")
        } else {
            format!("[{stem}](modules/{stem}.md)")
        };
        writeln!(index, "| {link} | {desc} | {lines}L |").unwrap();
    }
    writeln!(index).unwrap();
    writeln!(
        index,
        "**Total: {} modules, {total_lines} lines across all files.**",
        index_entries.len(),
    )
    .unwrap();

    write_generated_file(&out, Path::new("index.md"), &index)?;
    output_files.insert("index.md".to_string());
    write_manifest(&out, API_DOCS_MANIFEST, &output_files)?;

    println!(
        "API docs generated: {} modules, {total_lines} lines → {}",
        index_entries.len(),
        output_dir,
    );

    Ok(())
}

fn planned_api_doc_files(groups: &BTreeMap<String, Vec<String>>) -> BTreeSet<String> {
    let mut files = BTreeSet::from(["index.md".to_string(), "root.md".to_string()]);
    for group_key in groups.keys().filter(|group_key| !group_key.is_empty()) {
        files.insert(format!("modules/{group_key}.md"));
    }
    files
}

fn clean_api_docs_manifest_files(
    dir: &Path,
    planned_files: &BTreeSet<String>,
) -> Result<(), Error> {
    clean_manifest_files(
        dir,
        API_DOCS_MANIFEST,
        planned_files,
        "foundry.docs_api",
        safe_api_docs_manifest_path,
    )
}

fn safe_api_docs_manifest_path(file: &str) -> Option<PathBuf> {
    let path = safe_manifest_path_with_extension(file, "md", true)?;
    let mut components = path.components();
    match (
        components.next(),
        components.next(),
        components.next(),
        components.next(),
    ) {
        (Some(std::path::Component::Normal(name)), None, None, None)
            if name == "index.md" || name == "root.md" =>
        {
            Some(path)
        }
        (
            Some(std::path::Component::Normal(dir)),
            Some(std::path::Component::Normal(_file)),
            None,
            None,
        ) if dir == "modules" => Some(path),
        _ => None,
    }
}

// ── Internals ────────────────────────────────────────────────────────

fn find_doc_root() -> Result<PathBuf, Error> {
    // Check standard cargo doc output location
    let target_dir = PathBuf::from("target/doc/foundry");
    if target_dir.exists() {
        return Ok(target_dir);
    }
    // Try from CARGO_TARGET_DIR env
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        let p = PathBuf::from(dir).join("doc/foundry");
        if p.exists() {
            return Ok(p);
        }
    }
    Err(Error::message(
        "cannot find cargo doc output at target/doc/foundry — run `cargo doc --no-deps` first",
    ))
}

fn auto_group_modules(
    all_modules: &BTreeMap<String, BTreeMap<String, Vec<String>>>,
    skip: &[&str],
) -> BTreeMap<String, Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for mod_path in all_modules.keys() {
        let top = if mod_path.is_empty() {
            String::new()
        } else {
            mod_path.split("::").next().unwrap_or(mod_path).to_string()
        };
        if skip.contains(&top.as_str()) {
            continue;
        }
        groups.entry(top).or_default().push(mod_path.clone());
    }
    groups
}

fn filter_root_items(items: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    let skip_structs = ["Cookie", "CookieJar"];
    let mut filtered = BTreeMap::new();
    for (kind, names) in items {
        let clean: Vec<String> = names
            .iter()
            .filter(|n| !(kind == "struct" && skip_structs.contains(&n.as_str())))
            .cloned()
            .collect();
        if !clean.is_empty() {
            filtered.insert(kind.clone(), clean);
        }
    }
    filtered
}

// ── Module discovery (sidebar-items.js) ──────────────────────────────

fn discover_modules(
    doc_root: &Path,
    mod_path: &str,
    modules: &mut BTreeMap<String, BTreeMap<String, Vec<String>>>,
) {
    let dir = if mod_path.is_empty() {
        doc_root.to_path_buf()
    } else {
        doc_root.join(mod_path.replace("::", "/"))
    };

    let sidebar_path = dir.join("sidebar-items.js");
    let content = match fs::read_to_string(&sidebar_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let json_str = content
        .trim()
        .strip_prefix("window.SIDEBAR_ITEMS = ")
        .and_then(|s| s.strip_suffix(';'))
        .unwrap_or("{}");

    let data: serde_json::Value = serde_json::from_str(json_str).unwrap_or_default();
    let obj = match data.as_object() {
        Some(o) => o,
        None => return,
    };

    let mut items: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (kind, names) in obj {
        if kind == "mod" {
            if let Some(arr) = names.as_array() {
                for name in arr.iter().filter_map(|v| v.as_str()) {
                    if name.starts_with('_') {
                        continue;
                    }
                    let child = if mod_path.is_empty() {
                        name.to_string()
                    } else {
                        format!("{mod_path}::{name}")
                    };
                    discover_modules(doc_root, &child, modules);
                }
            }
            continue;
        }

        if let Some(arr) = names.as_array() {
            let v: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if !v.is_empty() {
                items.insert(kind.clone(), v);
            }
        }
    }

    if !items.is_empty() {
        modules.insert(mod_path.to_string(), items);
    }
}

// ── Formatting (regex-based, no scraper dependency) ──────────────────

fn format_module(doc_root: &Path, mod_path: &str, items: &BTreeMap<String, Vec<String>>) -> String {
    let mut out = String::new();
    let dir = if mod_path.is_empty() {
        doc_root.to_path_buf()
    } else {
        doc_root.join(mod_path.replace("::", "/"))
    };

    for kind in [
        "derive", "macro", "constant", "static", "type", "enum", "struct", "trait", "fn",
    ] {
        let Some(names) = items.get(kind) else {
            continue;
        };
        for name in names {
            let filename = match kind {
                "struct" => format!("struct.{name}.html"),
                "enum" => format!("enum.{name}.html"),
                "trait" => format!("trait.{name}.html"),
                "fn" => format!("fn.{name}.html"),
                "type" => format!("type.{name}.html"),
                "constant" => format!("constant.{name}.html"),
                "static" => format!("static.{name}.html"),
                "derive" => format!("derive.{name}.html"),
                "macro" => format!("macro.{name}.html"),
                _ => continue,
            };
            let html_path = dir.join(&filename);
            let html = match fs::read_to_string(&html_path) {
                Ok(h) => h,
                Err(_) => continue,
            };

            match kind {
                "derive" => writeln!(out, "derive {name}").unwrap(),
                "macro" => writeln!(out, "macro {name}!").unwrap(),
                "struct" => {
                    writeln!(out, "struct {name}").unwrap();
                    for m in extract_own_methods(&html) {
                        writeln!(out, "  {m}").unwrap();
                    }
                }
                "enum" => {
                    let decl = extract_item_decl(&html);
                    let variants = extract_enum_variants(&decl);
                    if variants.is_empty() {
                        writeln!(out, "enum {name}").unwrap();
                    } else if variants.len() > 15 {
                        let preview: Vec<_> = variants.iter().take(5).map(|s| s.as_str()).collect();
                        writeln!(
                            out,
                            "enum {name} {{ {}, ... +{} more }}",
                            preview.join(", "),
                            variants.len() - 5
                        )
                        .unwrap();
                    } else {
                        writeln!(out, "enum {name} {{ {} }}", variants.join(", ")).unwrap();
                    }
                    for m in extract_own_methods(&html) {
                        writeln!(out, "  {m}").unwrap();
                    }
                }
                "trait" => {
                    let decl = extract_item_decl(&html);
                    writeln!(out, "trait {}", simplify_trait_decl(&decl, name)).unwrap();
                    for m in extract_trait_methods(&decl) {
                        writeln!(out, "  {m}").unwrap();
                    }
                }
                "fn" => {
                    let sig = simplify_fn_decl(&extract_item_decl(&html));
                    if !sig.is_empty() {
                        writeln!(out, "{sig}").unwrap();
                    }
                }
                "type" => {
                    let clean = extract_item_decl(&html).trim().to_string();
                    if !clean.is_empty() {
                        writeln!(out, "{clean}").unwrap();
                    }
                }
                "constant" => {
                    let clean = extract_item_decl(&html).trim().to_string();
                    if !clean.is_empty() {
                        let trimmed = if let Some(pos) = clean.find(" = ") {
                            format!("{};", &clean[..pos])
                        } else {
                            clean
                        };
                        writeln!(out, "{trimmed}").unwrap();
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Extract `<pre class="rust item-decl">...</pre>` content via regex.
fn extract_item_decl(html: &str) -> String {
    let re = Regex::new(r#"<pre class="rust item-decl">([\s\S]*?)</pre>"#).unwrap();
    re.captures(html)
        .and_then(|c| c.get(1))
        .map(|m| decode_and_strip_tags(m.as_str()))
        .unwrap_or_default()
}

/// Extract method signatures from `#implementations-list` section only.
fn extract_own_methods(html: &str) -> Vec<String> {
    // Find the implementations-list section, stop before trait-implementations or synthetic
    let start = match html.find("id=\"implementations-list\"") {
        Some(p) => p,
        None => return Vec::new(),
    };

    let rest = &html[start..];
    let end = rest
        .find("id=\"trait-implementations-list\"")
        .or_else(|| rest.find("id=\"synthetic-implementations-list\""))
        .or_else(|| rest.find("id=\"blanket-implementations-list\""))
        .unwrap_or(rest.len());

    let section = &rest[..end];

    let re = Regex::new(r#"<h4 class="code-header">(.*?)</h4>"#).unwrap();
    re.captures_iter(section)
        .filter_map(|c| {
            let raw = decode_and_strip_tags(c.get(1)?.as_str());
            let sig = simplify_method_sig(&raw);
            if sig.len() > 3 {
                Some(sig)
            } else {
                None
            }
        })
        .collect()
}

fn extract_enum_variants(decl: &str) -> Vec<String> {
    let start = match decl.find('{') {
        Some(p) => p + 1,
        None => return Vec::new(),
    };
    let end = match decl.rfind('}') {
        Some(p) => p,
        None => return Vec::new(),
    };

    decl[start..end]
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            if t.is_empty() || t.starts_with("//") || t.starts_with("/*") {
                return None;
            }
            let name = t.split(['(', '{', ',']).next()?.trim();
            if !name.is_empty() && name.chars().next().is_some_and(|c| c.is_uppercase()) {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn extract_trait_methods(decl: &str) -> Vec<String> {
    let mut methods: Vec<String> = decl
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            if t.starts_with("fn ") || t.starts_with("async fn ") {
                let sig = simplify_method_sig(&format!("pub {t}"));
                if !sig.is_empty() {
                    Some(sig)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Fallback: extract from desugared async_trait form
    if methods.is_empty() {
        methods = extract_desugared_async_methods(decl);
    }
    methods
}

fn extract_desugared_async_methods(decl: &str) -> Vec<String> {
    let mut methods = Vec::new();
    let mut current_fn = String::new();
    let mut in_fn = false;
    let mut brace_depth: usize = 0;

    for line in decl.lines() {
        let t = line.trim();
        if !in_fn && t.starts_with("fn ") {
            in_fn = true;
            current_fn = t.to_string();
            brace_depth = t
                .matches('{')
                .count()
                .saturating_sub(t.matches('}').count());
            if t.ends_with("{ ... }") || t.ends_with(';') {
                push_desugared(&mut methods, &current_fn);
                in_fn = false;
                current_fn.clear();
            }
        } else if in_fn {
            current_fn.push(' ');
            current_fn.push_str(t);
            brace_depth += t.matches('{').count();
            brace_depth = brace_depth.saturating_sub(t.matches('}').count());
            if t.ends_with("{ ... }") || t.ends_with(';') || brace_depth == 0 {
                push_desugared(&mut methods, &current_fn);
                in_fn = false;
                current_fn.clear();
            }
        }
    }
    methods
}

fn push_desugared(methods: &mut Vec<String>, sig: &str) {
    let is_async = sig.contains("Pin<Box<dyn Future<Output =");
    let fn_start = match sig.find("fn ") {
        Some(p) => p,
        None => return,
    };
    let after = &sig[fn_start + 3..];
    let name_end = after.find(['<', '(']).unwrap_or(after.len());
    let name = after[..name_end].trim();
    let clean_params = clean_params(&extract_params(sig));

    let result = if is_async {
        let ret = extract_async_return_type(sig);
        if ret == "()" {
            format!("async fn {name}({clean_params})")
        } else {
            format!("async fn {name}({clean_params}) -> {ret}")
        }
    } else {
        let ret = extract_return_type(sig);
        if ret.is_empty() || ret == "()" {
            format!("fn {name}({clean_params})")
        } else {
            format!("fn {name}({clean_params}) -> {ret}")
        }
    };

    if !result.is_empty() {
        methods.push(result);
    }
}

// ── Signature helpers ────────────────────────────────────────────────

fn simplify_method_sig(sig: &str) -> String {
    let s = sig.trim().strip_prefix("pub ").unwrap_or(sig.trim());
    let clean = strip_where_clause(s)
        .replace("{ … }", "")
        .replace("{ ... }", "");
    strip_where_from_sig(&collapse_whitespace(
        clean.trim().trim_end_matches(';').trim(),
    ))
}

fn simplify_trait_decl(decl: &str, name: &str) -> String {
    let mut supers = Vec::new();
    let mut collecting = false;
    for line in decl.lines() {
        let t = line.trim();
        if t.starts_with("pub trait ") {
            if t.contains(':') {
                collecting = true;
                if let Some(after) = t.split(':').nth(1) {
                    for p in after.split('+') {
                        let p = p.trim().trim_end_matches('{').trim();
                        if !p.is_empty() {
                            supers.push(p.to_string());
                        }
                    }
                }
            }
            continue;
        }
        if collecting {
            if t.starts_with('+') {
                let p = t
                    .trim_start_matches('+')
                    .trim()
                    .trim_end_matches('{')
                    .trim();
                if !p.is_empty() {
                    supers.push(p.to_string());
                }
            } else if t.starts_with("fn ") || t.starts_with("//") || t == "{" {
                break;
            }
        }
    }
    let supers: Vec<_> = supers
        .into_iter()
        .filter(|s| {
            !matches!(
                s.as_str(),
                "Send" | "Sync" | "'static" | "Sized" | "'static {"
            )
        })
        .collect();
    if supers.is_empty() {
        name.to_string()
    } else {
        format!("{name}: {}", supers.join(" + "))
    }
}

fn simplify_fn_decl(decl: &str) -> String {
    let s = decl.trim().strip_prefix("pub ").unwrap_or(decl.trim());
    let clean = collapse_whitespace(strip_where_clause(s).trim().trim_end_matches(';').trim());
    let fn_part = clean.strip_prefix("fn ").unwrap_or(&clean);
    format!("fn {fn_part}")
}

fn strip_where_clause(sig: &str) -> String {
    let mut depth: usize = 0;
    let mut paren: usize = 0;
    for (i, c) in sig.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            'w' if depth == 0
                && paren == 0
                && sig[i..].starts_with("where")
                && (i == 0 || matches!(sig.as_bytes()[i - 1], b' ' | b'\n')) =>
            {
                return sig[..i].trim().to_string();
            }
            _ => {}
        }
    }
    sig.to_string()
}

fn strip_where_from_sig(sig: &str) -> String {
    if let Some(pos) = sig.find("where ") {
        let before = &sig[..pos];
        if before.matches('<').count() <= before.matches('>').count() {
            return before.trim().trim_end_matches(',').trim().to_string();
        }
    }
    sig.to_string()
}

fn extract_params(sig: &str) -> String {
    let mut depth = 0;
    let mut start = None;
    let mut end = None;
    for (i, c) in sig.char_indices() {
        match c {
            '(' if depth == 0 => {
                start = Some(i + 1);
                depth = 1;
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    match (start, end) {
        (Some(s), Some(e)) => sig[s..e].to_string(),
        _ => String::new(),
    }
}

fn clean_params(params: &str) -> String {
    split_params(params)
        .iter()
        .filter(|p| !p.trim().is_empty() && !p.trim().starts_with('\''))
        .map(|p| clean_single_param(p.trim()))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn split_params(params: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    for c in params.chars() {
        match c {
            '<' | '(' => {
                depth += 1;
                current.push(c);
            }
            '>' | ')' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                result.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    let t = current.trim().to_string();
    if !t.is_empty() {
        result.push(t);
    }
    result
}

fn clean_single_param(param: &str) -> String {
    let mut r = param.to_string();
    for (from, to) in [
        ("&'life0 ", "&"),
        ("&'life1 ", "&"),
        ("&'life2 ", "&"),
        ("&'a ", "&"),
        ("&'_ ", "&"),
        ("<'life0, 'life1, 'async_trait>", ""),
        ("<'life0, 'async_trait>", ""),
        ("<'async_trait>", ""),
    ] {
        r = r.replace(from, to);
    }
    if let Some(pos) = r.find(':') {
        let name = r[..pos].trim();
        if name.starts_with('_') && name.len() > 1 {
            r = format!("{}: {}", &name[1..], r[pos + 1..].trim());
        }
    }
    r
}

fn extract_async_return_type(sig: &str) -> String {
    if let Some(pos) = sig.find("Output = ") {
        let after = &sig[pos + 9..];
        let mut depth = 0;
        for (i, c) in after.char_indices() {
            match c {
                '<' => depth += 1,
                '>' if depth == 0 => return after[..i].trim().to_string(),
                '>' => depth -= 1,
                _ => {}
            }
        }
    }
    "()".to_string()
}

fn extract_return_type(sig: &str) -> String {
    let mut depth = 0;
    let mut past_params = false;
    for (i, c) in sig.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    past_params = true;
                }
            }
            '-' if past_params && depth == 0 && sig[i..].starts_with("->") => {
                let rest = sig[i + 2..].trim();
                let end = rest
                    .find("where")
                    .or_else(|| rest.find('{'))
                    .unwrap_or(rest.len());
                return rest[..end].trim().to_string();
            }
            _ => {}
        }
    }
    String::new()
}

fn collapse_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                result.push(' ');
            }
            prev_space = true;
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    result
}

fn decode_and_strip_tags(html: &str) -> String {
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    let stripped = tag_re.replace_all(html, "");
    stripped
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn api_docs_cleanup_removes_only_manifest_owned_docs() {
        let dir = tempdir().unwrap();
        let modules_dir = dir.path().join("modules");
        fs::create_dir_all(&modules_dir).unwrap();

        fs::write(dir.path().join("index.md"), "planned").unwrap();
        fs::write(dir.path().join("manual.md"), "manual").unwrap();
        fs::write(modules_dir.join("old.md"), "old").unwrap();
        fs::write(modules_dir.join("manual.md"), "manual").unwrap();
        fs::write(
            dir.path().join(API_DOCS_MANIFEST),
            serde_json::to_string(&vec![
                "modules/old.md",
                "manual.md",
                "../outside.md",
                "modules/../../unsafe.md",
                "modules/not-markdown.txt",
            ])
            .unwrap(),
        )
        .unwrap();

        let planned = BTreeSet::from(["index.md".to_string(), "modules/current.md".to_string()]);
        clean_api_docs_manifest_files(dir.path(), &planned).unwrap();

        assert!(!dir.path().join("index.md").exists());
        assert!(!modules_dir.join("old.md").exists());
        assert!(dir.path().join("manual.md").exists());
        assert!(modules_dir.join("manual.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn api_docs_cleanup_refuses_symlinked_modules_directory() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("old.md");
        fs::write(&outside_file, "outside").unwrap();
        symlink(outside.path(), dir.path().join("modules")).unwrap();
        fs::write(
            dir.path().join(API_DOCS_MANIFEST),
            serde_json::to_string(&vec!["modules/old.md"]).unwrap(),
        )
        .unwrap();

        let error = clean_api_docs_manifest_files(dir.path(), &BTreeSet::new()).unwrap_err();

        assert!(error.to_string().contains("symlink"));
        assert_eq!(fs::read_to_string(outside_file).unwrap(), "outside");
    }

    #[test]
    fn api_docs_manifest_path_is_limited_to_known_output_shape() {
        assert_eq!(
            safe_api_docs_manifest_path("index.md"),
            Some(PathBuf::from("index.md"))
        );
        assert_eq!(
            safe_api_docs_manifest_path("modules/http.md"),
            Some(PathBuf::from("modules/http.md"))
        );
        assert!(safe_api_docs_manifest_path("manual.md").is_none());
        assert!(safe_api_docs_manifest_path("modules/nested/http.md").is_none());
        assert!(safe_api_docs_manifest_path("../index.md").is_none());
        assert!(safe_api_docs_manifest_path("modules\\http.md").is_none());
        assert!(safe_api_docs_manifest_path("modules/http.txt").is_none());
    }
}
