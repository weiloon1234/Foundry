use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, Type};

use crate::common::{
    apply_serde_rename_all as apply_rename_all, option_inner_type, reject_directional_serde_skip,
    reject_duplicate_contract_field_names, reject_duplicate_contract_variant_names,
    reject_serde_flatten_with_deny_unknown_fields, rust_ident_name, serde_denies_unknown_fields,
    serde_has_default, serde_has_flatten, serde_has_skip_serializing_if, serde_rename,
    serde_rename_all, should_skip_contract_field, ts_has_optional,
    type_argument_if_last_segment_ident, type_path_last_segment_matches, vec_inner_type,
    wire_field_name,
};

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let name_str = name.to_string();

    match &input.data {
        Data::Struct(data) => expand_struct(name, &name_str, &data.fields, &input.attrs),
        Data::Enum(data) => expand_enum(name, &name_str, data, &input.attrs),
        Data::Union(_) => Err(syn::Error::new_spanned(
            &input,
            "ApiSchema cannot be derived for unions",
        )),
    }
}

fn expand_struct(
    name: &syn::Ident,
    name_str: &str,
    fields: &Fields,
    attrs: &[syn::Attribute],
) -> syn::Result<TokenStream> {
    let named = match fields {
        Fields::Named(named) => named,
        _ => {
            return Err(syn::Error::new_spanned(
                fields,
                "ApiSchema derive requires named fields",
            ))
        }
    };

    let mut property_inserts = Vec::new();
    let mut required_inserts = Vec::new();
    let rename_all = serde_rename_all(attrs)?;
    let struct_has_default = serde_has_default(attrs)?;
    let deny_unknown_fields = serde_denies_unknown_fields(attrs)?;
    let after_hooks = parse_struct_after_hooks(attrs)?;
    let mut field_wire_names = Vec::<(String, String)>::new();

    reject_duplicate_contract_field_names(named, attrs)?;

    for field in &named.named {
        if should_skip_contract_field(field)? {
            continue;
        }

        reject_directional_serde_skip(field)?;

        let field_ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(field, "expected named field"))?;
        let rust_name = rust_ident_name(field_ident);
        let wire_name =
            serde_rename(&field.attrs)?.unwrap_or_else(|| apply_rename_all(rename_all, &rust_name));
        field_wire_names.push((rust_name, wire_name));
    }

    for field in &named.named {
        if should_skip_contract_field(field)? {
            continue;
        }

        let field_ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(field, "expected named field"))?;
        let rust_field_name = rust_ident_name(field_ident);
        let field_name = field_wire_names
            .iter()
            .find_map(|(rust, wire)| (rust == &rust_field_name).then_some(wire.clone()))
            .expect("field wire name should be precomputed");
        let field_ty = &field.ty;
        let is_flattened = serde_has_flatten(&field.attrs)?;
        let has_default = struct_has_default || serde_has_default(&field.attrs)?;
        let has_sparse_serialization = serde_has_skip_serializing_if(&field.attrs)?;

        // Check if field is Option
        let (is_option, inner_ty) = if let Some(inner) = option_inner_type(field_ty) {
            (true, inner)
        } else {
            (false, field_ty)
        };

        // Generate schema for the inner type
        let schema_expr = type_to_schema_expr(inner_ty);

        if is_flattened {
            if deny_unknown_fields {
                reject_serde_flatten_with_deny_unknown_fields(field)?;
            }
            if is_option {
                return Err(syn::Error::new_spanned(
                    field,
                    "`#[serde(flatten)]` on `Option<T>` is not supported by Foundry's TypeScript export; flatten a struct with optional fields instead",
                ));
            }
            property_inserts.push(quote! {
                {
                    let flattened_schema = #schema_expr;
                    if let Some(flattened_properties) = flattened_schema
                        .get("properties")
                        .and_then(|value| value.as_object())
                    {
                        for (property_name, property_schema) in flattened_properties {
                            ::foundry::openapi::insert_json_schema_property(
                                &mut properties,
                                #name_str,
                                property_name,
                                property_schema.clone(),
                            );
                        }
                    }
                    if let Some(flattened_additional_properties) =
                        flattened_schema.get("additionalProperties")
                    {
                        additional_properties = Some(flattened_additional_properties.clone());
                    }
                    if let Some(flattened_required) = flattened_schema
                        .get("required")
                        .and_then(|value| value.as_array())
                    {
                        for required_name in flattened_required {
                            if let Some(required_name) = required_name.as_str() {
                                let required_name = required_name.to_string();
                                if !required.contains(&required_name) {
                                    required.push(required_name);
                                }
                            }
                        }
                    }
                }
            });
            continue;
        }

        // Parse #[validate(...)] attributes for constraints
        let constraints = parse_validate_constraints(&field.attrs)?;
        let is_required = constraints
            .iter()
            .any(|constraint| matches!(constraint, ValidateConstraint::Required));

        if has_default && !is_option && !is_required && !ts_has_optional(&field.attrs)? {
            return Err(syn::Error::new_spanned(
                field,
                "`#[serde(default)]` makes this field optional at runtime; add `#[ts(optional, as = \"Option<_>\")]` so generated TypeScript matches, or add `#[validate(required)]` if omission should fail validation",
            ));
        }

        if has_sparse_serialization && !ts_has_optional(&field.attrs)? {
            return Err(syn::Error::new_spanned(
                field,
                "`#[serde(skip_serializing_if)]` can omit this field at runtime; add `#[ts(optional)]` so generated TypeScript matches",
            ));
        }

        if is_required || (!is_option && !has_default && !has_sparse_serialization) {
            required_inserts.push(required_field_insert(&field_name));
        }

        let schema_context = SchemaFieldContext {
            rust_name: &rust_field_name,
            field_wire_names: &field_wire_names,
        };
        let file_constraints_apply_to_items = vec_inner_type(inner_ty)
            .map(type_is_uploaded_file)
            .unwrap_or(false);
        let constraint_inserts: Vec<TokenStream> = constraints
            .iter()
            .filter(|constraint| {
                !(file_constraints_apply_to_items && constraint.is_file_upload_constraint())
            })
            .filter_map(|c| c.to_schema_insert(Some(&schema_context)))
            .collect();
        let item_constraint_inserts: Vec<TokenStream> = constraints
            .iter()
            .filter(|constraint| {
                file_constraints_apply_to_items && constraint.is_file_upload_constraint()
            })
            .filter_map(|c| c.to_schema_insert(Some(&schema_context)))
            .collect();
        let item_constraint_block = if item_constraint_inserts.is_empty() {
            quote!()
        } else {
            quote! {
                if let Some(__foundry_items_obj) = obj
                    .get_mut("items")
                    .and_then(::foundry::serde_json::Value::as_object_mut)
                {
                    let obj = __foundry_items_obj;
                    #(#item_constraint_inserts)*
                }
            }
        };

        if is_option {
            property_inserts.push(quote! {
                {
                    let mut field_schema = #schema_expr;
                    if let Some(obj) = field_schema.as_object_mut() {
                        obj.insert("nullable".into(), ::foundry::serde_json::Value::Bool(true));
                        #(#constraint_inserts)*
                        #item_constraint_block
                    }
                    ::foundry::openapi::insert_json_schema_property(
                        &mut properties,
                        #name_str,
                        #field_name,
                        field_schema,
                    );
                }
            });
        } else {
            property_inserts.push(quote! {
                {
                    let mut field_schema = #schema_expr;
                    if let Some(obj) = field_schema.as_object_mut() {
                        #(#constraint_inserts)*
                        #item_constraint_block
                    }
                    ::foundry::openapi::insert_json_schema_property(
                        &mut properties,
                        #name_str,
                        #field_name,
                        field_schema,
                    );
                }
            });
        }
    }

    let struct_validation_inserts = after_hooks
        .iter()
        .map(|hook| {
            let hook_name = validation_hook_name(hook);
            foundry_validation_metadata_insert_dynamic_code(
                quote!("after"),
                vec![("hook", quote!(#hook_name))],
                None,
                true,
            )
        })
        .collect::<Vec<_>>();

    Ok(quote! {
        impl ::foundry::openapi::ApiSchema for #name {
            fn schema() -> ::foundry::serde_json::Value {
                let mut properties = ::foundry::serde_json::Map::new();
                let mut required = Vec::<String>::new();
                let mut additional_properties = if #deny_unknown_fields {
                    Some(::foundry::serde_json::Value::Bool(false))
                } else {
                    None
                };
                #(#property_inserts)*
                #(#required_inserts)*

                let mut schema_obj = ::foundry::serde_json::Map::new();
                schema_obj.insert("type".to_string(), ::foundry::serde_json::json!("object"));
                schema_obj.insert("properties".to_string(), ::foundry::serde_json::Value::Object(properties));
                if !required.is_empty() {
                    schema_obj.insert("required".to_string(), ::foundry::serde_json::json!(required));
                }
                if let Some(additional_properties) = additional_properties {
                    schema_obj.insert("additionalProperties".to_string(), additional_properties);
                }
                {
                    let obj = &mut schema_obj;
                    #(#struct_validation_inserts)*
                }
                ::foundry::serde_json::Value::Object(schema_obj)
            }

            fn schema_name() -> &'static str {
                #name_str
            }
        }
    })
}

fn required_field_insert(field_name: &str) -> TokenStream {
    quote! {
        {
            let required_name = #field_name.to_string();
            if !required.contains(&required_name) {
                required.push(required_name);
            }
        }
    }
}

fn type_is_uploaded_file(ty: &Type) -> bool {
    type_path_last_segment_matches(ty, "UploadedFile")
}

fn expand_enum(
    name: &syn::Ident,
    name_str: &str,
    data: &syn::DataEnum,
    attrs: &[syn::Attribute],
) -> syn::Result<TokenStream> {
    // Check if all variants are unit (no fields)
    let is_simple = data
        .variants
        .iter()
        .all(|v| matches!(v.fields, Fields::Unit));

    if !is_simple {
        return Err(syn::Error::new_spanned(
            name,
            "ApiSchema derive for enums only supports simple enums (unit variants)",
        ));
    }

    let rename_all = serde_rename_all(attrs)?;
    reject_duplicate_contract_variant_names(data, attrs)?;
    let variant_names: Vec<String> = data
        .variants
        .iter()
        .map(|variant| {
            let variant_name = rust_ident_name(&variant.ident);
            serde_rename(&variant.attrs)
                .map(|rename| rename.unwrap_or_else(|| apply_rename_all(rename_all, &variant_name)))
        })
        .collect::<syn::Result<_>>()?;

    Ok(quote! {
        impl ::foundry::openapi::ApiSchema for #name {
            fn schema() -> ::foundry::serde_json::Value {
                ::foundry::serde_json::json!({
                    "type": "string",
                    "enum": [#(#variant_names),*]
                })
            }

            fn schema_name() -> &'static str {
                #name_str
            }
        }
    })
}

/// Map a Rust type to its JSON Schema expression as a `TokenStream`.
fn type_to_schema_expr(ty: &Type) -> TokenStream {
    if matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty()) {
        return quote! { ::foundry::serde_json::json!({"type": "null"}) };
    }

    // Check Vec<T>
    if vec_inner_type(ty).is_some() {
        return quote! {
            <#ty as ::foundry::openapi::ApiSchema>::schema()
        };
    }

    // Check Option<T> (shouldn't happen at top level, handled above)
    if let Some(inner) = option_inner_type(ty) {
        let inner_expr = type_to_schema_expr(inner);
        return quote! {
            {
                let mut s = #inner_expr;
                if let Some(obj) = s.as_object_mut() {
                    obj.insert("nullable".into(), ::foundry::serde_json::Value::Bool(true));
                }
                s
            }
        };
    }

    // Primitive type checks
    if type_path_last_segment_matches(ty, "String") || type_path_last_segment_matches(ty, "str") {
        return quote! { ::foundry::serde_json::json!({"type": "string"}) };
    }
    if type_path_last_segment_matches(ty, "i8")
        || type_path_last_segment_matches(ty, "i16")
        || type_path_last_segment_matches(ty, "i32")
        || type_path_last_segment_matches(ty, "u8")
        || type_path_last_segment_matches(ty, "u16")
    {
        return quote! { ::foundry::serde_json::json!({"type": "integer", "format": "int32"}) };
    }
    if type_path_last_segment_matches(ty, "i64") || type_path_last_segment_matches(ty, "isize") {
        return quote! { ::foundry::serde_json::json!({"type": "integer", "format": "int64"}) };
    }
    if type_path_last_segment_matches(ty, "u32")
        || type_path_last_segment_matches(ty, "u64")
        || type_path_last_segment_matches(ty, "u128")
        || type_path_last_segment_matches(ty, "i128")
        || type_path_last_segment_matches(ty, "usize")
    {
        return quote! { ::foundry::serde_json::json!({"type": "integer"}) };
    }
    if type_path_last_segment_matches(ty, "f32") {
        return quote! { ::foundry::serde_json::json!({"type": "number", "format": "float"}) };
    }
    if type_path_last_segment_matches(ty, "f64") {
        return quote! { ::foundry::serde_json::json!({"type": "number", "format": "double"}) };
    }
    if type_path_last_segment_matches(ty, "bool") {
        return quote! { ::foundry::serde_json::json!({"type": "boolean"}) };
    }
    if type_path_last_segment_matches(ty, "DateTime") {
        return quote! { ::foundry::serde_json::json!({"type": "string", "format": "date-time"}) };
    }
    if type_path_last_segment_matches(ty, "Date") {
        return quote! { ::foundry::serde_json::json!({"type": "string", "format": "date"}) };
    }
    if type_path_last_segment_matches(ty, "Uuid") {
        return quote! { ::foundry::serde_json::json!({"type": "string", "format": "uuid"}) };
    }
    if type_argument_if_last_segment_ident(ty, "ModelId").is_some() {
        return quote! { ::foundry::serde_json::json!({"type": "string", "format": "uuid"}) };
    }
    if type_path_last_segment_matches(ty, "Value") {
        return quote! { ::foundry::serde_json::json!({}) };
    }

    // For types that implement ApiSchema, call their schema() at runtime.
    // This correctly resolves AppEnum, other ApiSchema structs, etc.
    quote! {
        <#ty as ::foundry::openapi::ApiSchema>::schema()
    }
}

// ---------------------------------------------------------------------------
// Validate constraint parsing
// ---------------------------------------------------------------------------

struct SchemaFieldContext<'a> {
    rust_name: &'a str,
    field_wire_names: &'a [(String, String)],
}

#[derive(Debug)]
enum ValidateConstraint {
    Required,
    Nullable,
    Bail,
    Filled,
    Email,
    Url,
    UuidFormat(Option<syn::Expr>),
    Ulid,
    HexColor,
    MacAddress,
    Numeric,
    Integer,
    Boolean,
    Accepted,
    Declined,
    Confirmed(Option<syn::Expr>),
    CustomRule(syn::Expr),
    Metadata {
        code: &'static str,
        params: Vec<(&'static str, syn::Expr)>,
        field_params: Vec<&'static str>,
        values: Vec<syn::Expr>,
        values_param: Option<&'static str>,
        values_are_field_refs: bool,
        server_only: bool,
    },
    ImageFile,
    Alpha,
    AlphaDash,
    AlphaNumeric,
    Ascii,
    Lowercase,
    Uppercase,
    Regex(syn::Expr),
    NotRegex(syn::Expr),
    StartsWith(Vec<syn::Expr>),
    DoesntStartWith(Vec<syn::Expr>),
    EndsWith(Vec<syn::Expr>),
    DoesntEndWith(Vec<syn::Expr>),
    Contains(Vec<syn::Expr>),
    DoesntContain(Vec<syn::Expr>),
    RequiredKeys(Vec<syn::Expr>),
    Digits,
    MinDigits(syn::Expr),
    MaxDigits(syn::Expr),
    DigitsBetween(syn::Expr, syn::Expr),
    Date,
    Time,
    DateTime,
    LocalDateTime,
    Timezone,
    Ip,
    Ipv4,
    Ipv6,
    Json,
    MinLength(syn::Expr),
    MaxLength(syn::Expr),
    Size(syn::Expr),
    MinItems(syn::Expr),
    MaxItems(syn::Expr),
    UniqueItems,
    Decimal(syn::Expr, syn::Expr),
    MinNumeric(syn::Expr),
    MaxNumeric(syn::Expr),
    MultipleOf(syn::Expr),
    Between(syn::Expr, syn::Expr),
    Gt(syn::Expr),
    Gte(syn::Expr),
    Lt(syn::Expr),
    Lte(syn::Expr),
    MaxFileSize(syn::Expr),
    MaxDimensions(syn::Expr, syn::Expr),
    MinDimensions(syn::Expr, syn::Expr),
    AllowedMimes(Vec<syn::Expr>),
    AllowedExtensions(Vec<syn::Expr>),
    InList(Vec<syn::Expr>),
    NotIn(Vec<syn::Expr>),
    AppEnum(syn::Path),
    Nested,
    Each(Vec<ValidateConstraint>),
}

impl ValidateConstraint {
    fn is_file_upload_constraint(&self) -> bool {
        matches!(
            self,
            Self::ImageFile
                | Self::MaxFileSize(_)
                | Self::MaxDimensions(_, _)
                | Self::MinDimensions(_, _)
                | Self::AllowedMimes(_)
                | Self::AllowedExtensions(_)
        )
    }

    fn to_schema_insert(&self, context: Option<&SchemaFieldContext<'_>>) -> Option<TokenStream> {
        match self {
            Self::Required => None, // handled separately via required array
            Self::Nullable => Some(foundry_validation_metadata_insert(
                "nullable",
                Vec::new(),
                None,
                false,
            )),
            Self::Bail => Some(foundry_validation_metadata_insert(
                "bail",
                Vec::new(),
                None,
                false,
            )),
            Self::Filled => Some(quote! {
                match obj.get("type").and_then(::foundry::serde_json::Value::as_str) {
                    Some("array") => {
                        let __foundry_min_items = obj
                            .get("minItems")
                            .and_then(::foundry::serde_json::Value::as_u64)
                            .unwrap_or(0)
                            .max(1);
                        obj.insert("minItems".into(), ::foundry::serde_json::json!(__foundry_min_items));
                    }
                    Some("string") => {
                        ::foundry::openapi::insert_json_schema_pattern(obj, r"\S");
                    }
                    _ => {}
                }
            }),
            Self::Email => Some(quote! {
                obj.insert("format".into(), ::foundry::serde_json::json!("email"));
            }),
            Self::Url => Some(quote! {
                obj.insert("format".into(), ::foundry::serde_json::json!("uri"));
            }),
            Self::UuidFormat(version) => {
                let version_pattern = version.as_ref().map(|version| {
                    quote! {
                        let __foundry_uuid_version = (#version) as u8;
                        if (1..=8).contains(&__foundry_uuid_version) {
                            let __foundry_uuid_version = format!("{:x}", __foundry_uuid_version);
                            let __foundry_canonical = format!(
                                "[0-9a-fA-F]{{8}}-[0-9a-fA-F]{{4}}-{}[0-9a-fA-F]{{3}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{12}}",
                                __foundry_uuid_version
                            );
                            let __foundry_compact = format!(
                                "[0-9a-fA-F]{{12}}{}[0-9a-fA-F]{{19}}",
                                __foundry_uuid_version
                            );
                            let __foundry_pattern = format!(
                                "^(?:{}|{}|\\{{{}\\}}|urn:uuid:{})$",
                                __foundry_compact,
                                __foundry_canonical,
                                __foundry_canonical,
                                __foundry_canonical
                            );
                            ::foundry::openapi::insert_json_schema_pattern(obj, __foundry_pattern);
                        }
                    }
                });
                Some(quote! {
                obj.insert("format".into(), ::foundry::serde_json::json!("uuid"));
                    #version_pattern
                })
            }
            Self::Ulid => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(
                    obj,
                    "^[0-7][0-9A-HJKMNP-TV-Za-hjkmnp-tv-z]{25}$",
                );
            }),
            Self::HexColor => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(
                    obj,
                    "^#(?:[0-9A-Fa-f]{3}|[0-9A-Fa-f]{4}|[0-9A-Fa-f]{6}|[0-9A-Fa-f]{8})$",
                );
            }),
            Self::MacAddress => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(
                    obj,
                    "^(?:(?:[0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}|(?:[0-9A-Fa-f]{2}-){5}[0-9A-Fa-f]{2})$",
                );
            }),
            Self::Numeric => Some(quote! {
                if obj.get("type") == Some(&::foundry::serde_json::json!("string")) {
                    ::foundry::openapi::insert_json_schema_pattern(
                        obj,
                        r"^[+-]?(?:(?:\d+(?:\.\d*)?)|(?:\.\d+))(?:[eE][+-]?\d+)?$",
                    );
                }
            }),
            Self::Integer => Some(quote! {
                match obj.get("type").and_then(::foundry::serde_json::Value::as_str) {
                    Some("string") => {
                        ::foundry::openapi::insert_json_schema_pattern(obj, r"^[+-]?\d+$");
                    }
                    Some("number") => {
                        obj.insert("multipleOf".into(), ::foundry::serde_json::json!(1));
                    }
                    _ => {}
                }
                obj.insert("x-foundry-integer-format".into(), ::foundry::serde_json::json!("i64"));
            }),
            Self::Boolean => Some(quote! {
                if obj.get("type") != Some(&::foundry::serde_json::json!("boolean")) {
                    obj.insert(
                        "enum".into(),
                        ::foundry::serde_json::json!(["true", "false", "1", "0"]),
                    );
                }
            }),
            Self::Accepted => Some(quote! {
                if obj.get("type") == Some(&::foundry::serde_json::json!("boolean")) {
                    obj.insert("enum".into(), ::foundry::serde_json::json!([true]));
                } else {
                    obj.insert(
                        "enum".into(),
                        ::foundry::serde_json::json!(["yes", "on", "1", "true"]),
                    );
                }
            }),
            Self::Declined => Some(quote! {
                if obj.get("type") == Some(&::foundry::serde_json::json!("boolean")) {
                    obj.insert("enum".into(), ::foundry::serde_json::json!([false]));
                } else {
                    obj.insert(
                        "enum".into(),
                        ::foundry::serde_json::json!(["no", "off", "0", "false"]),
                    );
                }
            }),
            Self::Confirmed(other) => {
                let other = match other {
                    Some(other) => context
                        .map(|context| field_reference_value(other, context.field_wire_names))
                        .unwrap_or_else(|| quote!((#other).to_string())),
                    None => {
                        let context = context?;
                        let confirmation = default_confirmation_wire_name(
                            context.rust_name,
                            context.field_wire_names,
                        );
                        quote!(#confirmation.to_string())
                    }
                };
                Some(foundry_validation_metadata_insert(
                    "confirmed",
                    vec![("other", other)],
                    None,
                    false,
                ))
            }
            Self::Nested => Some(foundry_validation_metadata_insert(
                "nested",
                Vec::new(),
                None,
                false,
            )),
            Self::CustomRule(rule) => {
                let rule_name = custom_rule_name_expr(rule);
                Some(foundry_validation_metadata_insert_dynamic_code(
                    rule_name.clone(),
                    vec![("rule", rule_name)],
                    None,
                    true,
                ))
            }
            Self::Metadata {
                code,
                params,
                field_params,
                values,
                values_param,
                values_are_field_refs,
                server_only,
            } => {
                let mut rendered_params = params
                    .iter()
                    .map(|(name, expr)| {
                        let value = if field_params.contains(name) {
                            context
                                .map(|context| {
                                    field_reference_value(expr, context.field_wire_names)
                                })
                                .unwrap_or_else(|| quote!((#expr).to_string()))
                        } else {
                            quote!((#expr).to_string())
                        };
                        (*name, value)
                    })
                    .collect::<Vec<_>>();
                let rendered_values = values
                    .iter()
                    .map(|value| {
                        if *values_are_field_refs {
                            context
                                .map(|context| {
                                    field_reference_value(value, context.field_wire_names)
                                })
                                .unwrap_or_else(|| quote!((#value).to_string()))
                        } else {
                            quote!((#value).to_string())
                        }
                    })
                    .collect::<Vec<_>>();
                if let Some(values_param) = values_param {
                    rendered_params.push((
                        *values_param,
                        quote!({
                            let __foundry_values: Vec<String> = vec![#(#rendered_values),*];
                            __foundry_values.join(", ")
                        }),
                    ));
                }
                let rendered_values = if rendered_values.is_empty() {
                    None
                } else {
                    Some(quote!(vec![#(#rendered_values),*]))
                };
                Some(foundry_validation_metadata_insert(
                    code,
                    rendered_params,
                    rendered_values,
                    *server_only,
                ))
            }
            Self::ImageFile => Some(quote! {
                ::foundry::openapi::insert_foundry_server_only_validation(obj, "image");
            }),
            Self::Alpha => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(obj, r"^[\p{L}\p{M}]*$");
            }),
            Self::AlphaDash => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(obj, r"^[\p{L}\p{M}\p{N}_-]*$");
            }),
            Self::AlphaNumeric => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(obj, r"^[\p{L}\p{M}\p{N}]*$");
            }),
            Self::Ascii => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(obj, r"^[\x00-\x7F]*$");
            }),
            Self::Lowercase => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(obj, r"^[^\p{Lu}]*$");
            }),
            Self::Uppercase => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(obj, r"^[^\p{Ll}]*$");
            }),
            Self::Regex(expr) => {
                let metadata_insert = foundry_validation_metadata_insert(
                    "regex",
                    vec![("pattern", quote!(__foundry_pattern.clone()))],
                    None,
                    true,
                );
                Some(quote! {
                    let __foundry_pattern = (#expr).to_string();
                    if ::foundry::typescript::rust_regex_is_client_compatible(&__foundry_pattern) {
                        ::foundry::openapi::insert_json_schema_pattern(obj, __foundry_pattern);
                    } else {
                        #metadata_insert
                    }
                })
            }
            Self::NotRegex(expr) => {
                let metadata_insert = foundry_validation_metadata_insert(
                    "not_regex",
                    vec![("pattern", quote!(__foundry_pattern.clone()))],
                    None,
                    true,
                );
                Some(quote! {
                    let __foundry_pattern = (#expr).to_string();
                    if ::foundry::typescript::rust_regex_is_client_compatible(&__foundry_pattern) {
                        ::foundry::openapi::insert_json_schema_not_any_pattern(
                            obj,
                            [__foundry_pattern],
                        );
                    } else {
                        #metadata_insert
                    }
                })
            }
            Self::StartsWith(exprs) => Some(quote! {
                let __foundry_patterns = vec![
                    #({
                        let __foundry_prefix = (#exprs).to_string();
                        ::std::format!(
                            "^{}",
                            ::foundry::openapi::escape_json_schema_pattern_literal(&__foundry_prefix)
                        )
                    }),*
                ];
                ::foundry::openapi::insert_json_schema_any_pattern(obj, __foundry_patterns);
            }),
            Self::DoesntStartWith(exprs) => Some(quote! {
                let __foundry_patterns = vec![
                    #({
                        let __foundry_prefix = (#exprs).to_string();
                        ::std::format!(
                            "^{}",
                            ::foundry::openapi::escape_json_schema_pattern_literal(&__foundry_prefix)
                        )
                    }),*
                ];
                ::foundry::openapi::insert_json_schema_not_any_pattern(obj, __foundry_patterns);
            }),
            Self::EndsWith(exprs) => Some(quote! {
                let __foundry_patterns = vec![
                    #({
                        let __foundry_suffix = (#exprs).to_string();
                        ::std::format!(
                            "{}$",
                            ::foundry::openapi::escape_json_schema_pattern_literal(&__foundry_suffix)
                        )
                    }),*
                ];
                ::foundry::openapi::insert_json_schema_any_pattern(obj, __foundry_patterns);
            }),
            Self::DoesntEndWith(exprs) => Some(quote! {
                let __foundry_patterns = vec![
                    #({
                        let __foundry_suffix = (#exprs).to_string();
                        ::std::format!(
                            "{}$",
                            ::foundry::openapi::escape_json_schema_pattern_literal(&__foundry_suffix)
                        )
                    }),*
                ];
                ::foundry::openapi::insert_json_schema_not_any_pattern(obj, __foundry_patterns);
            }),
            Self::Contains(exprs) => Some(quote! {
                let __foundry_values = vec![#((#exprs).to_string()),*];
                if obj.get("type") == Some(&::foundry::serde_json::json!("array")) {
                    ::foundry::openapi::insert_json_schema_array_contains_all(
                        obj,
                        __foundry_values,
                    );
                } else {
                    let __foundry_patterns = __foundry_values
                        .into_iter()
                        .map(|__foundry_needle| {
                            ::foundry::openapi::escape_json_schema_pattern_literal(&__foundry_needle)
                        })
                        .collect::<Vec<_>>();
                    ::foundry::openapi::insert_json_schema_any_pattern(obj, __foundry_patterns);
                }
            }),
            Self::DoesntContain(exprs) => Some(quote! {
                let __foundry_values = vec![#((#exprs).to_string()),*];
                if obj.get("type") == Some(&::foundry::serde_json::json!("array")) {
                    ::foundry::openapi::insert_json_schema_array_not_contains_any(
                        obj,
                        __foundry_values,
                    );
                } else {
                    let __foundry_patterns = __foundry_values
                        .into_iter()
                        .map(|__foundry_needle| {
                            ::foundry::openapi::escape_json_schema_pattern_literal(&__foundry_needle)
                        })
                        .collect::<Vec<_>>();
                    ::foundry::openapi::insert_json_schema_not_any_pattern(obj, __foundry_patterns);
                }
            }),
            Self::RequiredKeys(exprs) => Some(quote! {
                let __foundry_values = vec![#((#exprs).to_string()),*];
                if obj.get("type") == Some(&::foundry::serde_json::json!("object")) {
                    let __foundry_required = obj
                        .entry("required".to_string())
                        .or_insert_with(|| ::foundry::serde_json::json!([]));
                    if let Some(__foundry_required) = __foundry_required.as_array_mut() {
                        for __foundry_key in &__foundry_values {
                            if !__foundry_required
                                .iter()
                                .any(|value| value.as_str() == Some(__foundry_key.as_str()))
                            {
                                __foundry_required
                                    .push(::foundry::serde_json::Value::String(__foundry_key.clone()));
                            }
                        }
                    }
                }
                ::foundry::openapi::insert_foundry_validation_rule(
                    obj,
                    ::foundry::serde_json::json!({
                        "code": "required_keys",
                        "values": __foundry_values,
                    }),
                );
            }),
            Self::Digits => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(obj, "^[0-9]*$");
            }),
            Self::MinDigits(expr) => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(obj, "^[0-9]*$");
                obj.insert("minLength".into(), ::foundry::serde_json::json!((#expr) as usize));
            }),
            Self::MaxDigits(expr) => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(obj, "^[0-9]*$");
                obj.insert("maxLength".into(), ::foundry::serde_json::json!((#expr) as usize));
            }),
            Self::DigitsBetween(min, max) => Some(quote! {
                ::foundry::openapi::insert_json_schema_pattern(obj, "^[0-9]*$");
                obj.insert("minLength".into(), ::foundry::serde_json::json!((#min) as usize));
                obj.insert("maxLength".into(), ::foundry::serde_json::json!((#max) as usize));
            }),
            Self::Date => Some(quote! {
                obj.insert("format".into(), ::foundry::serde_json::json!("date"));
            }),
            Self::Time => Some(quote! {
                obj.insert("format".into(), ::foundry::serde_json::json!("time"));
            }),
            Self::DateTime | Self::LocalDateTime => Some(quote! {
                obj.insert("format".into(), ::foundry::serde_json::json!("date-time"));
            }),
            Self::Timezone => Some(quote! {
                obj.insert("format".into(), ::foundry::serde_json::json!("timezone"));
            }),
            Self::Ip => Some(quote! {
                obj.insert("format".into(), ::foundry::serde_json::json!("ip"));
            }),
            Self::Ipv4 => Some(quote! {
                obj.insert("format".into(), ::foundry::serde_json::json!("ipv4"));
            }),
            Self::Ipv6 => Some(quote! {
                obj.insert("format".into(), ::foundry::serde_json::json!("ipv6"));
            }),
            Self::Json => Some(quote! {
                if obj.get("type") == Some(&::foundry::serde_json::json!("string")) {
                    obj.insert("format".into(), ::foundry::serde_json::json!("json-string"));
                } else {
                    ::foundry::openapi::insert_foundry_validation_rule(
                        obj,
                        ::foundry::serde_json::json!({
                            "code": "json",
                            "serverOnly": true,
                        }),
                    );
                }
            }),
            Self::MinLength(expr) => Some(quote! {
                obj.insert("minLength".into(), ::foundry::serde_json::json!(#expr));
            }),
            Self::MaxLength(expr) => Some(quote! {
                obj.insert("maxLength".into(), ::foundry::serde_json::json!(#expr));
            }),
            Self::Size(expr) => Some(quote! {
                match obj.get("type").and_then(::foundry::serde_json::Value::as_str) {
                    Some("array") => {
                        let __foundry_size = (#expr) as usize;
                        obj.insert("minItems".into(), ::foundry::serde_json::json!(__foundry_size));
                        obj.insert("maxItems".into(), ::foundry::serde_json::json!(__foundry_size));
                    }
                    Some("integer") | Some("number") => {
                        let __foundry_size = (#expr) as f64;
                        obj.insert("minimum".into(), ::foundry::serde_json::json!(__foundry_size));
                        obj.insert("maximum".into(), ::foundry::serde_json::json!(__foundry_size));
                    }
                    Some("string") => {
                        let __foundry_size = (#expr) as usize;
                        obj.insert("minLength".into(), ::foundry::serde_json::json!(__foundry_size));
                        obj.insert("maxLength".into(), ::foundry::serde_json::json!(__foundry_size));
                    }
                    _ => {}
                }
            }),
            Self::MinItems(expr) => Some(quote! {
                obj.insert("minItems".into(), ::foundry::serde_json::json!(#expr));
            }),
            Self::MaxItems(expr) => Some(quote! {
                obj.insert("maxItems".into(), ::foundry::serde_json::json!(#expr));
            }),
            Self::UniqueItems => Some(quote! {
                obj.insert("uniqueItems".into(), ::foundry::serde_json::json!(true));
            }),
            Self::Decimal(min, max) => Some(quote! {
                let __foundry_min = (#min) as usize;
                let __foundry_max = (#max) as usize;
                let __foundry_pattern = if __foundry_min > __foundry_max {
                    "a^".to_string()
                } else if __foundry_min == __foundry_max {
                    if __foundry_min == 0 {
                        "^[+-]?[0-9]+\\.$".to_string()
                    } else {
                        ::std::format!(
                            "^[+-]?(?:[0-9]+\\.[0-9]{{{0}}}|\\.[0-9]{{{0}}})$",
                            __foundry_min
                        )
                    }
                } else if __foundry_min == 0 {
                    ::std::format!(
                        "^[+-]?(?:[0-9]+\\.[0-9]{{0,{0}}}|\\.[0-9]{{1,{0}}})$",
                        __foundry_max
                    )
                } else {
                    ::std::format!(
                        "^[+-]?(?:[0-9]+\\.[0-9]{{{0},{1}}}|\\.[0-9]{{{0},{1}}})$",
                        __foundry_min,
                        __foundry_max
                    )
                };
                ::foundry::openapi::insert_json_schema_pattern(obj, __foundry_pattern);
            }),
            Self::MinNumeric(expr) => Some(quote! {
                obj.insert("minimum".into(), ::foundry::serde_json::json!(#expr));
            }),
            Self::MaxNumeric(expr) => Some(quote! {
                obj.insert("maximum".into(), ::foundry::serde_json::json!(#expr));
            }),
            Self::MultipleOf(expr) => Some(quote! {
                let __foundry_multiple_of = (#expr) as f64;
                if __foundry_multiple_of.is_finite() && __foundry_multiple_of > 0.0 {
                    obj.insert(
                        "multipleOf".into(),
                        ::foundry::serde_json::json!(__foundry_multiple_of),
                    );
                }
            }),
            Self::Between(min, max) => Some(quote! {
                obj.insert("minimum".into(), ::foundry::serde_json::json!(#min));
                obj.insert("maximum".into(), ::foundry::serde_json::json!(#max));
            }),
            Self::Gt(expr) => Some(quote! {
                obj.insert("exclusiveMinimum".into(), ::foundry::serde_json::json!(#expr));
            }),
            Self::Gte(expr) => Some(quote! {
                obj.insert("minimum".into(), ::foundry::serde_json::json!(#expr));
            }),
            Self::Lt(expr) => Some(quote! {
                obj.insert("exclusiveMaximum".into(), ::foundry::serde_json::json!(#expr));
            }),
            Self::Lte(expr) => Some(quote! {
                obj.insert("maximum".into(), ::foundry::serde_json::json!(#expr));
            }),
            Self::MaxFileSize(expr) => Some(quote! {
                obj.insert(
                    "x-foundry-max-file-size-kb".into(),
                    ::foundry::serde_json::json!(#expr),
                );
            }),
            Self::MaxDimensions(width, height) => Some(quote! {
                let __foundry_width = (#width) as u32;
                let __foundry_height = (#height) as u32;
                obj.insert(
                    "x-foundry-max-dimensions".into(),
                    ::foundry::serde_json::json!({
                        "width": __foundry_width,
                        "height": __foundry_height,
                    }),
                );
                ::foundry::openapi::insert_foundry_server_only_validation(obj, "max_dimensions");
            }),
            Self::MinDimensions(width, height) => Some(quote! {
                let __foundry_width = (#width) as u32;
                let __foundry_height = (#height) as u32;
                obj.insert(
                    "x-foundry-min-dimensions".into(),
                    ::foundry::serde_json::json!({
                        "width": __foundry_width,
                        "height": __foundry_height,
                    }),
                );
                ::foundry::openapi::insert_foundry_server_only_validation(obj, "min_dimensions");
            }),
            Self::AllowedMimes(values) => Some(quote! {
                obj.insert(
                    "x-foundry-allowed-mimes".into(),
                    ::foundry::serde_json::json!(vec![#((#values).to_string()),*]),
                );
                ::foundry::openapi::insert_foundry_server_only_validation(obj, "allowed_mimes");
            }),
            Self::AllowedExtensions(values) => Some(quote! {
                obj.insert(
                    "x-foundry-allowed-extensions".into(),
                    ::foundry::serde_json::json!(vec![#((#values).to_string()),*]),
                );
            }),
            Self::InList(values) => Some(quote! {
                obj.insert(
                    "enum".into(),
                    ::foundry::openapi::json_schema_enum_values_for_schema(
                        obj,
                        vec![#((#values).to_string()),*],
                    ),
                );
            }),
            Self::NotIn(values) => Some(quote! {
                let __foundry_values = ::foundry::openapi::json_schema_enum_values_for_schema(
                    obj,
                    vec![#((#values).to_string()),*],
                );
                obj.insert(
                    "not".into(),
                    ::foundry::serde_json::json!({
                        "enum": __foundry_values,
                    }),
                );
            }),
            Self::AppEnum(type_path) => Some(quote! {
                if !obj.contains_key("enum") {
                    let __foundry_values = match obj
                        .get("type")
                        .and_then(::foundry::serde_json::Value::as_str)
                    {
                        Some("integer") | Some("number") => {
                            let __foundry_meta = <#type_path as ::foundry::FoundryAppEnum>::meta();
                            let __foundry_values = __foundry_meta
                                .options
                                .iter()
                                .map(|__option| match &__option.value {
                                    ::foundry::EnumKey::String(__value) => __value.clone(),
                                    ::foundry::EnumKey::Int(__value) => __value.to_string(),
                                })
                                .collect::<Vec<String>>();
                            ::foundry::openapi::json_schema_enum_values_for_schema(obj, __foundry_values)
                        }
                        _ => ::foundry::openapi::json_schema_enum_values_for_schema(
                            obj,
                            <#type_path as ::foundry::FoundryAppEnum>::accepted_keys().into_vec(),
                        ),
                    };
                    obj.insert("enum".into(), __foundry_values);
                }
            }),
            Self::Each(constraints) => {
                let item_inserts = constraints
                    .iter()
                    .filter_map(|constraint| constraint.to_schema_insert(None))
                    .collect::<Vec<_>>();
                if item_inserts.is_empty() {
                    None
                } else {
                    Some(quote! {
                        if let Some(obj) = obj
                            .get_mut("items")
                            .and_then(::foundry::serde_json::Value::as_object_mut)
                        {
                            #(#item_inserts)*
                        }
                    })
                }
            }
        }
    }
}

fn foundry_validation_metadata_insert(
    code: &'static str,
    params: Vec<(&'static str, TokenStream)>,
    values: Option<TokenStream>,
    server_only: bool,
) -> TokenStream {
    foundry_validation_metadata_insert_dynamic_code(quote!(#code), params, values, server_only)
}

fn foundry_validation_metadata_insert_dynamic_code(
    code: TokenStream,
    params: Vec<(&'static str, TokenStream)>,
    values: Option<TokenStream>,
    server_only: bool,
) -> TokenStream {
    let param_inserts = params.iter().map(|(name, value)| {
        quote! {
            __foundry_params.insert(
                #name.to_string(),
                ::foundry::serde_json::Value::String((#value).to_string()),
            );
        }
    });
    let params_insert = if params.is_empty() {
        None
    } else {
        Some(quote! {
            let mut __foundry_params = ::foundry::serde_json::Map::new();
            #(#param_inserts)*
            __foundry_rule.insert(
                "params".to_string(),
                ::foundry::serde_json::Value::Object(__foundry_params),
            );
        })
    };
    let values_insert = values.map(|values| {
        quote! {
            __foundry_rule.insert("values".to_string(), ::foundry::serde_json::json!(#values));
        }
    });
    let server_only_insert = server_only.then(|| {
        quote! {
            __foundry_rule.insert("serverOnly".to_string(), ::foundry::serde_json::json!(true));
        }
    });

    quote! {
        {
            let mut __foundry_rule = ::foundry::serde_json::Map::new();
            __foundry_rule.insert(
                "code".to_string(),
                ::foundry::serde_json::Value::String((#code).to_string()),
            );
            #params_insert
            #values_insert
            #server_only_insert
            ::foundry::openapi::insert_foundry_validation_rule(
                obj,
                ::foundry::serde_json::Value::Object(__foundry_rule),
            );
        }
    }
}

fn custom_rule_name_expr(expr: &syn::Expr) -> TokenStream {
    string_literal_expr(expr)
        .map(|rule| quote!(#rule))
        .unwrap_or_else(|| quote!((#expr).as_str()))
}

fn validation_hook_name(hook: &syn::Path) -> String {
    hook.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn field_reference_value(expr: &syn::Expr, field_wire_names: &[(String, String)]) -> TokenStream {
    if let Some(rust_field_name) = string_literal_expr(expr) {
        let wire_name = wire_field_name(&rust_field_name, field_wire_names);
        quote!(#wire_name.to_string())
    } else {
        quote!((#expr).to_string())
    }
}

fn default_confirmation_wire_name(
    rust_field_name: &str,
    field_wire_names: &[(String, String)],
) -> String {
    let confirmation = format!("{rust_field_name}_confirmation");
    wire_field_name(&confirmation, field_wire_names)
}

fn string_literal_expr(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Lit(expr) => match &expr.lit {
            syn::Lit::Str(lit) => Some(lit.value()),
            _ => None,
        },
        _ => None,
    }
}

fn parse_struct_after_hooks(attrs: &[syn::Attribute]) -> syn::Result<Vec<syn::Path>> {
    let mut hooks = Vec::new();
    let mut seen_messages = HashSet::new();
    let mut seen_attributes = HashSet::new();

    for attr in attrs.iter().filter(|attr| attr.path().is_ident("validate")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("after") {
                let content;
                syn::parenthesized!(content in meta.input);
                let hook: syn::Path = content.parse()?;
                if !content.is_empty() {
                    return Err(syn::Error::new(
                        content.span(),
                        "after(...) accepts exactly one validation hook path",
                    ));
                }
                hooks.push(hook);
            } else if meta.path.is_ident("messages") {
                meta.parse_nested_meta(|field_meta| {
                    let field = field_meta
                        .path
                        .get_ident()
                        .ok_or_else(|| {
                            syn::Error::new_spanned(&field_meta.path, "expected field name")
                        })?
                        .to_string();
                    field_meta.parse_nested_meta(|rule_meta| {
                        let rule = rule_meta
                            .path
                            .get_ident()
                            .ok_or_else(|| {
                                syn::Error::new_spanned(&rule_meta.path, "expected rule name")
                            })?
                            .to_string();
                        let _: syn::Token![=] = rule_meta.input.parse()?;
                        let value: syn::LitStr = rule_meta.input.parse()?;
                        ensure_non_blank_lit_str(
                            value,
                            &format!("validation message for field `{field}` rule `{rule}`"),
                        )?;
                        if !seen_messages.insert((field.clone(), rule.clone())) {
                            return Err(syn::Error::new(
                                rule_meta.path.span(),
                                format!(
                                    "duplicate validation message for field `{field}` rule `{rule}`"
                                ),
                            ));
                        }
                        Ok(())
                    })
                })?;
            } else if meta.path.is_ident("attributes") {
                meta.parse_nested_meta(|inner| {
                    let field = inner
                        .path
                        .get_ident()
                        .ok_or_else(|| {
                            syn::Error::new_spanned(
                                &inner.path,
                                "attributes key must be a single identifier",
                            )
                        })?
                        .to_string();
                    let _: syn::Token![=] = inner.input.parse()?;
                    let value: syn::LitStr = inner.input.parse()?;
                    ensure_non_blank_lit_str(
                        value,
                        &format!("validation attribute label for field `{field}`"),
                    )?;
                    if !seen_attributes.insert(field.clone()) {
                        return Err(syn::Error::new(
                            inner.path.span(),
                            format!("duplicate validation attribute label for field `{field}`"),
                        ));
                    }
                    Ok(())
                })?;
            } else {
                return Err(meta.error(
                    "unsupported validate struct attribute; expected messages(...), attributes(...), or after(...)",
                ));
            }
            Ok(())
        })?;
    }

    Ok(hooks)
}

fn validation_rule_code(name: &str) -> &'static str {
    match name {
        "required_if" => "required_if",
        "required_unless" => "required_unless",
        "accepted_if" => "accepted_if",
        "declined_if" => "declined_if",
        "prohibited_if" => "prohibited_if",
        "prohibited_unless" => "prohibited_unless",
        "required_if_accepted" => "required_if_accepted",
        "required_if_declined" => "required_if_declined",
        "required_with" => "required_with",
        "required_with_all" => "required_with_all",
        "required_without" => "required_without",
        "required_without_all" => "required_without_all",
        "prohibited_if_accepted" => "prohibited_if_accepted",
        "prohibited_if_declined" => "prohibited_if_declined",
        "prohibits" => "prohibits",
        "same" => "same",
        "different" => "different",
        "before" => "before",
        "before_or_equal" => "before_or_equal",
        "after" => "after",
        "after_or_equal" => "after_or_equal",
        "date_equals" => "date_equals",
        "unique" => "unique",
        "exists" => "exists",
        _ => unreachable!("unknown OpenAPI validation metadata rule"),
    }
}

fn parse_validate_constraints(attrs: &[syn::Attribute]) -> syn::Result<Vec<ValidateConstraint>> {
    let mut constraints = Vec::new();

    for attr in attrs.iter().filter(|a| a.path().is_ident("validate")) {
        attr.parse_args_with(|input: syn::parse::ParseStream<'_>| {
            parse_validate_constraint_stream(input, &mut constraints)
        })?;
    }

    Ok(constraints)
}

fn parse_validate_constraint_stream(
    input: syn::parse::ParseStream<'_>,
    constraints: &mut Vec<ValidateConstraint>,
) -> syn::Result<()> {
    while !input.is_empty() {
        let ident: syn::Ident = input.parse()?;
        let name = ident.to_string();

        match name.as_str() {
            "required" => {
                constraints.push(ValidateConstraint::Required);
                consume_optional_validate_args(input)?;
            }
            "nullable" => {
                constraints.push(ValidateConstraint::Nullable);
                consume_optional_validate_args(input)?;
            }
            "bail" => {
                constraints.push(ValidateConstraint::Bail);
                consume_optional_validate_args(input)?;
            }
            "filled" => {
                constraints.push(ValidateConstraint::Filled);
                consume_optional_validate_args(input)?;
            }
            "email" => {
                constraints.push(ValidateConstraint::Email);
                consume_optional_validate_args(input)?;
            }
            "url" => {
                constraints.push(ValidateConstraint::Url);
                consume_optional_validate_args(input)?;
            }
            "uuid" => {
                let version = parse_optional_single_validate_arg(input, "uuid")?;
                constraints.push(ValidateConstraint::UuidFormat(version));
            }
            "ulid" => {
                constraints.push(ValidateConstraint::Ulid);
                consume_optional_validate_args(input)?;
            }
            "hex_color" => {
                constraints.push(ValidateConstraint::HexColor);
                consume_optional_validate_args(input)?;
            }
            "mac_address" => {
                constraints.push(ValidateConstraint::MacAddress);
                consume_optional_validate_args(input)?;
            }
            "numeric" => {
                constraints.push(ValidateConstraint::Numeric);
                consume_optional_validate_args(input)?;
            }
            "integer" => {
                constraints.push(ValidateConstraint::Integer);
                consume_optional_validate_args(input)?;
            }
            "boolean" => {
                constraints.push(ValidateConstraint::Boolean);
                consume_optional_validate_args(input)?;
            }
            "accepted" => {
                constraints.push(ValidateConstraint::Accepted);
                consume_optional_validate_args(input)?;
            }
            "declined" => {
                constraints.push(ValidateConstraint::Declined);
                consume_optional_validate_args(input)?;
            }
            "confirmed" => {
                let other = parse_optional_single_validate_arg(input, "confirmed")?;
                constraints.push(ValidateConstraint::Confirmed(other));
            }
            "prohibited" => {
                constraints.push(ValidateConstraint::Metadata {
                    code: "prohibited",
                    params: Vec::new(),
                    field_params: Vec::new(),
                    values: Vec::new(),
                    values_param: None,
                    values_are_field_refs: false,
                    server_only: false,
                });
                consume_optional_validate_args(input)?;
            }
            "required_if" | "required_unless" | "accepted_if" | "declined_if" | "prohibited_if"
            | "prohibited_unless" => {
                let (other, value) = parse_two_validate_args(input, &name)?;
                constraints.push(ValidateConstraint::Metadata {
                    code: validation_rule_code(&name),
                    params: vec![("other", other), ("value", value)],
                    field_params: vec!["other"],
                    values: Vec::new(),
                    values_param: None,
                    values_are_field_refs: false,
                    server_only: false,
                });
            }
            "required_if_accepted"
            | "required_if_declined"
            | "required_with"
            | "required_without"
            | "prohibited_if_accepted"
            | "prohibited_if_declined" => {
                let other = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::Metadata {
                    code: validation_rule_code(&name),
                    params: vec![("other", other)],
                    field_params: vec!["other"],
                    values: Vec::new(),
                    values_param: None,
                    values_are_field_refs: false,
                    server_only: false,
                });
            }
            "required_with_all" | "required_without_all" | "prohibits" => {
                let values = parse_validate_arg_list(input, &name)?;
                constraints.push(ValidateConstraint::Metadata {
                    code: validation_rule_code(&name),
                    params: Vec::new(),
                    field_params: Vec::new(),
                    values,
                    values_param: Some("other"),
                    values_are_field_refs: true,
                    server_only: false,
                });
            }
            "image" => {
                constraints.push(ValidateConstraint::ImageFile);
                reject_validate_args(input, "image")?;
            }
            "alpha" => {
                constraints.push(ValidateConstraint::Alpha);
                consume_optional_validate_args(input)?;
            }
            "alpha_dash" => {
                constraints.push(ValidateConstraint::AlphaDash);
                consume_optional_validate_args(input)?;
            }
            "alpha_num" | "alpha_numeric" => {
                constraints.push(ValidateConstraint::AlphaNumeric);
                consume_optional_validate_args(input)?;
            }
            "ascii" => {
                constraints.push(ValidateConstraint::Ascii);
                consume_optional_validate_args(input)?;
            }
            "lowercase" => {
                constraints.push(ValidateConstraint::Lowercase);
                consume_optional_validate_args(input)?;
            }
            "uppercase" => {
                constraints.push(ValidateConstraint::Uppercase);
                consume_optional_validate_args(input)?;
            }
            "regex" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::Regex(expr));
            }
            "not_regex" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::NotRegex(expr));
            }
            "starts_with" => {
                let values = parse_validate_arg_list(input, &name)?;
                constraints.push(ValidateConstraint::StartsWith(values));
            }
            "doesnt_start_with" => {
                let values = parse_validate_arg_list(input, &name)?;
                constraints.push(ValidateConstraint::DoesntStartWith(values));
            }
            "ends_with" => {
                let values = parse_validate_arg_list(input, &name)?;
                constraints.push(ValidateConstraint::EndsWith(values));
            }
            "doesnt_end_with" => {
                let values = parse_validate_arg_list(input, &name)?;
                constraints.push(ValidateConstraint::DoesntEndWith(values));
            }
            "contains" => {
                let values = parse_validate_arg_list(input, &name)?;
                constraints.push(ValidateConstraint::Contains(values));
            }
            "doesnt_contain" => {
                let values = parse_validate_arg_list(input, &name)?;
                constraints.push(ValidateConstraint::DoesntContain(values));
            }
            "required_keys" => {
                let values = parse_validate_arg_list(input, &name)?;
                constraints.push(ValidateConstraint::RequiredKeys(values));
            }
            "digits" => {
                constraints.push(ValidateConstraint::Digits);
                consume_optional_validate_args(input)?;
            }
            "min_digits" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::MinDigits(expr));
            }
            "max_digits" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::MaxDigits(expr));
            }
            "digits_between" => {
                let (min, max) = parse_two_validate_args(input, &name)?;
                constraints.push(ValidateConstraint::DigitsBetween(min, max));
            }
            "date" => {
                constraints.push(ValidateConstraint::Date);
                consume_optional_validate_args(input)?;
            }
            "time" => {
                constraints.push(ValidateConstraint::Time);
                consume_optional_validate_args(input)?;
            }
            "datetime" => {
                constraints.push(ValidateConstraint::DateTime);
                consume_optional_validate_args(input)?;
            }
            "local_datetime" => {
                constraints.push(ValidateConstraint::LocalDateTime);
                consume_optional_validate_args(input)?;
            }
            "timezone" => {
                constraints.push(ValidateConstraint::Timezone);
                consume_optional_validate_args(input)?;
            }
            "same" | "different" | "before" | "before_or_equal" | "after" | "after_or_equal"
            | "date_equals" => {
                let other = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::Metadata {
                    code: validation_rule_code(&name),
                    params: vec![("other", other)],
                    field_params: vec!["other"],
                    values: Vec::new(),
                    values_param: None,
                    values_are_field_refs: false,
                    server_only: false,
                });
            }
            "ip" => {
                constraints.push(ValidateConstraint::Ip);
                consume_optional_validate_args(input)?;
            }
            "ipv4" => {
                constraints.push(ValidateConstraint::Ipv4);
                consume_optional_validate_args(input)?;
            }
            "ipv6" => {
                constraints.push(ValidateConstraint::Ipv6);
                consume_optional_validate_args(input)?;
            }
            "json" => {
                constraints.push(ValidateConstraint::Json);
                consume_optional_validate_args(input)?;
            }
            "min" | "min_length" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::MinLength(expr));
            }
            "max" | "max_length" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::MaxLength(expr));
            }
            "size" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::Size(expr));
            }
            "min_items" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::MinItems(expr));
            }
            "max_items" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::MaxItems(expr));
            }
            "distinct" => {
                constraints.push(ValidateConstraint::UniqueItems);
                consume_optional_validate_args(input)?;
            }
            "decimal" => {
                let (min, max) = parse_one_or_two_validate_args(input, &name)?;
                constraints.push(ValidateConstraint::Decimal(min, max));
            }
            "min_numeric" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::MinNumeric(expr));
            }
            "max_numeric" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::MaxNumeric(expr));
            }
            "multiple_of" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::MultipleOf(expr));
            }
            "between" => {
                let (min, max) = parse_two_validate_args(input, &name)?;
                constraints.push(ValidateConstraint::Between(min, max));
            }
            "gt" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::Gt(expr));
            }
            "gte" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::Gte(expr));
            }
            "lt" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::Lt(expr));
            }
            "lte" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::Lte(expr));
            }
            "max_file_size" => {
                let expr = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::MaxFileSize(expr));
            }
            "max_dimensions" => {
                let (width, height) = parse_two_validate_args(input, &name)?;
                constraints.push(ValidateConstraint::MaxDimensions(width, height));
            }
            "min_dimensions" => {
                let (width, height) = parse_two_validate_args(input, &name)?;
                constraints.push(ValidateConstraint::MinDimensions(width, height));
            }
            "allowed_mimes" => {
                let values = parse_validate_arg_list(input, &name)?;
                constraints.push(ValidateConstraint::AllowedMimes(values));
            }
            "allowed_extensions" => {
                let values = parse_validate_arg_list(input, &name)?;
                constraints.push(ValidateConstraint::AllowedExtensions(values));
            }
            "in_list" => {
                let values = parse_validate_arg_list(input, &name)?;
                constraints.push(ValidateConstraint::InList(values));
            }
            "not_in" => {
                let values = parse_validate_arg_list(input, &name)?;
                constraints.push(ValidateConstraint::NotIn(values));
            }
            "app_enum" => {
                let type_path = parse_app_enum_arg(input, &name)?;
                constraints.push(ValidateConstraint::AppEnum(type_path));
            }
            "nested" => {
                constraints.push(ValidateConstraint::Nested);
                reject_validate_args(input, "nested")?;
            }
            "unique" | "exists" => {
                let (table, column) = parse_two_validate_args(input, &name)?;
                constraints.push(ValidateConstraint::Metadata {
                    code: validation_rule_code(&name),
                    params: vec![("table", table), ("column", column)],
                    field_params: Vec::new(),
                    values: Vec::new(),
                    values_param: None,
                    values_are_field_refs: false,
                    server_only: true,
                });
            }
            "rule" => {
                let rule = parse_first_validate_arg(input, &name)?;
                constraints.push(ValidateConstraint::CustomRule(rule));
            }
            "each" => {
                let content;
                syn::parenthesized!(content in input);
                let mut item_constraints = Vec::new();
                parse_validate_constraint_stream(&content, &mut item_constraints)?;
                if item_constraints.is_empty() {
                    return Err(syn::Error::new(
                        content.span(),
                        "validation rule `each` requires at least one nested rule",
                    ));
                }
                constraints.push(ValidateConstraint::Each(item_constraints));
            }
            _ => {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("unknown validation rule `{name}`"),
                ));
            }
        }

        if !input.is_empty() {
            let _: syn::Token![,] = input.parse()?;
        }
    }
    Ok(())
}

fn consume_optional_validate_args(input: syn::parse::ParseStream<'_>) -> syn::Result<()> {
    if input.peek(syn::token::Paren) {
        let content;
        syn::parenthesized!(content in input);
        let args = parse_validate_args(&content)?;
        if let Some(arg) = args.first() {
            return Err(syn::Error::new_spanned(
                arg,
                "validation rule accepts no positional arguments; use `message = \"...\"` for message overrides",
            ));
        }
    }
    Ok(())
}

fn reject_validate_args(input: syn::parse::ParseStream<'_>, rule_name: &str) -> syn::Result<()> {
    if input.peek(syn::token::Paren) {
        return Err(syn::Error::new(
            input.span(),
            format!("{rule_name} rule takes no arguments"),
        ));
    }
    Ok(())
}

fn parse_optional_single_validate_arg(
    input: syn::parse::ParseStream<'_>,
    rule_name: &str,
) -> syn::Result<Option<syn::Expr>> {
    if !input.peek(syn::token::Paren) {
        return Ok(None);
    }

    let content;
    syn::parenthesized!(content in input);
    let args = parse_validate_args(&content)?;
    match args.as_slice() {
        [] => Ok(None),
        [arg] => Ok(Some(arg.clone())),
        _ => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("validation rule `{rule_name}` accepts at most one schema constraint argument"),
        )),
    }
}

fn parse_first_validate_arg(
    input: syn::parse::ParseStream<'_>,
    rule_name: &str,
) -> syn::Result<syn::Expr> {
    let content;
    syn::parenthesized!(content in input);
    let args = parse_validate_args(&content)?;
    let [arg]: [syn::Expr; 1] = args.try_into().map_err(|args: Vec<syn::Expr>| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "validation rule `{rule_name}` requires exactly one schema constraint argument, got {}",
                args.len()
            ),
        )
    })?;
    Ok(arg)
}

fn parse_two_validate_args(
    input: syn::parse::ParseStream<'_>,
    rule_name: &str,
) -> syn::Result<(syn::Expr, syn::Expr)> {
    let content;
    syn::parenthesized!(content in input);
    let args = parse_validate_args(&content)?;
    let [min, max]: [syn::Expr; 2] = args.try_into().map_err(|args: Vec<syn::Expr>| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "validation rule `{rule_name}` requires exactly two schema constraint arguments, got {}",
                args.len()
            ),
        )
    })?;
    Ok((min, max))
}

fn parse_one_or_two_validate_args(
    input: syn::parse::ParseStream<'_>,
    rule_name: &str,
) -> syn::Result<(syn::Expr, syn::Expr)> {
    let content;
    syn::parenthesized!(content in input);
    let args = parse_validate_args(&content)?;
    match args.as_slice() {
        [min] => Ok((min.clone(), min.clone())),
        [min, max] => Ok((min.clone(), max.clone())),
        _ => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "validation rule `{rule_name}` requires one or two schema constraint arguments, got {}",
                args.len()
            ),
        )),
    }
}

fn parse_validate_arg_list(
    input: syn::parse::ParseStream<'_>,
    rule_name: &str,
) -> syn::Result<Vec<syn::Expr>> {
    let content;
    syn::parenthesized!(content in input);
    let args = parse_validate_args(&content)?;
    if args.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("validation rule `{rule_name}` requires at least one schema value"),
        ));
    }
    Ok(args)
}

fn parse_app_enum_arg(
    input: syn::parse::ParseStream<'_>,
    rule_name: &str,
) -> syn::Result<syn::Path> {
    let content;
    syn::parenthesized!(content in input);
    let type_path: syn::Path = content.parse()?;
    if !content.is_empty() {
        return Err(syn::Error::new(
            content.span(),
            format!("validation rule `{rule_name}` accepts exactly one AppEnum type argument"),
        ));
    }
    Ok(type_path)
}

fn parse_validate_args(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<syn::Expr>> {
    let mut args = Vec::new();
    let mut saw_message = false;
    while !input.is_empty() {
        if input.peek(syn::Ident) && input.peek2(syn::Token![=]) {
            let key: syn::Ident = input.parse()?;
            let _: syn::Token![=] = input.parse()?;
            if key == "message" {
                if saw_message {
                    return Err(syn::Error::new(
                        key.span(),
                        "validation rule declares duplicate `message` override",
                    ));
                }
                saw_message = true;
                let value: syn::LitStr = input.parse()?;
                ensure_non_blank_lit_str(value, "validation rule message override")?;
            } else {
                return Err(syn::Error::new(
                    key.span(),
                    format!("unknown keyword argument `{key}`"),
                ));
            }
        } else {
            args.push(input.parse()?);
        }

        if !input.is_empty() {
            let _: syn::Token![,] = input.parse()?;
        }
    }
    Ok(args)
}

fn ensure_non_blank_lit_str(value: syn::LitStr, description: &str) -> syn::Result<()> {
    if value.value().trim().is_empty() {
        return Err(syn::Error::new(
            value.span(),
            format!("{description} must not be blank"),
        ));
    }
    Ok(())
}
