//! TypeScript type auto-export.
//!
//! Types that derive `ApiSchema`, `AppEnum`, or `foundry::TS` are automatically
//! registered for TypeScript export via the `inventory` crate.
//!
//! `AppEnum` types also export runtime metadata:
//! ```ts
//! export type CountryStatus = "enabled" | "disabled";
//! export const CountryStatusValues = ["enabled", "disabled"] as const;
//! export const CountryStatusOptions = [
//!   { value: "enabled", labelKey: "enum.country_status.enabled" },
//!   { value: "disabled", labelKey: "enum.country_status.disabled" },
//! ] as const;
//! ```

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::app_enum::{EnumKey, EnumKeyKind, EnumMeta};
use crate::cli::CommandRegistrar;
use crate::foundation::{Error, Result};
use crate::http::{HttpRegistrar, RouteManifestEntry, RouteRegistrar};
use crate::support::generated_manifest::{
    clean_manifest_files as clean_generated_manifest_files, safe_manifest_path_with_extension,
    write_generated_file, write_manifest,
};
use crate::support::CommandId;

const TYPES_EXPORT_COMMAND: CommandId = CommandId::new("types:export");
const TYPES_EXPORT_MANIFEST: &str = ".foundry-types-manifest.json";

/// A registered TypeScript type exporter.
pub struct TsType {
    pub name: &'static str,
    pub export_fn: fn(&Path) -> std::result::Result<(), ts_rs::ExportError>,
    pub output_path_fn: fn() -> Option<&'static Path>,
}

inventory::collect!(TsType);

/// A registered AppEnum with runtime metadata for TypeScript export.
pub struct TsAppEnum {
    pub name: &'static str,
    pub meta_fn: fn() -> EnumMeta,
}

inventory::collect!(TsAppEnum);

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("string literal serialization should not fail")
}

fn enum_key_kind_literal(kind: EnumKeyKind) -> &'static str {
    match kind {
        EnumKeyKind::String => "string",
        EnumKeyKind::Int => "int",
    }
}

fn enum_key_literal(value: &EnumKey) -> String {
    match value {
        EnumKey::String(value) => json_string(value),
        EnumKey::Int(value) => value.to_string(),
    }
}

fn render_array(items: &[String]) -> String {
    if items.is_empty() {
        "[]".to_string()
    } else {
        format!("[\n  {},\n]", items.join(",\n  "))
    }
}

#[derive(Debug)]
struct RenderedAppEnum {
    content: String,
    has_groups: bool,
}

struct EnumGroup {
    property: String,
    actions: Vec<EnumGroupAction>,
}

struct EnumGroupAction {
    property: String,
    value: String,
}

fn is_ts_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if !(first == '_' || first == '$' || first.is_ascii_alphabetic()) {
        return false;
    }

    chars.all(|ch| ch == '_' || ch == '$' || ch.is_ascii_alphanumeric())
}

fn to_camel_case_identifier_with_context(value: &str, context: &str) -> Result<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch);
        } else if matches!(ch, '_' | '-' | ' ') {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            return Err(Error::message(format!(
                "{context} only supports ASCII property keys; `{value}` contains unsupported character `{ch}`"
            )));
        }
    }

    if !current.is_empty() {
        words.push(current);
    }

    if words.is_empty() {
        return Err(Error::message(format!(
            "{context} requires non-empty property keys"
        )));
    }

    let mut identifier = words[0].to_ascii_lowercase();
    for word in words.iter().skip(1) {
        let lower = word.to_ascii_lowercase();
        let mut chars = lower.chars();
        if let Some(first) = chars.next() {
            identifier.push(first.to_ascii_uppercase());
            identifier.push_str(chars.as_str());
        }
    }

    if !is_ts_identifier(&identifier) {
        return Err(Error::message(format!(
            "{context} normalized `{value}` to invalid TypeScript identifier `{identifier}`"
        )));
    }

    Ok(identifier)
}

fn to_camel_case_identifier(value: &str) -> Result<String> {
    to_camel_case_identifier_with_context(value, "AppEnum grouped TypeScript export")
}

fn parse_grouped_key<'a>(name: &str, key: &'a str) -> Result<Option<(&'a str, &'a str)>> {
    let mut parts = key.split('.');
    let module = parts.next().unwrap_or_default();
    let Some(action) = parts.next() else {
        return Ok(None);
    };

    if parts.next().is_some() || module.is_empty() || action.is_empty() {
        return Err(Error::message(format!(
            "AppEnum `{name}` grouped TypeScript export expects keys shaped `<module>.<action>`; got `{key}`"
        )));
    }

    if !is_ts_identifier(action) {
        return Err(Error::message(format!(
            "AppEnum `{name}` grouped TypeScript export requires action `{action}` from key `{key}` to be a TypeScript identifier"
        )));
    }

    Ok(Some((module, action)))
}

fn app_enum_groups(name: &str, meta: &EnumMeta) -> Result<Option<Vec<EnumGroup>>> {
    if meta.key_kind != EnumKeyKind::String {
        return Ok(None);
    }

    let mut saw_grouped = false;
    let mut saw_plain = false;
    let mut groups = Vec::<EnumGroup>::new();
    let mut module_properties = HashMap::<String, String>::new();

    for option in meta.options.iter() {
        let EnumKey::String(value) = &option.value else {
            continue;
        };

        let Some((module, action)) = parse_grouped_key(name, value)? else {
            saw_plain = true;
            continue;
        };

        saw_grouped = true;
        let module_property = to_camel_case_identifier(module)?;

        if let Some(existing) = module_properties.get(&module_property) {
            if existing != module {
                return Err(Error::message(format!(
                    "AppEnum `{name}` grouped TypeScript export has module keys `{existing}` and `{module}` that both normalize to `{module_property}`"
                )));
            }
        } else {
            module_properties.insert(module_property.clone(), module.to_string());
        }

        let group = if let Some(index) = groups
            .iter()
            .position(|group| group.property == module_property)
        {
            &mut groups[index]
        } else {
            groups.push(EnumGroup {
                property: module_property.clone(),
                actions: Vec::new(),
            });
            groups.last_mut().expect("just pushed group")
        };

        if group.actions.iter().any(|entry| entry.property == action) {
            return Err(Error::message(format!(
                "AppEnum `{name}` grouped TypeScript export has duplicate action `{action}` in module `{module}`"
            )));
        }

        group.actions.push(EnumGroupAction {
            property: action.to_string(),
            value: value.clone(),
        });
    }

    if saw_grouped && saw_plain {
        return Err(Error::message(format!(
            "AppEnum `{name}` grouped TypeScript export mixes dotted `<module>.<action>` keys with non-dotted keys"
        )));
    }

    if saw_grouped {
        Ok(Some(groups))
    } else {
        Ok(None)
    }
}

fn render_groups(name: &str, groups: &[EnumGroup]) -> String {
    let group_literals: Vec<String> = groups
        .iter()
        .map(|group| {
            let action_literals: Vec<String> = group
                .actions
                .iter()
                .map(|action| format!("{}: {}", action.property, json_string(&action.value)))
                .collect();

            format!("  {}: {{ {} }}", group.property, action_literals.join(", "))
        })
        .collect();

    format!(
        "\nexport const {name}Groups = {{\n{},\n}} as const;\n",
        group_literals.join(",\n")
    )
}

fn render_app_enum(name: &str, meta: &EnumMeta) -> Result<RenderedAppEnum> {
    let value_literals: Vec<String> = meta
        .options
        .iter()
        .map(|option| enum_key_literal(&option.value))
        .collect();
    let type_union = if value_literals.is_empty() {
        "never".to_string()
    } else {
        value_literals.join(" | ")
    };
    let option_literals: Vec<String> = meta
        .options
        .iter()
        .map(|option| {
            format!(
                "{{ value: {}, labelKey: {} }}",
                enum_key_literal(&option.value),
                json_string(&option.label_key),
            )
        })
        .collect();

    let groups = app_enum_groups(name, meta)?;
    let mut content = format!(
        "// Auto-generated from AppEnum. Do not edit.\n\n\
         export type {name} = {type_union};\n\n\
         export const {name}Values = {} as const;\n\n\
         export const {name}Options = {} as const;\n\n\
         export const {name}Meta = {{\n\
           id: {},\n\
           keyKind: {},\n\
           options: {name}Options,\n\
         }} as const;\n",
        render_array(&value_literals),
        render_array(&option_literals),
        json_string(&meta.id),
        json_string(enum_key_kind_literal(meta.key_kind)),
    );

    if let Some(groups) = &groups {
        content.push_str(&render_groups(name, groups));
    }

    Ok(RenderedAppEnum {
        content,
        has_groups: groups.is_some(),
    })
}

#[derive(Default)]
struct RouteIdTreeNode {
    value: Option<String>,
    children: BTreeMap<String, RouteIdTreeChild>,
}

struct RouteIdTreeChild {
    segment: String,
    node: RouteIdTreeNode,
}

fn option_string_literal(value: Option<&str>) -> String {
    value.map(json_string).unwrap_or_else(|| "null".to_string())
}

fn string_array_literal<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let values = values.into_iter().map(json_string).collect::<Vec<_>>();
    format!("[{}]", values.join(", "))
}

fn first_route_id(node: &RouteIdTreeNode) -> Option<&str> {
    node.value.as_deref().or_else(|| {
        node.children
            .values()
            .find_map(|child| first_route_id(&child.node))
    })
}

fn insert_route_id_node(
    node: &mut RouteIdTreeNode,
    route_id: &str,
    segments: &[&str],
) -> Result<()> {
    let Some((segment, remaining)) = segments.split_first() else {
        if let Some(existing) = &node.value {
            return Err(Error::message(format!(
                "RouteManifest TypeScript export contains duplicate route id `{existing}`"
            )));
        }
        if let Some(existing) = first_route_id(node) {
            return Err(Error::message(format!(
                "RouteManifest TypeScript export cannot group route id `{route_id}` because it is also a prefix of `{existing}`"
            )));
        }

        node.value = Some(route_id.to_string());
        return Ok(());
    };

    if let Some(existing) = &node.value {
        return Err(Error::message(format!(
            "RouteManifest TypeScript export cannot group route id `{route_id}` because `{existing}` is already a route id"
        )));
    }
    if segment.is_empty() {
        return Err(Error::message(format!(
            "RouteManifest TypeScript export requires non-empty route id segments; got `{route_id}`"
        )));
    }

    let property =
        to_camel_case_identifier_with_context(segment, "RouteManifest TypeScript export")?;
    if let Some(child) = node.children.get_mut(&property) {
        if child.segment != *segment {
            return Err(Error::message(format!(
                "RouteManifest TypeScript export has route id segments `{}` and `{segment}` that both normalize to `{property}`",
                child.segment
            )));
        }

        return insert_route_id_node(&mut child.node, route_id, remaining);
    }

    node.children.insert(
        property.clone(),
        RouteIdTreeChild {
            segment: (*segment).to_string(),
            node: RouteIdTreeNode::default(),
        },
    );
    let child = node
        .children
        .get_mut(&property)
        .expect("just inserted route id segment");
    insert_route_id_node(&mut child.node, route_id, remaining)
}

fn route_id_tree(routes: &[RouteManifestEntry]) -> Result<RouteIdTreeNode> {
    let mut root = RouteIdTreeNode::default();

    for route in routes {
        let route_id = route.id.as_str();
        let segments = route_id.split('.').collect::<Vec<_>>();
        insert_route_id_node(&mut root, route_id, &segments)?;
    }

    Ok(root)
}

fn render_route_id_node(node: &RouteIdTreeNode, indent: usize) -> String {
    if node.children.is_empty() {
        return "{}".to_string();
    }

    let entry_indent = " ".repeat(indent + 2);
    let closing_indent = " ".repeat(indent);
    let entries = node
        .children
        .iter()
        .map(|(property, child)| {
            if child.node.children.is_empty() {
                let route_id = child
                    .node
                    .value
                    .as_ref()
                    .expect("leaf route id should have a value");
                format!("{entry_indent}{property}: {}", json_string(route_id))
            } else {
                format!(
                    "{entry_indent}{property}: {}",
                    render_route_id_node(&child.node, indent + 2)
                )
            }
        })
        .collect::<Vec<_>>();

    format!("{{\n{},\n{closing_indent}}}", entries.join(",\n"))
}

fn render_route_ids(routes: &[RouteManifestEntry]) -> Result<String> {
    let tree = route_id_tree(routes)?;
    Ok(render_route_id_node(&tree, 0))
}

fn ensure_unique_route_manifest(routes: &[RouteManifestEntry]) -> Result<()> {
    let mut route_ids = HashSet::new();
    for route in routes {
        if !route_ids.insert(route.id.as_str()) {
            return Err(Error::message(format!(
                "RouteManifest TypeScript export contains duplicate route id `{}`",
                route.id.as_str()
            )));
        }
    }
    Ok(())
}

fn render_route_manifest(routes: &[RouteManifestEntry]) -> Result<String> {
    ensure_unique_route_manifest(routes)?;
    let route_ids = render_route_ids(routes)?;

    let route_literals = routes
        .iter()
        .map(|route| {
            let params = string_array_literal(route.params.iter().map(String::as_str));
            let permissions =
                string_array_literal(route.permissions.iter().map(|permission| permission.as_str()));
            let responses = route
                .responses
                .iter()
                .map(|response| {
                    format!(
                        "{{ status: {}, schema: {} }}",
                        response.status,
                        json_string(response.schema)
                    )
                })
                .collect::<Vec<_>>();

            format!(
                "  {}: {{ id: {}, path: {}, method: {}, params: {}, guard: {}, permissions: {}, summary: {}, request: {}, responses: [{}] }}",
                json_string(route.id.as_str()),
                json_string(route.id.as_str()),
                json_string(&route.path),
                option_string_literal(route.method.as_deref()),
                params,
                option_string_literal(route.guard.as_ref().map(|guard| guard.as_str())),
                permissions,
                option_string_literal(route.summary.as_deref()),
                option_string_literal(route.request),
                responses.join(", "),
            )
        })
        .collect::<Vec<_>>();

    let route_params = if routes.is_empty() {
        "export type RouteParams = Record<RouteName, Record<never, never>>;\n".to_string()
    } else {
        let entries = routes
            .iter()
            .map(|route| {
                if route.params.is_empty() {
                    format!(
                        "  {}: Record<never, never>;",
                        json_string(route.id.as_str())
                    )
                } else {
                    let fields = route
                        .params
                        .iter()
                        .map(|param| format!("{}: RouteParamValue", json_string(param)))
                        .collect::<Vec<_>>();
                    format!(
                        "  {}: {{ {} }};",
                        json_string(route.id.as_str()),
                        fields.join("; ")
                    )
                }
            })
            .collect::<Vec<_>>();
        format!(
            "export type RouteParams = {{\n{}\n}};\n",
            entries.join("\n")
        )
    };

    Ok(format!(
        "// Auto-generated from Foundry routes. Do not edit.\n\n\
         export type RouteParamValue = string | number | boolean;\n\
         export type RouteUrlOptions = {{ basePath?: string }};\n\
         type RouteManifestRuntimeEntry = {{ readonly path: string; readonly params: readonly string[] }};\n\n\
         export const RouteManifest = {{\n{}\n\
         }} as const;\n\n\
         export const RouteIds = {} as const;\n\n\
         export type RouteName = keyof typeof RouteManifest;\n\
         {}\n\
         type RouteArgs<Name extends RouteName> = Name extends RouteName\n\
           ? keyof RouteParams[Name] extends never\n\
             ? [params?: RouteParams[Name], options?: RouteUrlOptions]\n\
             : [params: RouteParams[Name], options?: RouteUrlOptions]\n\
           : never;\n\n\
         function replaceAll(input: string, search: string, value: string): string {{\n\
           return input.split(search).join(value);\n\
         }}\n\n\
         function normalizeBasePath(basePath: string | undefined): string {{\n\
           if (!basePath || basePath === \"/\") {{\n\
             return \"\";\n\
           }}\n\
           const normalized = basePath.startsWith(\"/\") ? basePath : `/${{basePath}}`;\n\
           return normalized.replace(/\\/+$/, \"\");\n\
         }}\n\n\
         function stripBasePath(path: string, basePath: string | undefined): string {{\n\
           const normalized = normalizeBasePath(basePath);\n\
           if (!normalized) {{\n\
             return path;\n\
           }}\n\
           if (path === normalized) {{\n\
             return \"/\";\n\
           }}\n\
           if (path.startsWith(`${{normalized}}/`)) {{\n\
             return path.slice(normalized.length);\n\
           }}\n\
           return path;\n\
         }}\n\n\
         function substituteRouteParams(\n\
           name: RouteName,\n\
           entry: RouteManifestRuntimeEntry,\n\
           params: Record<string, RouteParamValue>,\n\
         ): string {{\n\
           let path = entry.path;\n\
           for (const param of entry.params) {{\n\
             if (!Object.prototype.hasOwnProperty.call(params, param)) {{\n\
               throw new Error(`Route ${{String(name)}} is missing required parameter ${{param}}`);\n\
             }}\n\
             const value = encodeURIComponent(String(params[param]));\n\
             path = replaceAll(path, `{{${{param}}}}`, value);\n\
             path = replaceAll(path, `{{*${{param}}}}`, value);\n\
             path = replaceAll(path, `:${{param}}`, value);\n\
           }}\n\
           return path;\n\
         }}\n\n\
         function resolveRouteUrl(\n\
           name: RouteName,\n\
           params: Record<string, RouteParamValue>,\n\
           options: RouteUrlOptions,\n\
         ): string {{\n\
           const entry = RouteManifest[name] as RouteManifestRuntimeEntry | undefined;\n\
           if (!entry) {{\n\
             throw new Error(`Unknown route ${{String(name)}}`);\n\
           }}\n\
           return stripBasePath(substituteRouteParams(name, entry, params), options.basePath);\n\
         }}\n\n\
         export function routeUrl<Name extends RouteName>(\n\
           name: Name,\n\
           ...args: RouteArgs<Name>\n\
         ): string {{\n\
           const params = (args[0] ?? {{}}) as Record<string, RouteParamValue>;\n\
           const options = (args[1] ?? {{}}) as RouteUrlOptions;\n\
           return resolveRouteUrl(name, params, options);\n\
         }}\n\n\
         export function createRouteUrlBuilder(options: RouteUrlOptions) {{\n\
           return function buildRouteUrl<Name extends RouteName>(\n\
             name: Name,\n\
             ...args: RouteArgs<Name>\n\
           ): string {{\n\
             const params = (args[0] ?? {{}}) as Record<string, RouteParamValue>;\n\
             const routeOptions = (args[1] ?? {{}}) as RouteUrlOptions;\n\
             return resolveRouteUrl(name, params, {{ ...options, ...routeOptions }});\n\
           }};\n\
         }}\n",
        route_literals.join(",\n"),
        route_ids,
        route_params,
    ))
}

/// Export all registered TypeScript types to a directory.
pub fn export_all(dir: &Path) -> Result<()> {
    export_all_with_routes(dir, &[])
}

/// Export all registered TypeScript types and HTTP route metadata to a directory.
pub fn export_all_with_routes(dir: &Path, routes: &[RouteManifestEntry]) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(Error::other)?;

    let ts_types: Vec<&TsType> = inventory::iter::<TsType>.into_iter().collect();
    let app_enums: Vec<&TsAppEnum> = inventory::iter::<TsAppEnum>.into_iter().collect();
    let ts_type_files = planned_ts_type_files(&ts_types)?;
    let output_files = planned_output_files(&ts_type_files, &app_enums);
    clean_manifest_files(dir, &output_files)?;

    let mut type_exports = BTreeMap::new();
    for ts_type in ts_types {
        (ts_type.export_fn)(dir)
            .map_err(|e| Error::message(format!("ts export `{}`: {e}", ts_type.name)))?;
        let file = ts_type_files
            .get(ts_type.name)
            .expect("planned TypeScript file should exist")
            .clone();
        type_exports.insert(ts_type.name, file);
    }

    // Rewrite AppEnum files entirely — if ts-rs also emitted an enum file,
    // the metadata-based AppEnum export owns the final file content.
    let mut enum_names = HashSet::new();
    let mut grouped_enum_names = HashSet::new();
    for app_enum in app_enums {
        let file_path = dir.join(format!("{}.ts", app_enum.name));
        let rendered = render_app_enum(app_enum.name, &(app_enum.meta_fn)())?;
        if rendered.has_groups {
            grouped_enum_names.insert(app_enum.name);
        }
        write_generated_file(&file_path, rendered.content)?;
        enum_names.insert(app_enum.name);
    }

    let mut names: Vec<&str> = type_exports
        .keys()
        .copied()
        .chain(enum_names.iter().copied())
        .collect();
    names.sort();
    names.dedup();

    write_generated_file(
        &dir.join("RouteManifest.ts"),
        render_route_manifest(routes)?,
    )?;

    let mut barrel = String::from("// Auto-generated barrel. Do not edit.\n");
    for name in &names {
        if enum_names.contains(name) {
            let groups_export = if grouped_enum_names.contains(name) {
                format!(", {name}Groups")
            } else {
                String::new()
            };
            barrel.push_str(&format!(
                "export {{ type {name}, {name}Values, {name}Options, {name}Meta{groups_export} }} from \"./{name}\";\n"
            ));
        } else {
            let file = type_exports
                .get(name)
                .expect("planned TypeScript export should exist");
            let module = ts_module_specifier(file)?;
            barrel.push_str(&format!("export type {{ {name} }} from \"{module}\";\n"));
        }
    }
    barrel.push_str(
        "export { RouteManifest, RouteIds, createRouteUrlBuilder, routeUrl, type RouteName, type RouteParams, type RouteParamValue, type RouteUrlOptions } from \"./RouteManifest\";\n",
    );
    write_generated_file(&dir.join("index.ts"), barrel)?;
    write_export_manifest(dir, &output_files)?;

    println!("Exported {} type(s) to {}", names.len(), dir.display());

    Ok(())
}

fn planned_ts_type_files(ts_types: &[&TsType]) -> Result<BTreeMap<&'static str, String>> {
    let mut files = BTreeMap::new();
    for ts_type in ts_types {
        let file = ts_type_output_file(ts_type)?;
        if let Some(existing) = files.insert(ts_type.name, file.clone()) {
            if existing != file {
                return Err(Error::message(format!(
                    "TypeScript export `{}` registered multiple output paths: `{existing}` and `{file}`",
                    ts_type.name
                )));
            }
        }
    }
    Ok(files)
}

fn planned_output_files(
    ts_type_files: &BTreeMap<&'static str, String>,
    app_enums: &[&TsAppEnum],
) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    for file in ts_type_files.values() {
        files.insert(file.clone());
    }
    for app_enum in app_enums {
        files.insert(format!("{}.ts", app_enum.name));
    }
    files.insert("RouteManifest.ts".to_string());
    files.insert("index.ts".to_string());
    files
}

fn ts_type_output_file(ts_type: &TsType) -> Result<String> {
    let path = (ts_type.output_path_fn)().ok_or_else(|| {
        Error::message(format!(
            "TypeScript export `{}` does not expose an output path",
            ts_type.name
        ))
    })?;
    let file = path.to_str().ok_or_else(|| {
        Error::message(format!(
            "TypeScript export `{}` output path must be valid UTF-8",
            ts_type.name
        ))
    })?;

    let Some(path) = safe_manifest_path_with_extension(file, "ts", true) else {
        return Err(Error::message(format!(
            "TypeScript export `{}` output path `{file}` must be a relative .ts path inside the generated types directory",
            ts_type.name
        )));
    };

    Ok(path.to_string_lossy().replace('\\', "/"))
}

fn ts_module_specifier(file: &str) -> Result<String> {
    let module = file.strip_suffix(".ts").ok_or_else(|| {
        Error::message(format!(
            "TypeScript barrel export path `{file}` must end with .ts"
        ))
    })?;
    Ok(format!("./{module}"))
}

fn clean_manifest_files(dir: &Path, output_files: &BTreeSet<String>) -> Result<()> {
    clean_generated_manifest_files(
        dir,
        TYPES_EXPORT_MANIFEST,
        output_files,
        "foundry.typescript",
        |file| safe_manifest_path_with_extension(file, "ts", true),
    )
}

fn write_export_manifest(dir: &Path, output_files: &BTreeSet<String>) -> Result<()> {
    write_manifest(dir, TYPES_EXPORT_MANIFEST, output_files)
}

fn collect_route_manifest(routes: &[RouteRegistrar]) -> Result<Vec<RouteManifestEntry>> {
    let mut registrar = HttpRegistrar::new();
    for route in routes {
        route(&mut registrar)?;
    }
    registrar.collect_route_manifest()
}

/// CLI registrar for the `types:export` command.
pub fn builtin_cli_registrar(routes: Vec<RouteRegistrar>) -> CommandRegistrar {
    Arc::new(move |registry| {
        let routes = routes.clone();
        registry.command(
            TYPES_EXPORT_COMMAND,
            clap::Command::new("types:export")
                .about("Export registered TypeScript types")
                .arg(
                    clap::Arg::new("output")
                        .long("output")
                        .short('o')
                        .help("Output directory (overrides config)"),
                ),
            move |invocation| {
                let routes = routes.clone();
                async move {
                    let output = if let Some(dir) = invocation.matches().get_one::<String>("output")
                    {
                        PathBuf::from(dir)
                    } else {
                        let config = invocation.app().config().typescript().unwrap_or_default();
                        PathBuf::from(config.output_dir)
                    };

                    let route_manifest = collect_route_manifest(&routes)?;
                    export_all_with_routes(&output, &route_manifest)
                }
            },
        )?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::app_enum::{EnumKey, EnumKeyKind, EnumMeta, EnumOption};
    use crate::http::{RouteManifestEntry, RouteManifestResponse};
    use crate::support::{Collection, GuardId, PermissionId, RouteId};

    use super::export_all;
    use super::export_all_with_routes;
    use super::render_app_enum;
    use super::render_route_manifest;
    use super::TYPES_EXPORT_MANIFEST;

    #[derive(Clone, Debug, PartialEq, Eq, crate::AppEnum)]
    enum MinimalExportStatus {
        Pending,
        Completed,
    }

    #[derive(Clone, Debug, PartialEq, Eq, crate::AppEnum)]
    enum MinimalExportPriority {
        Low = 1,
        High = 2,
    }

    #[derive(Clone, Debug, PartialEq, Eq, crate::AppEnum)]
    enum MinimalExportPermission {
        #[foundry(key = "audit_logs.read")]
        AuditLogsRead,
        #[foundry(key = "audit_logs.manage")]
        AuditLogsManage,
        #[foundry(key = "observability.view")]
        ObservabilityView,
    }

    #[allow(dead_code)]
    #[derive(Clone, Debug, serde::Serialize, ts_rs::TS, crate::ApiSchema)]
    struct MinimalExportAppEnumDto {
        status: MinimalExportStatus,
        priority: Option<MinimalExportPriority>,
        permissions: Vec<MinimalExportPermission>,
    }

    #[allow(dead_code)]
    #[derive(Clone, Debug, serde::Serialize, ts_rs::TS, crate::TS)]
    #[ts(export_to = "custom/CustomExportPathDto.ts")]
    struct CustomExportPathDto {
        value: String,
    }

    fn string_meta(values: &[&str]) -> EnumMeta {
        EnumMeta {
            id: "permission".to_string(),
            key_kind: EnumKeyKind::String,
            options: Collection::from(
                values
                    .iter()
                    .map(|value| EnumOption {
                        value: EnumKey::String((*value).to_string()),
                        label_key: format!("enum.permission.{value}"),
                    })
                    .collect::<Vec<_>>(),
            ),
        }
    }

    fn route_manifest_entry(id: &'static str, path: &str, params: &[&str]) -> RouteManifestEntry {
        RouteManifestEntry {
            id: RouteId::new(id),
            path: path.to_string(),
            method: Some("get".to_string()),
            params: params.iter().map(|param| (*param).to_string()).collect(),
            guard: Some(GuardId::new("admin")),
            permissions: vec![PermissionId::new("users.read")],
            summary: Some("Show user".to_string()),
            request: Some("ShowUserRequest"),
            responses: vec![RouteManifestResponse {
                status: 200,
                schema: "ShowUserResponse",
            }],
        }
    }

    #[test]
    fn exports_framework_typescript_helpers() {
        let dir = tempdir().unwrap();
        export_all(dir.path()).unwrap();

        for file in [
            "DatatableFilterBinding.ts",
            "DatatableFilterField.ts",
            "DatatableFilterValueKind.ts",
            "DatatableJsonResponse.ts",
            "DatatableRequest.ts",
            "MessageResponse.ts",
            "RefreshTokenRequest.ts",
            "TokenPair.ts",
            "TokenResponse.ts",
            "WsTokenResponse.ts",
            "RouteManifest.ts",
            "MinimalExportAppEnumDto.ts",
        ] {
            assert!(
                dir.path().join(file).exists(),
                "expected generated TypeScript file: {file}"
            );
        }
        assert!(
            dir.path().join("custom/CustomExportPathDto.ts").exists(),
            "expected custom export_to TypeScript file"
        );

        let datatable_filter_field =
            fs::read_to_string(dir.path().join("DatatableFilterField.ts")).unwrap();
        assert!(
            datatable_filter_field.contains("import type { DatatableFilterBinding } from \"./DatatableFilterBinding\";"),
            "expected DatatableFilterField.ts to import DatatableFilterBinding:\n{datatable_filter_field}"
        );
        assert!(
            datatable_filter_field.contains("import type { DatatableFilterOptions } from \"./DatatableFilterOptions\";"),
            "expected DatatableFilterField.ts to import DatatableFilterOptions:\n{datatable_filter_field}"
        );
        assert!(
            datatable_filter_field.contains("binding: DatatableFilterBinding"),
            "expected DatatableFilterField.ts to expose binding metadata:\n{datatable_filter_field}"
        );

        let datatable_filter_options =
            fs::read_to_string(dir.path().join("DatatableFilterOptions.ts")).unwrap();
        assert!(
            datatable_filter_options
                .contains("import type { DatatableFilterOption } from \"./DatatableFilterOption\";"),
            "expected DatatableFilterOptions.ts to import DatatableFilterOption:\n{datatable_filter_options}"
        );

        let datatable_filter_binding =
            fs::read_to_string(dir.path().join("DatatableFilterBinding.ts")).unwrap();
        assert!(
            datatable_filter_binding
                .contains("import type { DatatableFilterOp } from \"./DatatableFilterOp\";"),
            "expected DatatableFilterBinding.ts to import DatatableFilterOp:\n{datatable_filter_binding}"
        );
        assert!(
            datatable_filter_binding.contains(
                "import type { DatatableFilterValueKind } from \"./DatatableFilterValueKind\";"
            ),
            "expected DatatableFilterBinding.ts to import DatatableFilterValueKind:\n{datatable_filter_binding}"
        );
        assert!(
            datatable_filter_binding.contains("value_kind: DatatableFilterValueKind"),
            "expected DatatableFilterBinding.ts to expose value_kind:\n{datatable_filter_binding}"
        );

        let datatable_filter_kind =
            fs::read_to_string(dir.path().join("DatatableFilterKind.ts")).unwrap();
        assert!(
            datatable_filter_kind.contains("\"number\""),
            "expected DatatableFilterKind.ts to include number:\n{datatable_filter_kind}"
        );

        let datatable_filter_value_kind =
            fs::read_to_string(dir.path().join("DatatableFilterValueKind.ts")).unwrap();
        assert!(
            datatable_filter_value_kind.contains("\"decimal\""),
            "expected DatatableFilterValueKind.ts to include decimal:\n{datatable_filter_value_kind}"
        );

        let datatable_request = fs::read_to_string(dir.path().join("DatatableRequest.ts")).unwrap();
        assert!(
            datatable_request.contains("page: number"),
            "expected DatatableRequest.ts page field to use number:\n{datatable_request}"
        );
        assert!(
            datatable_request.contains("per_page: number"),
            "expected DatatableRequest.ts per_page field to use number:\n{datatable_request}"
        );
        assert!(
            !datatable_request.contains("bigint"),
            "did not expect bigint in DatatableRequest.ts:\n{datatable_request}"
        );

        let datatable_filter_value =
            fs::read_to_string(dir.path().join("DatatableFilterValue.ts")).unwrap();
        assert!(
            datatable_filter_value.contains("{ \"number\": number }"),
            "expected DatatableFilterValue::Number to use number:\n{datatable_filter_value}"
        );
        assert!(
            !datatable_filter_value.contains("bigint"),
            "did not expect bigint in DatatableFilterValue.ts:\n{datatable_filter_value}"
        );

        let datatable_json_response =
            fs::read_to_string(dir.path().join("DatatableJsonResponse.ts")).unwrap();
        assert!(
            datatable_json_response.contains("DatatablePaginationMeta"),
            "expected DatatableJsonResponse.ts to reference pagination metadata:\n{datatable_json_response}"
        );

        let datatable_pagination_meta =
            fs::read_to_string(dir.path().join("DatatablePaginationMeta.ts")).unwrap();
        assert!(
            datatable_pagination_meta.contains("page: number"),
            "expected DatatablePaginationMeta.ts page field to use number:\n{datatable_pagination_meta}"
        );
        assert!(
            datatable_pagination_meta.contains("total_pages: number"),
            "expected DatatablePaginationMeta.ts total_pages field to use number:\n{datatable_pagination_meta}"
        );
        assert!(
            !datatable_pagination_meta.contains("bigint"),
            "did not expect bigint in DatatablePaginationMeta.ts:\n{datatable_pagination_meta}"
        );

        let minimal_status = fs::read_to_string(dir.path().join("MinimalExportStatus.ts")).unwrap();
        assert!(
            minimal_status
                .contains("export type MinimalExportStatus = \"pending\" | \"completed\";"),
            "expected MinimalExportStatus.ts to export a string union:\n{minimal_status}"
        );
        assert!(
            minimal_status.contains("export const MinimalExportStatusValues = ["),
            "expected MinimalExportStatus.ts to export Values:\n{minimal_status}"
        );
        assert!(
            minimal_status.contains(
                "{ value: \"pending\", labelKey: \"enum.minimal_export_status.pending\" }"
            ),
            "expected MinimalExportStatus.ts to export option metadata:\n{minimal_status}"
        );
        assert!(
            minimal_status.contains("keyKind: \"string\""),
            "expected MinimalExportStatus.ts to expose string keyKind:\n{minimal_status}"
        );

        let minimal_priority =
            fs::read_to_string(dir.path().join("MinimalExportPriority.ts")).unwrap();
        assert!(
            minimal_priority.contains("export type MinimalExportPriority = 1 | 2;"),
            "expected MinimalExportPriority.ts to export a numeric union:\n{minimal_priority}"
        );
        assert!(
            minimal_priority
                .contains("{ value: 1, labelKey: \"enum.minimal_export_priority.low\" }"),
            "expected MinimalExportPriority.ts to keep numeric option values:\n{minimal_priority}"
        );
        assert!(
            minimal_priority.contains("keyKind: \"int\""),
            "expected MinimalExportPriority.ts to expose int keyKind:\n{minimal_priority}"
        );

        let minimal_permission =
            fs::read_to_string(dir.path().join("MinimalExportPermission.ts")).unwrap();
        assert!(
            minimal_permission.contains("export const MinimalExportPermissionGroups = {"),
            "expected grouped AppEnum export:\n{minimal_permission}"
        );
        assert!(
            minimal_permission.contains(
                "auditLogs: { read: \"audit_logs.read\", manage: \"audit_logs.manage\" }"
            ),
            "expected snake_case modules to become camelCase groups:\n{minimal_permission}"
        );
        assert!(
            minimal_permission.contains("observability: { view: \"observability.view\" }"),
            "expected non-read/manage actions to stay available in groups:\n{minimal_permission}"
        );

        let app_enum_dto =
            fs::read_to_string(dir.path().join("MinimalExportAppEnumDto.ts")).unwrap();
        assert!(
            app_enum_dto
                .contains("import type { MinimalExportStatus } from \"./MinimalExportStatus\";"),
            "expected DTO to import string AppEnum without field override:\n{app_enum_dto}"
        );
        assert!(
            app_enum_dto.contains(
                "import type { MinimalExportPriority } from \"./MinimalExportPriority\";"
            ),
            "expected DTO to import numeric AppEnum without field override:\n{app_enum_dto}"
        );
        assert!(
            app_enum_dto.contains(
                "import type { MinimalExportPermission } from \"./MinimalExportPermission\";"
            ),
            "expected DTO to import vector AppEnum without field override:\n{app_enum_dto}"
        );
        assert!(
            app_enum_dto.contains("status: MinimalExportStatus"),
            "expected DTO field to reference AppEnum by name:\n{app_enum_dto}"
        );
        assert!(
            app_enum_dto.contains("priority: MinimalExportPriority | null"),
            "expected optional DTO field to reference nullable AppEnum by name:\n{app_enum_dto}"
        );
        assert!(
            app_enum_dto.contains("permissions: Array<MinimalExportPermission>"),
            "expected Vec AppEnum field to reference AppEnum array by name:\n{app_enum_dto}"
        );

        let index = fs::read_to_string(dir.path().join("index.ts")).unwrap();
        assert!(
            index.contains("export type { WsTokenResponse } from \"./WsTokenResponse\";"),
            "expected index.ts to re-export WsTokenResponse:\n{index}"
        );
        assert!(
            index.contains(
                "export type { CustomExportPathDto } from \"./custom/CustomExportPathDto\";"
            ),
            "expected index.ts to respect export_to paths:\n{index}"
        );
        assert!(
            index.contains(
                "export { type MinimalExportStatus, MinimalExportStatusValues, MinimalExportStatusOptions, MinimalExportStatusMeta } from \"./MinimalExportStatus\";"
            ),
            "expected index.ts to re-export AppEnum metadata:\n{index}"
        );
        assert!(
            index.contains(
                "export { type MinimalExportPermission, MinimalExportPermissionValues, MinimalExportPermissionOptions, MinimalExportPermissionMeta, MinimalExportPermissionGroups } from \"./MinimalExportPermission\";"
            ),
            "expected index.ts to re-export AppEnum groups only for grouped enums:\n{index}"
        );
        assert!(
            !index.contains("MinimalExportStatusGroups"),
            "did not expect non-dotted AppEnum groups in barrel:\n{index}"
        );
        assert!(
            index.contains(
                "export { RouteManifest, RouteIds, createRouteUrlBuilder, routeUrl, type RouteName, type RouteParams, type RouteParamValue, type RouteUrlOptions } from \"./RouteManifest\";"
            ),
            "expected index.ts to re-export route manifest helpers:\n{index}"
        );

        let route_manifest = fs::read_to_string(dir.path().join("RouteManifest.ts")).unwrap();
        assert!(
            route_manifest.contains("export const RouteManifest = {"),
            "expected RouteManifest.ts to export manifest object:\n{route_manifest}"
        );
        assert!(
            route_manifest.contains("export const RouteIds = {} as const;"),
            "expected empty route ids when no routes were exported:\n{route_manifest}"
        );
    }

    #[test]
    fn export_preserves_unmanaged_typescript_files() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("manual.ts"),
            "export const manual = true;\n",
        )
        .unwrap();

        export_all(dir.path()).unwrap();

        assert_eq!(
            fs::read_to_string(dir.path().join("manual.ts")).unwrap(),
            "export const manual = true;\n"
        );
        assert!(dir.path().join(TYPES_EXPORT_MANIFEST).exists());
    }

    #[test]
    fn export_removes_stale_files_from_previous_manifest() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("StaleGenerated.ts"), "stale\n").unwrap();
        fs::write(
            dir.path().join(TYPES_EXPORT_MANIFEST),
            serde_json::to_string(&vec!["StaleGenerated.ts"]).unwrap(),
        )
        .unwrap();

        export_all(dir.path()).unwrap();

        assert!(!dir.path().join("StaleGenerated.ts").exists());
        let manifest: Vec<String> = serde_json::from_str(
            &fs::read_to_string(dir.path().join(TYPES_EXPORT_MANIFEST)).unwrap(),
        )
        .unwrap();
        assert!(manifest.iter().any(|file| file == "index.ts"));
        assert!(manifest.iter().any(|file| file == "RouteManifest.ts"));
    }

    #[cfg(unix)]
    #[test]
    fn export_replaces_symlinked_planned_file_without_touching_target() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let outside = dir.path().join("outside.ts");
        fs::write(&outside, "export const outside = true;\n").unwrap();
        symlink(&outside, dir.path().join("index.ts")).unwrap();

        export_all(dir.path()).unwrap();

        assert_eq!(
            fs::read_to_string(&outside).unwrap(),
            "export const outside = true;\n"
        );
        assert!(!fs::symlink_metadata(dir.path().join("index.ts"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn exports_route_manifest_file_and_barrel_helpers() {
        let dir = tempdir().unwrap();
        export_all_with_routes(
            dir.path(),
            &[route_manifest_entry(
                "admin.users.show",
                "/api/v1/admin/users/{id}",
                &["id"],
            )],
        )
        .unwrap();

        let route_manifest = fs::read_to_string(dir.path().join("RouteManifest.ts")).unwrap();
        assert!(
            route_manifest.contains("\"admin.users.show\": { id: \"admin.users.show\""),
            "expected route entry:\n{route_manifest}"
        );
        assert!(
            route_manifest.contains("path: \"/api/v1/admin/users/{id}\""),
            "expected route path:\n{route_manifest}"
        );
        assert!(
            route_manifest.contains("params: [\"id\"]"),
            "expected route params:\n{route_manifest}"
        );
        assert!(
            route_manifest.contains("guard: \"admin\""),
            "expected guard metadata:\n{route_manifest}"
        );
        assert!(
            route_manifest.contains("permissions: [\"users.read\"]"),
            "expected permission metadata:\n{route_manifest}"
        );
        assert!(
            route_manifest.contains("request: \"ShowUserRequest\""),
            "expected request schema metadata:\n{route_manifest}"
        );
        assert!(
            route_manifest.contains("{ status: 200, schema: \"ShowUserResponse\" }"),
            "expected response schema metadata:\n{route_manifest}"
        );

        let index = fs::read_to_string(dir.path().join("index.ts")).unwrap();
        assert!(
            index.contains("from \"./RouteManifest\";"),
            "expected route manifest barrel export:\n{index}"
        );
    }

    #[test]
    fn route_manifest_renders_grouped_route_ids_and_url_helpers() {
        let routes = vec![
            route_manifest_entry("admin.audit_logs.index", "/api/v1/admin/audit-logs", &[]),
            route_manifest_entry("admin.users.show", "/api/v1/admin/users/{id}", &["id"]),
            route_manifest_entry("files.download", "/api/v1/files/{*path}", &["path"]),
            route_manifest_entry("legacy.users.show", "/legacy/users/:id", &["id"]),
            route_manifest_entry("health", "/health", &[]),
        ];

        let rendered = render_route_manifest(&routes).unwrap();

        assert!(
            rendered.contains("auditLogs: {"),
            "expected snake_case route id segments to become camelCase:\n{rendered}"
        );
        assert!(
            rendered.contains("show: \"admin.users.show\""),
            "expected nested route id leaf:\n{rendered}"
        );
        assert!(
            rendered.contains("health: \"health\""),
            "expected non-dotted route ids to remain usable:\n{rendered}"
        );
        assert!(
            rendered.contains("\"admin.users.show\": { \"id\": RouteParamValue };"),
            "expected typed params for routes with params:\n{rendered}"
        );
        assert!(
            rendered.contains("\"health\": Record<never, never>;"),
            "expected no-param routes to be callable without params:\n{rendered}"
        );
        assert!(
            rendered.contains("encodeURIComponent(String(params[param]))"),
            "expected runtime param URL encoding:\n{rendered}"
        );
        assert!(
            rendered.contains("Route ${String(name)} is missing required parameter ${param}"),
            "expected clear missing-param runtime error:\n{rendered}"
        );
        assert!(
            rendered.contains("function stripBasePath"),
            "expected basePath stripping helper:\n{rendered}"
        );
        assert!(
            rendered.contains("export function createRouteUrlBuilder"),
            "expected portal route URL builder helper:\n{rendered}"
        );
    }

    #[test]
    fn route_manifest_rejects_duplicate_route_ids() {
        let routes = vec![
            route_manifest_entry("admin.users.show", "/users/{id}", &["id"]),
            route_manifest_entry("admin.users.show", "/admin/users/{id}", &["id"]),
        ];

        let error = render_route_manifest(&routes)
            .expect_err("duplicate route ids should fail manifest export");

        assert!(
            error
                .to_string()
                .contains("duplicate route id `admin.users.show`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn route_manifest_rejects_camel_case_route_id_collisions() {
        let routes = vec![
            route_manifest_entry("admin.audit_logs.index", "/audit-logs", &[]),
            route_manifest_entry("admin.audit-logs.index", "/audit/logs", &[]),
        ];

        let error =
            render_route_manifest(&routes).expect_err("camelCase route id collisions should fail");

        assert!(
            error.to_string().contains("both normalize to `auditLogs`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn app_enum_groups_are_not_rendered_for_plain_string_enums() {
        let rendered = render_app_enum("PlainStatus", &string_meta(&["pending", "completed"]))
            .expect("plain string enums should render");

        assert!(!rendered.has_groups);
        assert!(
            !rendered.content.contains("PlainStatusGroups"),
            "did not expect groups for non-dotted string enum:\n{}",
            rendered.content
        );
    }

    #[test]
    fn app_enum_groups_reject_mixed_dotted_and_plain_keys() {
        let error = render_app_enum("MixedPermission", &string_meta(&["users.read", "pending"]))
            .expect_err("mixed dotted and plain keys should fail");

        assert!(
            error
                .to_string()
                .contains("mixes dotted `<module>.<action>` keys with non-dotted keys"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn app_enum_groups_reject_camel_case_collisions() {
        let error = render_app_enum(
            "CollidingPermission",
            &string_meta(&["audit_logs.read", "audit-logs.manage"]),
        )
        .expect_err("camelCase module collisions should fail");

        assert!(
            error.to_string().contains("both normalize to `auditLogs`"),
            "unexpected error: {error}"
        );
    }
}
