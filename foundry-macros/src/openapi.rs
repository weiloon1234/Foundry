use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Type};

use crate::common::{
    option_inner_type, type_argument_if_last_segment_ident, type_path_last_segment_matches,
    vec_inner_type,
};

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let name_str = name.to_string();

    match &input.data {
        Data::Struct(data) => expand_struct(name, &name_str, &data.fields, &input.attrs),
        Data::Enum(data) => expand_enum(name, &name_str, data),
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
    _attrs: &[syn::Attribute],
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
    let mut required_fields = Vec::new();

    for field in &named.named {
        let field_ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(field, "expected named field"))?;
        let field_name = field_ident.to_string();
        let field_ty = &field.ty;

        // Check if field is Option
        let (is_option, inner_ty) = if let Some(inner) = option_inner_type(field_ty) {
            (true, inner)
        } else {
            (false, field_ty)
        };

        // Generate schema for the inner type
        let schema_expr = type_to_schema_expr(inner_ty);

        // Parse #[validate(...)] attributes for constraints
        let constraints = parse_validate_constraints(&field.attrs)?;
        let is_required = !is_option
            || constraints
                .iter()
                .any(|constraint| matches!(constraint, ValidateConstraint::Required));
        if is_required {
            required_fields.push(field_name.clone());
        }

        let constraint_inserts: Vec<TokenStream> = constraints
            .iter()
            .filter_map(|c| c.to_schema_insert())
            .collect();
        let nullable_wrap = if is_option && !is_required {
            quote!(let field_schema = ::foundry::openapi::nullable_schema(field_schema);)
        } else {
            quote!()
        };

        property_inserts.push(quote! {
            {
                let mut field_schema = #schema_expr;
                if let Some(obj) = field_schema.as_object_mut() {
                    #(#constraint_inserts)*
                }
                #nullable_wrap
                properties.insert(#field_name.to_string(), field_schema);
            }
        });
    }

    let required_tokens = if required_fields.is_empty() {
        quote! {}
    } else {
        quote! {
            schema_obj.insert(
                "required".to_string(),
                ::serde_json::json!([#(#required_fields),*]),
            );
        }
    };

    Ok(quote! {
        impl ::foundry::openapi::ApiSchema for #name {
            fn schema() -> ::serde_json::Value {
                let mut properties = ::serde_json::Map::new();
                #(#property_inserts)*

                let mut schema_obj = ::serde_json::Map::new();
                schema_obj.insert("type".to_string(), ::serde_json::json!("object"));
                schema_obj.insert("properties".to_string(), ::serde_json::Value::Object(properties));
                #required_tokens
                ::serde_json::Value::Object(schema_obj)
            }

            fn schema_name() -> &'static str {
                #name_str
            }
        }

        ::foundry::inventory::submit! {
            ::foundry::openapi::ApiSchemaDefinition {
                name: #name_str,
                schema_fn: <#name as ::foundry::openapi::ApiSchema>::schema,
            }
        }
    })
}

fn expand_enum(
    name: &syn::Ident,
    name_str: &str,
    data: &syn::DataEnum,
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

    let variant_names: Vec<String> = data.variants.iter().map(|v| v.ident.to_string()).collect();

    Ok(quote! {
        impl ::foundry::openapi::ApiSchema for #name {
            fn schema() -> ::serde_json::Value {
                ::serde_json::json!({
                    "type": "string",
                    "enum": [#(#variant_names),*]
                })
            }

            fn schema_name() -> &'static str {
                #name_str
            }
        }

        ::foundry::inventory::submit! {
            ::foundry::openapi::ApiSchemaDefinition {
                name: #name_str,
                schema_fn: <#name as ::foundry::openapi::ApiSchema>::schema,
            }
        }
    })
}

/// Map a Rust type to its JSON Schema expression as a `TokenStream`.
fn type_to_schema_expr(ty: &Type) -> TokenStream {
    // Check Vec<T>
    if let Some(inner) = vec_inner_type(ty) {
        let inner_expr = type_to_schema_expr(inner);
        return quote! {
            ::serde_json::json!({"type": "array", "items": #inner_expr})
        };
    }

    // Check Option<T> (shouldn't happen at top level, handled above)
    if let Some(inner) = option_inner_type(ty) {
        let inner_expr = type_to_schema_expr(inner);
        return quote! {
            ::foundry::openapi::nullable_schema(#inner_expr)
        };
    }

    // Primitive type checks
    if type_path_last_segment_matches(ty, "String") || type_path_last_segment_matches(ty, "str") {
        return quote! { ::serde_json::json!({"type": "string"}) };
    }
    if type_path_last_segment_matches(ty, "i32") {
        return quote! { ::serde_json::json!({"type": "integer", "format": "int32"}) };
    }
    if type_path_last_segment_matches(ty, "i64") {
        return quote! { ::serde_json::json!({"type": "integer", "format": "int64"}) };
    }
    if type_path_last_segment_matches(ty, "i16") {
        return quote! { ::serde_json::json!({"type": "integer", "format": "int32"}) };
    }
    if type_path_last_segment_matches(ty, "u32") || type_path_last_segment_matches(ty, "u64") {
        return quote! { ::serde_json::json!({"type": "integer"}) };
    }
    if type_path_last_segment_matches(ty, "f32") {
        return quote! { ::serde_json::json!({"type": "number", "format": "float"}) };
    }
    if type_path_last_segment_matches(ty, "f64") {
        return quote! { ::serde_json::json!({"type": "number", "format": "double"}) };
    }
    if type_path_last_segment_matches(ty, "bool") {
        return quote! { ::serde_json::json!({"type": "boolean"}) };
    }
    if type_path_last_segment_matches(ty, "DateTime") {
        return quote! { ::serde_json::json!({"type": "string", "format": "date-time"}) };
    }
    if type_path_last_segment_matches(ty, "Date") {
        return quote! { ::serde_json::json!({"type": "string", "format": "date"}) };
    }
    if type_path_last_segment_matches(ty, "Uuid") {
        return quote! { ::serde_json::json!({"type": "string", "format": "uuid"}) };
    }
    if type_argument_if_last_segment_ident(ty, "ModelId").is_some() {
        return quote! { ::serde_json::json!({"type": "string", "format": "uuid"}) };
    }
    if type_path_last_segment_matches(ty, "Value") {
        return quote! { ::serde_json::json!({}) };
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

#[derive(Debug)]
enum ValidateConstraint {
    Required,
    Email,
    Url,
    UuidFormat,
    MinLength(syn::Expr),
    MaxLength(syn::Expr),
    MinNumeric(syn::Expr),
    MaxNumeric(syn::Expr),
}

impl ValidateConstraint {
    fn to_schema_insert(&self) -> Option<TokenStream> {
        match self {
            Self::Required => None, // handled separately via required array
            Self::Email => Some(quote! {
                obj.insert("format".into(), ::serde_json::json!("email"));
            }),
            Self::Url => Some(quote! {
                obj.insert("format".into(), ::serde_json::json!("uri"));
            }),
            Self::UuidFormat => Some(quote! {
                obj.insert("format".into(), ::serde_json::json!("uuid"));
            }),
            Self::MinLength(expr) => Some(quote! {
                obj.insert("minLength".into(), ::serde_json::json!(#expr));
            }),
            Self::MaxLength(expr) => Some(quote! {
                obj.insert("maxLength".into(), ::serde_json::json!(#expr));
            }),
            Self::MinNumeric(expr) => Some(quote! {
                obj.insert("minimum".into(), ::serde_json::json!(#expr));
            }),
            Self::MaxNumeric(expr) => Some(quote! {
                obj.insert("maximum".into(), ::serde_json::json!(#expr));
            }),
        }
    }
}

fn parse_validate_constraints(attrs: &[syn::Attribute]) -> syn::Result<Vec<ValidateConstraint>> {
    let mut constraints = Vec::new();

    for attr in attrs.iter().filter(|a| a.path().is_ident("validate")) {
        attr.parse_args_with(|input: syn::parse::ParseStream<'_>| {
            while !input.is_empty() {
                let ident: syn::Ident = input.parse()?;
                let name = ident.to_string();

                match name.as_str() {
                    "required" => constraints.push(ValidateConstraint::Required),
                    "email" => constraints.push(ValidateConstraint::Email),
                    "url" => constraints.push(ValidateConstraint::Url),
                    "uuid" => constraints.push(ValidateConstraint::UuidFormat),
                    "min" | "min_length" => {
                        let content;
                        syn::parenthesized!(content in input);
                        let expr: syn::Expr = content.parse()?;
                        constraints.push(ValidateConstraint::MinLength(expr));
                    }
                    "max" | "max_length" => {
                        let content;
                        syn::parenthesized!(content in input);
                        let expr: syn::Expr = content.parse()?;
                        constraints.push(ValidateConstraint::MaxLength(expr));
                    }
                    "min_numeric" => {
                        let content;
                        syn::parenthesized!(content in input);
                        let expr: syn::Expr = content.parse()?;
                        constraints.push(ValidateConstraint::MinNumeric(expr));
                    }
                    "max_numeric" => {
                        let content;
                        syn::parenthesized!(content in input);
                        let expr: syn::Expr = content.parse()?;
                        constraints.push(ValidateConstraint::MaxNumeric(expr));
                    }
                    _ => {
                        // Skip unknown rules — they're for the Validate derive, not us.
                        // Consume any parenthesized content if present.
                        if input.peek(syn::token::Paren) {
                            let content;
                            syn::parenthesized!(content in input);
                            let _ = content.parse::<TokenStream>();
                        }
                    }
                }

                if !input.is_empty() {
                    let _: syn::Token![,] = input.parse()?;
                }
            }
            Ok(())
        })?;
    }

    Ok(constraints)
}
