use scraper::{Html, Selector};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "../../../src/config/api_docs_metadata.rs"]
mod api_docs_metadata;

use api_docs_metadata::{append_module_notes, module_description};

fn main() {
    let project_root = find_project_root();
    let doc_root = project_root.join("target/doc/foundry");

    // Parse CLI args: optional --output-dir <path> (default: docs/api)
    let out_dir = parse_output_dir(&project_root);

    // Run cargo doc if HTML not present
    if !doc_root.exists() {
        eprintln!("Running cargo doc --no-deps...");
        let status = Command::new("cargo")
            .args(["doc", "--no-deps"])
            .current_dir(&project_root)
            .status()
            .expect("failed to run cargo doc");
        if !status.success() {
            eprintln!("cargo doc failed");
            std::process::exit(1);
        }
    }

    // Discover all modules from rustdoc HTML
    let mut all_modules = BTreeMap::new();
    discover_modules(&doc_root, "", &mut all_modules);

    // Auto-group: submodules merge into their top-level parent file.
    // "" (root) -> root.md, "auth" + "auth::token" + ... -> modules/auth.md
    let skip = ["prelude"];
    let groups = auto_group_modules(&all_modules, &skip);

    // Clean and create output dirs
    let modules_dir = out_dir.join("modules");
    if out_dir.exists() {
        fs::remove_dir_all(&out_dir).expect("failed to clean output dir");
    }
    fs::create_dir_all(&modules_dir).expect("failed to create modules dir");

    let mut index_entries: Vec<(String, String, usize)> = Vec::new();

    // --- Write root.md ---
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
        fs::write(out_dir.join("root.md"), &content).expect("failed to write root.md");
        index_entries.push((
            "root".to_string(),
            "Crate root: derive macros, re-exports".to_string(),
            lines,
        ));
    }

    // --- Write module files into modules/ ---
    for (group_key, mod_paths) in &groups {
        if group_key.is_empty() {
            continue; // root already handled
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
            let lines = content.lines().count();
            fs::write(modules_dir.join(format!("{group_key}.md")), &content)
                .expect("failed to write module file");
            index_entries.push((group_key.to_string(), desc, lines));
        }
    }

    // Sort: root first, then alphabetical
    index_entries.sort_by(|a, b| {
        if a.0 == "root" {
            std::cmp::Ordering::Less
        } else if b.0 == "root" {
            std::cmp::Ordering::Greater
        } else {
            a.0.cmp(&b.0)
        }
    });

    // --- Write index.md ---
    let total_lines: usize = index_entries.iter().map(|(_, _, l)| l).sum();
    let mut index = String::new();
    writeln!(index, "# Foundry API Surface").unwrap();
    writeln!(index).unwrap();
    writeln!(
        index,
        "> Auto-generated from `cargo doc`. Regenerate: `make api-docs`"
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

    fs::write(out_dir.join("index.md"), &index).expect("failed to write index");

    eprintln!(
        "Wrote {} module files + index to {} ({total_lines} total lines)",
        index_entries.len(),
        out_dir.display(),
    );
}

/// Parse --output-dir from CLI args. Default: <project_root>/docs/api
fn parse_output_dir(project_root: &Path) -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    for i in 1..args.len() {
        if args[i] == "--output-dir" || args[i] == "-o" {
            if let Some(dir) = args.get(i + 1) {
                let p = PathBuf::from(dir);
                if p.is_absolute() {
                    return p;
                }
                return project_root.join(p);
            }
        }
    }
    project_root.join("docs/api")
}

fn find_project_root() -> PathBuf {
    let mut dir = std::env::current_dir().expect("no cwd");
    loop {
        if dir.join("Cargo.toml").exists() && dir.join("src/lib.rs").exists() {
            return dir;
        }
        if !dir.pop() {
            return std::env::current_dir().expect("no cwd");
        }
    }
}

/// Auto-group modules by top-level path segment.
/// "auth" + "auth::token" + "auth::session" → group key "auth"
/// "" (root) → group key ""
fn auto_group_modules(
    all_modules: &BTreeMap<String, BTreeMap<String, Vec<String>>>,
    skip: &[&str],
) -> BTreeMap<String, Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for mod_path in all_modules.keys() {
        // Skip filtered modules
        let top = top_level_module(mod_path);
        if skip.contains(&top.as_str()) {
            continue;
        }

        groups
            .entry(top)
            .or_default()
            .push(mod_path.clone());
    }

    groups
}

/// Get the top-level module name. "" stays "", "auth::token" -> "auth"
fn top_level_module(mod_path: &str) -> String {
    if mod_path.is_empty() {
        return String::new();
    }
    mod_path
        .split("::")
        .next()
        .unwrap_or(mod_path)
        .to_string()
}

/// Filter root-level items to remove re-exported third-party types
fn filter_root_items(
    items: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, Vec<String>> {
    let skip_structs: &[&str] = &["Cookie", "CookieJar"];
    let mut filtered = BTreeMap::new();
    for (kind, names) in items {
        let clean: Vec<String> = names
            .iter()
            .filter(|n| {
                if kind == "struct" {
                    !skip_structs.contains(&n.as_str())
                } else {
                    true
                }
            })
            .cloned()
            .collect();
        if !clean.is_empty() {
            filtered.insert(kind.clone(), clean);
        }
    }
    filtered
}

// ── Module discovery ─────────────────────────────────────────────────

/// Parse sidebar-items.js files recursively to build module → items map
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
    if !sidebar_path.exists() {
        return;
    }

    let content = fs::read_to_string(&sidebar_path).unwrap_or_default();
    let json_str = content
        .trim()
        .strip_prefix("window.SIDEBAR_ITEMS = ")
        .and_then(|s| s.strip_suffix(';'))
        .unwrap_or("{}");

    let data: Value = serde_json::from_str(json_str).unwrap_or(Value::Null);
    let obj = match data.as_object() {
        Some(o) => o,
        None => return,
    };

    let mut items: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (kind, names) in obj {
        if kind == "mod" {
            if let Some(arr) = names.as_array() {
                for name in arr {
                    if let Some(name_str) = name.as_str() {
                        if name_str.starts_with('_') {
                            continue;
                        }
                        let child_path = if mod_path.is_empty() {
                            name_str.to_string()
                        } else {
                            format!("{mod_path}::{name_str}")
                        };
                        discover_modules(doc_root, &child_path, modules);
                    }
                }
            }
            continue;
        }

        if let Some(arr) = names.as_array() {
            let names_vec: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if !names_vec.is_empty() {
                items.insert(kind.clone(), names_vec);
            }
        }
    }

    if !items.is_empty() {
        modules.insert(mod_path.to_string(), items);
    }
}

// ── Formatting ───────────────────────────────────────────────────────

fn format_module(
    doc_root: &Path,
    mod_path: &str,
    items: &BTreeMap<String, Vec<String>>,
) -> String {
    let mut out = String::new();
    let dir = if mod_path.is_empty() {
        doc_root.to_path_buf()
    } else {
        doc_root.join(mod_path.replace("::", "/"))
    };

    let kind_order = [
        "derive", "macro", "constant", "static", "type", "enum", "struct", "trait", "fn",
    ];

    for kind in kind_order {
        let Some(names) = items.get(kind) else {
            continue;
        };
        for name in names {
            let filename = format_filename(kind, name);
            let html_path = dir.join(&filename);
            if !html_path.exists() {
                continue;
            }

            let content = fs::read_to_string(&html_path).unwrap_or_default();
            let formatted = format_item(kind, name, &content);
            if !formatted.is_empty() {
                out.push_str(&formatted);
            }
        }
    }

    out
}

fn format_filename(kind: &str, name: &str) -> String {
    match kind {
        "struct" => format!("struct.{name}.html"),
        "enum" => format!("enum.{name}.html"),
        "trait" => format!("trait.{name}.html"),
        "fn" => format!("fn.{name}.html"),
        "type" => format!("type.{name}.html"),
        "constant" => format!("constant.{name}.html"),
        "static" => format!("static.{name}.html"),
        "derive" => format!("derive.{name}.html"),
        "macro" => format!("macro.{name}.html"),
        _ => format!("{kind}.{name}.html"),
    }
}

fn format_item(kind: &str, name: &str, html: &str) -> String {
    let mut out = String::new();
    match kind {
        "struct" => format_struct(&mut out, name, html),
        "enum" => format_enum(&mut out, name, html),
        "trait" => format_trait(&mut out, name, html),
        "fn" => format_fn(&mut out, name, html),
        "type" => format_type_alias(&mut out, name, html),
        "constant" => format_constant(&mut out, name, html),
        "derive" => {
            writeln!(out, "derive {name}").unwrap();
        }
        "macro" => {
            writeln!(out, "macro {name}!").unwrap();
        }
        _ => {}
    }
    out
}

fn format_struct(out: &mut String, name: &str, html: &str) {
    writeln!(out, "struct {name}").unwrap();
    for m in extract_own_methods(html) {
        if m.len() > 3 {
            writeln!(out, "  {m}").unwrap();
        }
    }
}

fn format_enum(out: &mut String, name: &str, html: &str) {
    let decl = extract_item_decl(html);
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
    for m in extract_own_methods(html) {
        if m.len() > 3 {
            writeln!(out, "  {m}").unwrap();
        }
    }
}

fn format_trait(out: &mut String, name: &str, html: &str) {
    let decl = extract_item_decl(html);
    writeln!(out, "trait {}", simplify_trait_decl(&decl, name)).unwrap();
    for m in extract_trait_methods(&decl) {
        writeln!(out, "  {m}").unwrap();
    }
}

fn format_fn(out: &mut String, _name: &str, html: &str) {
    let sig = simplify_fn_decl(&extract_item_decl(html));
    if !sig.is_empty() {
        writeln!(out, "{sig}").unwrap();
    }
}

fn format_type_alias(out: &mut String, _name: &str, html: &str) {
    let clean = extract_item_decl(html).trim().to_string();
    if !clean.is_empty() {
        writeln!(out, "{clean}").unwrap();
    }
}

fn format_constant(out: &mut String, _name: &str, html: &str) {
    let clean = extract_item_decl(html).trim().to_string();
    if !clean.is_empty() {
        let trimmed = if let Some(pos) = clean.find(" = ") {
            format!("{};", &clean[..pos])
        } else {
            clean
        };
        writeln!(out, "{trimmed}").unwrap();
    }
}

// ── HTML extraction helpers ──────────────────────────────────────────

fn extract_item_decl(html: &str) -> String {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("pre.rust.item-decl").unwrap();
    doc.select(&sel)
        .next()
        .map(|el| decode_html_entities(&el.text().collect::<String>()))
        .unwrap_or_default()
}

fn extract_own_methods(html: &str) -> Vec<String> {
    let doc = Html::parse_document(html);
    let mut methods = Vec::new();
    let impl_sel = Selector::parse("#implementations-list").unwrap();
    let header_sel = Selector::parse("h4.code-header").unwrap();

    if let Some(impl_list) = doc.select(&impl_sel).next() {
        for header in impl_list.select(&header_sel) {
            let sig = simplify_method_sig(&decode_html_entities(
                &header.text().collect::<String>(),
            ));
            if !sig.is_empty() {
                methods.push(sig);
            }
        }
    }
    methods
}

fn extract_enum_variants(decl: &str) -> Vec<String> {
    let mut variants = Vec::new();
    let start = match decl.find('{') {
        Some(p) => p + 1,
        None => return variants,
    };
    let end = match decl.rfind('}') {
        Some(p) => p,
        None => return variants,
    };
    for line in decl[start..end].lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") {
            continue;
        }
        let name = trimmed.split(['(', '{', ',']).next().unwrap_or("").trim();
        if !name.is_empty() && name.chars().next().is_some_and(|c| c.is_uppercase()) {
            variants.push(name.to_string());
        }
    }
    variants
}

fn extract_trait_methods(decl: &str) -> Vec<String> {
    let mut methods = Vec::new();
    for line in decl.lines() {
        let t = line.trim();
        if t.starts_with("fn ") || t.starts_with("async fn ") {
            let sig = simplify_method_sig(&format!("pub {t}"));
            if !sig.is_empty() {
                methods.push(sig);
            }
        }
    }
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
        let trimmed = line.trim();
        if !in_fn && trimmed.starts_with("fn ") {
            in_fn = true;
            current_fn = trimmed.to_string();
            brace_depth = trimmed.matches('{').count();
            brace_depth = brace_depth.saturating_sub(trimmed.matches('}').count());
            if trimmed.ends_with("{ ... }") || trimmed.ends_with(';') {
                let sig = desugar_async_fn(&current_fn);
                if !sig.is_empty() {
                    methods.push(sig);
                }
                in_fn = false;
                current_fn.clear();
            }
        } else if in_fn {
            current_fn.push(' ');
            current_fn.push_str(trimmed);
            brace_depth += trimmed.matches('{').count();
            brace_depth = brace_depth.saturating_sub(trimmed.matches('}').count());
            if trimmed.ends_with("{ ... }") || trimmed.ends_with(';') || brace_depth == 0 {
                let sig = desugar_async_fn(&current_fn);
                if !sig.is_empty() {
                    methods.push(sig);
                }
                in_fn = false;
                current_fn.clear();
            }
        }
    }
    methods
}

// ── Signature simplification ─────────────────────────────────────────

fn desugar_async_fn(sig: &str) -> String {
    let is_async = sig.contains("Pin<Box<dyn Future<Output =");
    let fn_start = match sig.find("fn ") {
        Some(p) => p,
        None => return String::new(),
    };
    let after_fn = &sig[fn_start + 3..];
    let name_end = after_fn.find(['<', '(']).unwrap_or(after_fn.len());
    let name = after_fn[..name_end].trim();
    let clean_params = clean_params(&extract_params(sig));

    if is_async {
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
    }
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
    if let Some(colon_pos) = r.find(':') {
        let name = r[..colon_pos].trim();
        if name.starts_with('_') && name.len() > 1 {
            r = format!("{}: {}", &name[1..], r[colon_pos + 1..].trim());
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

fn simplify_method_sig(sig: &str) -> String {
    let s = sig.trim().strip_prefix("pub ").unwrap_or(sig.trim());
    let clean = strip_where_clause(s)
        .replace("{ … }", "")
        .replace("{ ... }", "");
    strip_where_from_sig(&collapse_whitespace(clean.trim().trim_end_matches(';').trim()))
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
                        let p = clean_supertrait_part(p);
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
                let p = clean_supertrait_part(t.trim_start_matches('+'));
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
        .filter(|s| !matches!(s.as_str(), "Send" | "Sync" | "'static" | "Sized" | "'static {"))
        .collect();
    if supers.is_empty() {
        name.to_string()
    } else {
        format!("{name}: {}", supers.join(" + "))
    }
}

fn clean_supertrait_part(part: &str) -> &str {
    part.split("where")
        .next()
        .unwrap_or(part)
        .trim()
        .trim_end_matches('{')
        .trim()
}

fn simplify_fn_decl(decl: &str) -> String {
    let s = decl.trim().strip_prefix("pub ").unwrap_or(decl.trim());
    let clean = strip_where_clause(s).trim().trim_end_matches(';').trim().to_string();
    let collapsed = collapse_whitespace(&clean);
    if collapsed.starts_with("async fn ") {
        return collapsed;
    }
    let fn_part = collapsed.strip_prefix("fn ").unwrap_or(&collapsed);
    format!("fn {fn_part}")
}

fn strip_where_clause(sig: &str) -> String {
    let mut depth: usize = 0;
    let mut paren_depth: usize = 0;
    let mut past_params = false;
    for (i, c) in sig.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                if paren_depth == 0 {
                    past_params = true;
                }
            }
            'w' if depth == 0
                && paren_depth == 0
                && past_params
                && sig[i..].starts_with("where")
                && sig[i + "where".len()..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace) =>
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

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}
