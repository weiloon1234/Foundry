use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DataEnum, DeriveInput, Expr, Field, Fields, FieldsNamed, GenericArgument,
    Ident, LitBool, LitStr, Path, PathArguments, Token, Type,
};

pub fn split_identifier_words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();

    for (index, ch) in chars.iter().copied().enumerate() {
        if is_identifier_separator(ch) {
            push_identifier_token(&mut tokens, &mut current);
            continue;
        }

        if let Some(prev) = current.chars().last() {
            let next = chars.get(index + 1).copied();
            if should_split_identifier(prev, ch, next) {
                push_identifier_token(&mut tokens, &mut current);
            }
        }

        current.push(ch);
    }

    push_identifier_token(&mut tokens, &mut current);
    tokens
}

pub fn to_snake_case(name: &str) -> String {
    split_identifier_words(name)
        .into_iter()
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("_")
}

#[derive(Clone, Copy)]
pub enum SerdeRenameRule {
    Lowercase,
    Uppercase,
    PascalCase,
    CamelCase,
    SnakeCase,
    ScreamingSnakeCase,
    KebabCase,
    ScreamingKebabCase,
}

impl SerdeRenameRule {
    fn from_lit(lit: &LitStr) -> syn::Result<Self> {
        match lit.value().as_str() {
            "lowercase" => Ok(Self::Lowercase),
            "UPPERCASE" => Ok(Self::Uppercase),
            "PascalCase" => Ok(Self::PascalCase),
            "camelCase" => Ok(Self::CamelCase),
            "snake_case" => Ok(Self::SnakeCase),
            "SCREAMING_SNAKE_CASE" => Ok(Self::ScreamingSnakeCase),
            "kebab-case" => Ok(Self::KebabCase),
            "SCREAMING-KEBAB-CASE" => Ok(Self::ScreamingKebabCase),
            _ => Err(syn::Error::new_spanned(
                lit,
                "unsupported serde rename_all value for Foundry derive",
            )),
        }
    }

    fn apply(self, name: &str) -> String {
        match self {
            Self::Lowercase => name.to_ascii_lowercase(),
            Self::Uppercase => name.to_ascii_uppercase(),
            Self::PascalCase => to_pascal_case(name),
            Self::CamelCase => to_camel_case(name),
            Self::SnakeCase => to_snake_case(name),
            Self::ScreamingSnakeCase => split_identifier_words(name)
                .into_iter()
                .map(|word| word.to_ascii_uppercase())
                .collect::<Vec<_>>()
                .join("_"),
            Self::KebabCase => split_identifier_words(name)
                .into_iter()
                .map(|word| word.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("-"),
            Self::ScreamingKebabCase => split_identifier_words(name)
                .into_iter()
                .map(|word| word.to_ascii_uppercase())
                .collect::<Vec<_>>()
                .join("-"),
        }
    }
}

pub fn apply_serde_rename_all(rule: Option<SerdeRenameRule>, name: &str) -> String {
    rule.map(|rule| rule.apply(name))
        .unwrap_or_else(|| name.to_string())
}

pub fn rust_ident_name(ident: &syn::Ident) -> String {
    let name = ident.to_string();
    name.strip_prefix("r#").unwrap_or(&name).to_string()
}

pub fn serde_rename(attrs: &[Attribute]) -> syn::Result<Option<String>> {
    let mut rename = None::<LitStr>;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                if let Some(value) = parse_serde_name_value(meta, "rename")? {
                    set_once_lit_str(&mut rename, value, "rename")?;
                }
            } else {
                consume_meta_value(meta)?;
            }
            Ok(())
        })?;
    }

    Ok(rename.map(|value| value.value()))
}

pub fn serde_rename_all(attrs: &[Attribute]) -> syn::Result<Option<SerdeRenameRule>> {
    let mut rename_all = None::<LitStr>;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                if let Some(value) = parse_serde_name_value(meta, "rename_all")? {
                    set_once_lit_str(&mut rename_all, value, "rename_all")?;
                }
            } else {
                consume_meta_value(meta)?;
            }
            Ok(())
        })?;
    }

    rename_all
        .as_ref()
        .map(SerdeRenameRule::from_lit)
        .transpose()
}

pub fn wire_field_name(rust_field_name: &str, field_wire_names: &[(String, String)]) -> String {
    field_wire_names
        .iter()
        .find_map(|(rust, wire)| (rust == rust_field_name).then_some(wire.clone()))
        .unwrap_or_else(|| rust_field_name.to_string())
}

pub fn reject_duplicate_contract_field_names(
    fields: &FieldsNamed,
    attrs: &[Attribute],
) -> syn::Result<()> {
    reject_ts_renames(attrs, "contract struct")?;

    let rename_all = serde_rename_all(attrs)?;
    let mut seen = Vec::<(String, String)>::new();

    for field in &fields.named {
        if should_skip_contract_field(field)? {
            continue;
        }
        reject_custom_serde_transforms(&field.attrs, "field")?;
        reject_ts_renames(&field.attrs, "public field")?;
        if serde_has_flatten(&field.attrs)? {
            continue;
        }
        reject_serde_aliases(
            &field.attrs,
            "Foundry contract derives cannot represent `#[serde(alias = \"...\")]` on public fields; aliases make the backend accept undocumented JSON keys",
        )?;

        let Some(ident) = &field.ident else {
            continue;
        };
        let rust_name = rust_ident_name(ident);
        let wire_name = serde_rename(&field.attrs)?
            .unwrap_or_else(|| apply_serde_rename_all(rename_all, &rust_name));

        if let Some((previous_rust_name, _)) =
            seen.iter().find(|(_, seen_wire)| seen_wire == &wire_name)
        {
            return Err(syn::Error::new_spanned(
                field,
                format!(
                    "Foundry contract derive contains duplicate JSON field `{wire_name}` from Rust fields `{previous_rust_name}` and `{rust_name}`; use unique serde rename values or split the DTO"
                ),
            ));
        }

        seen.push((rust_name, wire_name));
    }

    Ok(())
}

pub fn reject_duplicate_contract_variant_names(
    data: &DataEnum,
    attrs: &[Attribute],
) -> syn::Result<()> {
    reject_ts_renames(attrs, "contract enum")?;

    let rename_all = serde_rename_all(attrs)?;
    let mut seen = Vec::<(String, String)>::new();

    for variant in &data.variants {
        reject_custom_serde_transforms(&variant.attrs, "enum variant")?;
        reject_ts_renames(&variant.attrs, "enum variant")?;
        reject_serde_aliases(
            &variant.attrs,
            "Foundry contract derives cannot represent `#[serde(alias = \"...\")]` on enum variants; aliases make the backend accept undocumented enum values",
        )?;
        let rust_name = rust_ident_name(&variant.ident);
        let wire_name = serde_rename(&variant.attrs)?
            .unwrap_or_else(|| apply_serde_rename_all(rename_all, &rust_name));

        if let Some((previous_rust_name, _)) =
            seen.iter().find(|(_, seen_wire)| seen_wire == &wire_name)
        {
            return Err(syn::Error::new_spanned(
                variant,
                format!(
                    "Foundry contract derive contains duplicate JSON enum variant `{wire_name}` from Rust variants `{previous_rust_name}` and `{rust_name}`; use unique serde rename values or split the enum"
                ),
            ));
        }

        seen.push((rust_name, wire_name));
    }

    Ok(())
}

fn reject_ts_renames(attrs: &[Attribute], target: &str) -> syn::Result<()> {
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("ts")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") || meta.path.is_ident("rename_all") {
                let attr_name = if meta.path.is_ident("rename") {
                    "rename"
                } else {
                    "rename_all"
                };
                let message = format!(
                    "Foundry contract derives use serde names as the single wire-name source; `#[ts({attr_name} = \"...\")]` on {target} would make generated TypeScript disagree with JSON, OpenAPI, or validation metadata. Use `#[serde(rename = \"...\")]` or `#[serde(rename_all = \"...\")]` instead"
                );

                if meta.input.peek(Token![=]) {
                    let value: Expr = meta.value()?.parse()?;
                    return Err(syn::Error::new_spanned(value, message));
                }

                return Err(meta.error(message));
            }

            consume_meta_value(meta)?;
            Ok(())
        })?;
    }

    Ok(())
}

fn reject_custom_serde_transforms(attrs: &[Attribute], target: &str) -> syn::Result<()> {
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("with")
                || meta.path.is_ident("serialize_with")
                || meta.path.is_ident("deserialize_with")
            {
                let message = format!(
                    "Foundry contract derives cannot represent custom serde {target} transforms such as `#[serde(with = \"...\")]`, `#[serde(serialize_with = \"...\")]`, or `#[serde(deserialize_with = \"...\")]`; use an explicit typed DTO shape or normalize legacy input before validation"
                );
                if meta.input.peek(Token![=]) {
                    let value: Expr = meta.value()?.parse()?;
                    return Err(syn::Error::new_spanned(value, message));
                }

                return Err(meta.error(message));
            }

            consume_meta_value(meta)?;
            Ok(())
        })?;
    }

    Ok(())
}

fn reject_serde_aliases(attrs: &[Attribute], message: &str) -> syn::Result<()> {
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("alias") {
                if meta.input.peek(Token![=]) {
                    let value: LitStr = meta.value()?.parse()?;
                    return Err(syn::Error::new_spanned(value, message));
                }

                return Err(meta.error(message));
            }

            consume_meta_value(meta)?;
            Ok(())
        })?;
    }

    Ok(())
}

pub fn should_skip_contract_field(field: &Field) -> syn::Result<bool> {
    for attr in &field.attrs {
        if is_contract_skip_attr(attr)? {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn serde_directional_skip(attrs: &[Attribute]) -> syn::Result<Option<&'static str>> {
    let mut directional_skip = None;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip_serializing") {
                directional_skip.get_or_insert("skip_serializing");
            } else if meta.path.is_ident("skip_deserializing") {
                directional_skip.get_or_insert("skip_deserializing");
            }
            consume_meta_value(meta)?;
            Ok(())
        })?;
    }

    Ok(directional_skip)
}

pub fn serde_skips_deserializing(attrs: &[Attribute]) -> syn::Result<Option<&'static str>> {
    let mut skip = None;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                skip.get_or_insert("skip");
            } else if meta.path.is_ident("skip_deserializing") {
                skip.get_or_insert("skip_deserializing");
            }
            consume_meta_value(meta)?;
            Ok(())
        })?;
    }

    Ok(skip)
}

pub fn reject_directional_serde_skip(field: &Field) -> syn::Result<()> {
    if let Some(attr) = serde_directional_skip(&field.attrs)? {
        return Err(syn::Error::new_spanned(
            field,
            format!("Foundry cannot represent `#[serde({attr})]` in a single TypeScript/OpenAPI contract; use separate request/response DTOs, or use `#[serde(skip)]` / `#[ts(skip)]` when the field is never public"),
        ));
    }

    Ok(())
}

pub fn serde_has_default(attrs: &[Attribute]) -> syn::Result<bool> {
    let mut has_default = false;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("default") {
                has_default = true;
            }
            consume_meta_value(meta)?;
            Ok(())
        })?;
    }

    Ok(has_default)
}

pub fn serde_denies_unknown_fields(attrs: &[Attribute]) -> syn::Result<bool> {
    let mut denies_unknown_fields = false;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("deny_unknown_fields") {
                denies_unknown_fields = true;
            }
            consume_meta_value(meta)?;
            Ok(())
        })?;
    }

    Ok(denies_unknown_fields)
}

pub fn serde_has_flatten(attrs: &[Attribute]) -> syn::Result<bool> {
    let mut has_flatten = false;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("flatten") {
                has_flatten = true;
            }
            consume_meta_value(meta)?;
            Ok(())
        })?;
    }

    Ok(has_flatten)
}

pub fn reject_serde_flatten_with_deny_unknown_fields(field: &Field) -> syn::Result<()> {
    if serde_has_flatten(&field.attrs)? {
        return Err(syn::Error::new_spanned(
            field,
            "`#[serde(deny_unknown_fields)]` cannot be combined with `#[serde(flatten)]`; remove one of the attributes or split the DTO so the strict request shape has explicit fields",
        ));
    }

    Ok(())
}

pub fn serde_has_skip_serializing_if(attrs: &[Attribute]) -> syn::Result<bool> {
    let mut skip_serializing = false;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip_serializing_if") {
                skip_serializing = true;
            }
            consume_meta_value(meta)?;
            Ok(())
        })?;
    }

    Ok(skip_serializing)
}

pub fn ts_has_optional(attrs: &[Attribute]) -> syn::Result<bool> {
    let mut optional = false;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("ts")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("optional") {
                optional = true;
            }
            consume_meta_value(meta)?;
            Ok(())
        })?;
    }

    Ok(optional)
}

pub fn validate_has_required_rule(attrs: &[Attribute]) -> syn::Result<bool> {
    let mut required = false;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("validate")) {
        attr.parse_args_with(|input: syn::parse::ParseStream<'_>| {
            while !input.is_empty() {
                let ident: syn::Ident = input.parse()?;
                if ident == "required" {
                    required = true;
                }
                if input.peek(syn::token::Paren) {
                    let content;
                    syn::parenthesized!(content in input);
                    let _ = content.parse::<TokenStream>();
                }
                if !input.is_empty() {
                    let _: syn::Token![,] = input.parse()?;
                }
            }
            Ok(())
        })?;
    }

    Ok(required)
}

fn is_contract_skip_attr(attr: &Attribute) -> syn::Result<bool> {
    if !(attr.path().is_ident("serde") || attr.path().is_ident("ts")) {
        return Ok(false);
    }

    let mut skipped = false;
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("skip") {
            skipped = true;
        }
        consume_meta_value(meta)?;
        Ok(())
    })?;

    Ok(skipped)
}

fn parse_serde_name_value(
    meta: syn::meta::ParseNestedMeta<'_>,
    name: &str,
) -> syn::Result<Option<LitStr>> {
    if meta.input.peek(Token![=]) {
        return Ok(Some(meta.value()?.parse()?));
    }

    if !meta.input.peek(syn::token::Paren) {
        return Ok(None);
    }

    let mut serialize = None::<LitStr>;
    let mut deserialize = None::<LitStr>;
    meta.parse_nested_meta(|nested| {
        if nested.path.is_ident("serialize") {
            set_once_lit_str(&mut serialize, nested.value()?.parse()?, "serialize")?;
        } else if nested.path.is_ident("deserialize") {
            set_once_lit_str(&mut deserialize, nested.value()?.parse()?, "deserialize")?;
        } else {
            consume_meta_value(nested)?;
        }
        Ok(())
    })?;

    match (serialize, deserialize) {
        (Some(serialize), Some(deserialize)) if serialize.value() != deserialize.value() => {
            Err(syn::Error::new_spanned(
                serialize,
                format!(
                    "Foundry derives cannot represent serde {name} with different serialize and deserialize names"
                ),
            ))
        }
        (Some(value), _) | (_, Some(value)) => Ok(Some(value)),
        (None, None) => Ok(None),
    }
}

fn set_once_lit_str(slot: &mut Option<LitStr>, value: LitStr, name: &str) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new_spanned(
            value,
            format!("duplicate serde `{name}` attribute"),
        ));
    }

    *slot = Some(value);
    Ok(())
}

pub fn consume_meta_value(meta: syn::meta::ParseNestedMeta<'_>) -> syn::Result<()> {
    if meta.input.peek(Token![=]) {
        let _ = meta.value()?.parse::<Expr>()?;
    } else if meta.input.peek(syn::token::Paren) {
        let content;
        syn::parenthesized!(content in meta.input);
        let _ = content.parse::<TokenStream>();
    }
    Ok(())
}

fn to_pascal_case(name: &str) -> String {
    split_identifier_words(name)
        .into_iter()
        .map(|word| capitalize_word(&word))
        .collect::<String>()
}

fn to_camel_case(name: &str) -> String {
    let mut words = split_identifier_words(name).into_iter();
    let Some(first) = words.next() else {
        return String::new();
    };
    let mut output = first.to_ascii_lowercase();
    for word in words {
        output.push_str(&capitalize_word(&word));
    }
    output
}

fn capitalize_word(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut output = first.to_uppercase().collect::<String>();
    output.push_str(&chars.as_str().to_ascii_lowercase());
    output
}

fn push_identifier_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

fn is_identifier_separator(ch: char) -> bool {
    matches!(ch, '_' | '-' | ' ')
}

fn should_split_identifier(prev: char, current: char, next: Option<char>) -> bool {
    if is_identifier_separator(prev) || is_identifier_separator(current) {
        return false;
    }

    (prev.is_lowercase() && current.is_uppercase())
        || (prev.is_alphabetic() && current.is_ascii_digit())
        || (prev.is_ascii_digit() && current.is_alphabetic())
        || (prev.is_uppercase()
            && current.is_uppercase()
            && next.is_some_and(|ch| ch.is_lowercase()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbTypeSpec {
    Int16,
    Int32,
    Int64,
    Bool,
    Float32,
    Float64,
    Numeric,
    Text,
    Json,
    Uuid,
    TimestampTz,
    Timestamp,
    Date,
    Time,
    Bytea,
    Int16Array,
    Int32Array,
    Int64Array,
    BoolArray,
    Float32Array,
    Float64Array,
    NumericArray,
    TextArray,
    JsonArray,
    UuidArray,
    TimestampTzArray,
    TimestampArray,
    DateArray,
    TimeArray,
    ByteaArray,
}

impl DbTypeSpec {
    pub fn from_lit(lit: &LitStr) -> syn::Result<Self> {
        match lit.value().as_str() {
            "int16" => Ok(Self::Int16),
            "int32" => Ok(Self::Int32),
            "int64" => Ok(Self::Int64),
            "bool" => Ok(Self::Bool),
            "float32" => Ok(Self::Float32),
            "float64" => Ok(Self::Float64),
            "numeric" => Ok(Self::Numeric),
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "uuid" => Ok(Self::Uuid),
            "timestamp_tz" => Ok(Self::TimestampTz),
            "timestamp" => Ok(Self::Timestamp),
            "date" => Ok(Self::Date),
            "time" => Ok(Self::Time),
            "bytea" => Ok(Self::Bytea),
            "int16_array" => Ok(Self::Int16Array),
            "int32_array" => Ok(Self::Int32Array),
            "int64_array" => Ok(Self::Int64Array),
            "bool_array" => Ok(Self::BoolArray),
            "float32_array" => Ok(Self::Float32Array),
            "float64_array" => Ok(Self::Float64Array),
            "numeric_array" => Ok(Self::NumericArray),
            "text_array" => Ok(Self::TextArray),
            "json_array" => Ok(Self::JsonArray),
            "uuid_array" => Ok(Self::UuidArray),
            "timestamp_tz_array" => Ok(Self::TimestampTzArray),
            "timestamp_array" => Ok(Self::TimestampArray),
            "date_array" => Ok(Self::DateArray),
            "time_array" => Ok(Self::TimeArray),
            "bytea_array" => Ok(Self::ByteaArray),
            _ => Err(syn::Error::new(
                lit.span(),
                "unsupported db_type for Foundry derive",
            )),
        }
    }

    pub fn tokens(self) -> TokenStream {
        match self {
            Self::Int16 => quote!(::foundry::DbType::Int16),
            Self::Int32 => quote!(::foundry::DbType::Int32),
            Self::Int64 => quote!(::foundry::DbType::Int64),
            Self::Bool => quote!(::foundry::DbType::Bool),
            Self::Float32 => quote!(::foundry::DbType::Float32),
            Self::Float64 => quote!(::foundry::DbType::Float64),
            Self::Numeric => quote!(::foundry::DbType::Numeric),
            Self::Text => quote!(::foundry::DbType::Text),
            Self::Json => quote!(::foundry::DbType::Json),
            Self::Uuid => quote!(::foundry::DbType::Uuid),
            Self::TimestampTz => quote!(::foundry::DbType::TimestampTz),
            Self::Timestamp => quote!(::foundry::DbType::Timestamp),
            Self::Date => quote!(::foundry::DbType::Date),
            Self::Time => quote!(::foundry::DbType::Time),
            Self::Bytea => quote!(::foundry::DbType::Bytea),
            Self::Int16Array => quote!(::foundry::DbType::Int16Array),
            Self::Int32Array => quote!(::foundry::DbType::Int32Array),
            Self::Int64Array => quote!(::foundry::DbType::Int64Array),
            Self::BoolArray => quote!(::foundry::DbType::BoolArray),
            Self::Float32Array => quote!(::foundry::DbType::Float32Array),
            Self::Float64Array => quote!(::foundry::DbType::Float64Array),
            Self::NumericArray => quote!(::foundry::DbType::NumericArray),
            Self::TextArray => quote!(::foundry::DbType::TextArray),
            Self::JsonArray => quote!(::foundry::DbType::JsonArray),
            Self::UuidArray => quote!(::foundry::DbType::UuidArray),
            Self::TimestampTzArray => quote!(::foundry::DbType::TimestampTzArray),
            Self::TimestampArray => quote!(::foundry::DbType::TimestampArray),
            Self::DateArray => quote!(::foundry::DbType::DateArray),
            Self::TimeArray => quote!(::foundry::DbType::TimeArray),
            Self::ByteaArray => quote!(::foundry::DbType::ByteaArray),
        }
    }
}

#[derive(Default)]
pub struct ModelArgs {
    pub table: Option<Expr>,
    pub primary_key: Option<LitStr>,
    pub primary_key_strategy: Option<LitStr>,
    pub lifecycle: Option<Path>,
    pub audit: Option<LitBool>,
    pub timestamps: Option<LitBool>,
    pub soft_deletes: Option<LitBool>,
}

#[derive(Default, Clone)]
pub struct FieldArgs {
    pub column: Option<LitStr>,
    pub alias: Option<LitStr>,
    pub source: Option<LitStr>,
    pub db_type: Option<DbTypeSpec>,
    pub write_mutator: Option<LitStr>,
    pub read_accessor: Option<LitStr>,
    pub audit_exclude: bool,
}

pub fn ensure_named_struct(input: &DeriveInput) -> syn::Result<&FieldsNamed> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "Foundry derives do not support generic structs",
        ));
    }

    match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => Ok(fields),
            _ => Err(syn::Error::new_spanned(
                &data.fields,
                "Foundry derives require a struct with named fields",
            )),
        },
        _ => Err(syn::Error::new_spanned(
            input,
            "Foundry derives are only supported on structs",
        )),
    }
}

pub fn parse_model_args(attrs: &[Attribute]) -> syn::Result<ModelArgs> {
    let mut args = ModelArgs::default();

    for attr in attrs.iter().filter(|attr| attr.path().is_ident("foundry")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                set_once_expr(&mut args.table, "table", meta.value()?)?;
            } else if meta.path.is_ident("primary_key") {
                set_once_parse(&mut args.primary_key, "primary_key", meta.value()?)?;
            } else if meta.path.is_ident("primary_key_strategy") {
                set_once_parse(
                    &mut args.primary_key_strategy,
                    "primary_key_strategy",
                    meta.value()?,
                )?;
            } else if meta.path.is_ident("lifecycle") {
                set_once_parse(&mut args.lifecycle, "lifecycle", meta.value()?)?;
            } else if meta.path.is_ident("audit") {
                set_once_parse(&mut args.audit, "audit", meta.value()?)?;
            } else if meta.path.is_ident("timestamps") {
                set_once_parse(&mut args.timestamps, "timestamps", meta.value()?)?;
            } else if meta.path.is_ident("soft_deletes") {
                set_once_parse(&mut args.soft_deletes, "soft_deletes", meta.value()?)?;
            } else {
                return Err(meta.error("unsupported foundry attribute for Model derive"));
            }
            Ok(())
        })?;
    }

    Ok(args)
}
pub fn parse_field_args(field: &Field) -> syn::Result<FieldArgs> {
    let mut args = FieldArgs::default();

    for attr in field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("foundry"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("column") {
                set_once_parse(&mut args.column, "column", meta.value()?)?;
            } else if meta.path.is_ident("alias") {
                set_once_parse(&mut args.alias, "alias", meta.value()?)?;
            } else if meta.path.is_ident("source") {
                set_once_parse(&mut args.source, "source", meta.value()?)?;
            } else if meta.path.is_ident("db_type") {
                let mut lit = None;
                set_once_parse(&mut lit, "db_type", meta.value()?)?;
                let lit = lit.ok_or_else(|| meta.error("db_type requires a string value"))?;
                args.db_type = Some(DbTypeSpec::from_lit(&lit)?);
            } else if meta.path.is_ident("write_mutator") {
                set_once_parse(&mut args.write_mutator, "write_mutator", meta.value()?)?;
            } else if meta.path.is_ident("read_accessor") {
                set_once_parse(&mut args.read_accessor, "read_accessor", meta.value()?)?;
            } else if meta.path.is_ident("audit_exclude") {
                if args.audit_exclude {
                    return Err(meta.error("duplicate audit_exclude attribute"));
                }
                args.audit_exclude = true;
            } else {
                return Err(meta.error("unsupported foundry field attribute"));
            }
            Ok(())
        })?;
    }

    Ok(args)
}

pub fn infer_or_explicit_db_type(
    ty: &Type,
    explicit: Option<DbTypeSpec>,
) -> syn::Result<DbTypeSpec> {
    if let Some(explicit) = explicit {
        return Ok(explicit);
    }

    if let Some(inferred) = infer_db_type(ty) {
        return Ok(inferred);
    }

    Err(syn::Error::new_spanned(
        ty,
        "unsupported field type; add #[foundry(db_type = \"...\")]",
    ))
}

pub fn infer_db_type(ty: &Type) -> Option<DbTypeSpec> {
    if let Some(inner) = option_inner_type(ty) {
        return infer_db_type(inner);
    }

    if is_vec_of_u8(ty) {
        return Some(DbTypeSpec::Bytea);
    }

    if let Some(inner) = vec_inner_type(ty) {
        return infer_array_db_type(inner);
    }

    if type_path_matches(ty, &["i16"]) {
        return Some(DbTypeSpec::Int16);
    }
    if type_path_matches(ty, &["i32"]) {
        return Some(DbTypeSpec::Int32);
    }
    if type_path_matches(ty, &["i64"]) {
        return Some(DbTypeSpec::Int64);
    }
    if type_path_matches(ty, &["bool"]) {
        return Some(DbTypeSpec::Bool);
    }
    if type_path_matches(ty, &["f32"]) {
        return Some(DbTypeSpec::Float32);
    }
    if type_path_matches(ty, &["f64"]) {
        return Some(DbTypeSpec::Float64);
    }
    if type_path_ends_with(ty, &["Numeric"]) {
        return Some(DbTypeSpec::Numeric);
    }
    if type_path_matches(ty, &["String"]) {
        return Some(DbTypeSpec::Text);
    }
    if type_path_matches(ty, &["str"]) {
        return Some(DbTypeSpec::Text);
    }
    if type_path_ends_with(ty, &["serde_json", "Value"]) {
        return Some(DbTypeSpec::Json);
    }
    if type_argument_if_last_segment_ident(ty, "ModelId").is_some() {
        return Some(DbTypeSpec::Uuid);
    }
    if type_path_ends_with(ty, &["uuid", "Uuid"]) || type_path_ends_with(ty, &["Uuid"]) {
        return Some(DbTypeSpec::Uuid);
    }
    if type_path_non_generic_ends_with(ty, &["DateTime"]) {
        return Some(DbTypeSpec::TimestampTz);
    }
    if type_path_non_generic_ends_with(ty, &["LocalDateTime"]) {
        return Some(DbTypeSpec::Timestamp);
    }
    if type_path_non_generic_ends_with(ty, &["Date"]) {
        return Some(DbTypeSpec::Date);
    }
    if type_path_non_generic_ends_with(ty, &["Time"]) {
        return Some(DbTypeSpec::Time);
    }

    None
}

fn infer_array_db_type(ty: &Type) -> Option<DbTypeSpec> {
    match infer_db_type(ty)? {
        DbTypeSpec::Int16 => Some(DbTypeSpec::Int16Array),
        DbTypeSpec::Int32 => Some(DbTypeSpec::Int32Array),
        DbTypeSpec::Int64 => Some(DbTypeSpec::Int64Array),
        DbTypeSpec::Bool => Some(DbTypeSpec::BoolArray),
        DbTypeSpec::Float32 => Some(DbTypeSpec::Float32Array),
        DbTypeSpec::Float64 => Some(DbTypeSpec::Float64Array),
        DbTypeSpec::Numeric => Some(DbTypeSpec::NumericArray),
        DbTypeSpec::Text => Some(DbTypeSpec::TextArray),
        DbTypeSpec::Json => Some(DbTypeSpec::JsonArray),
        DbTypeSpec::Uuid => Some(DbTypeSpec::UuidArray),
        DbTypeSpec::TimestampTz => Some(DbTypeSpec::TimestampTzArray),
        DbTypeSpec::Timestamp => Some(DbTypeSpec::TimestampArray),
        DbTypeSpec::Date => Some(DbTypeSpec::DateArray),
        DbTypeSpec::Time => Some(DbTypeSpec::TimeArray),
        DbTypeSpec::Bytea => Some(DbTypeSpec::ByteaArray),
        DbTypeSpec::Int16Array
        | DbTypeSpec::Int32Array
        | DbTypeSpec::Int64Array
        | DbTypeSpec::BoolArray
        | DbTypeSpec::Float32Array
        | DbTypeSpec::Float64Array
        | DbTypeSpec::NumericArray
        | DbTypeSpec::TextArray
        | DbTypeSpec::JsonArray
        | DbTypeSpec::UuidArray
        | DbTypeSpec::TimestampTzArray
        | DbTypeSpec::TimestampArray
        | DbTypeSpec::DateArray
        | DbTypeSpec::TimeArray
        | DbTypeSpec::ByteaArray => None,
    }
}

pub fn option_inner_type(ty: &Type) -> Option<&Type> {
    type_argument_if_last_segment_ident(ty, "Option")
}

pub fn vec_inner_type(ty: &Type) -> Option<&Type> {
    type_argument_if_last_segment_ident(ty, "Vec")
}

fn is_vec_of_u8(ty: &Type) -> bool {
    vec_inner_type(ty)
        .map(|inner| type_path_matches(inner, &["u8"]))
        .unwrap_or(false)
}

pub fn loaded_inner_type(ty: &Type) -> Option<&Type> {
    type_argument_if_last_segment_ident(ty, "Loaded")
}

pub fn require_ident(field: &Field) -> syn::Result<&Ident> {
    field
        .ident
        .as_ref()
        .ok_or_else(|| syn::Error::new(field.span(), "Foundry derives require named struct fields"))
}

pub fn field_name_literal(field_ident: &Ident, explicit: &Option<LitStr>) -> LitStr {
    explicit
        .clone()
        .unwrap_or_else(|| LitStr::new(&field_ident.to_string(), field_ident.span()))
}

pub fn screaming_const_ident(field_ident: &Ident) -> Ident {
    format_ident!(
        "{}",
        to_screaming_snake(&field_ident.to_string()),
        span = field_ident.span()
    )
}

pub fn static_ident(prefix: &str, type_ident: &Ident) -> Ident {
    format_ident!(
        "__FOUNDRY_{}_{}",
        prefix,
        to_screaming_snake(&type_ident.to_string()),
        span = Span::call_site()
    )
}

pub fn helper_ident(prefix: &str, type_ident: &Ident) -> Ident {
    format_ident!(
        "__foundry_{}_{}",
        prefix,
        type_ident.to_string().to_lowercase(),
        span = Span::call_site()
    )
}

fn to_screaming_snake(value: &str) -> String {
    let mut output = String::new();
    let mut previous_was_separator = true;

    for character in value.chars() {
        if character == '_' {
            if !output.ends_with('_') {
                output.push('_');
            }
            previous_was_separator = true;
            continue;
        }

        if character.is_ascii_uppercase() && !previous_was_separator && !output.ends_with('_') {
            output.push('_');
        }

        output.push(character.to_ascii_uppercase());
        previous_was_separator = false;
    }

    output
}

fn set_once_parse<T: Parse>(
    slot: &mut Option<T>,
    name: &str,
    value: ParseStream<'_>,
) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new(
            value.span(),
            format!("duplicate `{name}` attribute"),
        ));
    }
    *slot = Some(value.parse()?);
    Ok(())
}

fn set_once_expr(slot: &mut Option<Expr>, name: &str, value: ParseStream<'_>) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new(
            value.span(),
            format!("duplicate `{name}` attribute"),
        ));
    }
    *slot = Some(value.parse()?);
    Ok(())
}

pub fn type_argument_if_last_segment_ident<'a>(ty: &'a Type, ident: &str) -> Option<&'a Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != ident {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let GenericArgument::Type(inner) = arguments.args.first()? else {
        return None;
    };
    Some(inner)
}

pub fn type_path_last_segment_matches(ty: &Type, expected: &str) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident == expected)
        .unwrap_or(false)
}

fn type_path_matches(ty: &Type, expected: &[&str]) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    let segments = type_path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    segments.len() == expected.len()
        && segments
            .iter()
            .map(String::as_str)
            .eq(expected.iter().copied())
}

fn type_path_ends_with(ty: &Type, expected_tail: &[&str]) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    let segments = type_path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    if segments.len() < expected_tail.len() {
        return false;
    }
    segments[segments.len() - expected_tail.len()..]
        .iter()
        .map(String::as_str)
        .eq(expected_tail.iter().copied())
}

fn type_path_non_generic_ends_with(ty: &Type, expected_tail: &[&str]) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    let Some(last) = type_path.path.segments.last() else {
        return false;
    };
    if !matches!(last.arguments, PathArguments::None) {
        return false;
    }
    type_path_ends_with(ty, expected_tail)
}
