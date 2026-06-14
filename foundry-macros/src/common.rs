use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DeriveInput, Expr, Field, Fields, FieldsNamed, GenericArgument, Ident,
    LitBool, LitStr, Path, PathArguments, Type,
};

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
