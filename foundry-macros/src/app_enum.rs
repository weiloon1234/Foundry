use proc_macro2::TokenStream;
use quote::quote;
use std::collections::{HashMap, HashSet};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Expr, ExprLit, Fields, Lit, Path, Variant};

// ---------------------------------------------------------------------------
// Compile-time identifier normalization (proc-macro version, independent from runtime)
// ---------------------------------------------------------------------------

fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

fn is_separator(ch: char) -> bool {
    matches!(ch, '_' | '-' | ' ')
}

fn should_split(prev: char, current: char, next: Option<char>) -> bool {
    if is_separator(prev) || is_separator(current) {
        return false;
    }

    (prev.is_lowercase() && current.is_uppercase())
        || (prev.is_alphabetic() && current.is_ascii_digit())
        || (prev.is_ascii_digit() && current.is_alphabetic())
        || (prev.is_uppercase()
            && current.is_uppercase()
            && next.is_some_and(|ch| ch.is_lowercase()))
}

fn split_identifier_words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();

    for (index, ch) in chars.iter().copied().enumerate() {
        if is_separator(ch) {
            push_token(&mut tokens, &mut current);
            continue;
        }

        if let Some(prev) = current.chars().last() {
            let next = chars.get(index + 1).copied();
            if should_split(prev, ch, next) {
                push_token(&mut tokens, &mut current);
            }
        }

        current.push(ch);
    }

    push_token(&mut tokens, &mut current);
    tokens
}

fn to_snake_case(name: &str) -> String {
    split_identifier_words(name)
        .into_iter()
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("_")
}

// ---------------------------------------------------------------------------
// Parsed variant info
// ---------------------------------------------------------------------------

struct VariantInfo {
    ident: syn::Ident,
    key_str: String,
    label_key_str: String,
    aliases: Vec<String>,
    discriminant: Option<i32>,
}

struct EnumArgs {
    id: Option<String>,
    id_type: Option<Path>,
    label_prefix: Option<String>,
}

// ---------------------------------------------------------------------------
// Attribute parsing
// ---------------------------------------------------------------------------

fn parse_enum_args(attrs: &[syn::Attribute]) -> syn::Result<EnumArgs> {
    let mut id = None;
    let mut id_type = None;
    let mut label_prefix = None;

    for attr in attrs.iter().filter(|a| a.path().is_ident("foundry")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") {
                let value: syn::LitStr = meta.value()?.parse()?;
                if id.is_some() {
                    return Err(syn::Error::new(value.span(), "duplicate `id` attribute"));
                }
                id = Some(value.value());
            } else if meta.path.is_ident("id_type") {
                if id_type.is_some() {
                    return Err(syn::Error::new(
                        meta.path.span(),
                        "duplicate `id_type` attribute",
                    ));
                }
                id_type = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("label_prefix") {
                let value: syn::LitStr = meta.value()?.parse()?;
                if label_prefix.is_some() {
                    return Err(syn::Error::new(
                        value.span(),
                        "duplicate `label_prefix` attribute",
                    ));
                }
                label_prefix = Some(value.value());
            } else {
                return Err(meta.error(
                    "unsupported foundry enum attribute; expected id = \"...\", id_type = Type, or label_prefix = \"...\"",
                ));
            }
            Ok(())
        })?;
    }

    Ok(EnumArgs {
        id,
        id_type,
        label_prefix,
    })
}

fn parse_variant_attrs(
    variant: &Variant,
) -> syn::Result<(Option<String>, Option<String>, Vec<String>)> {
    let mut key = None;
    let mut label_key = None;
    let mut aliases = Vec::new();

    for attr in variant
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("foundry"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("key") {
                let value: syn::LitStr = meta.value()?.parse()?;
                if key.is_some() {
                    return Err(syn::Error::new(value.span(), "duplicate `key` attribute"));
                }
                key = Some(value.value());
            } else if meta.path.is_ident("label_key") {
                let value: syn::LitStr = meta.value()?.parse()?;
                if label_key.is_some() {
                    return Err(syn::Error::new(
                        value.span(),
                        "duplicate `label_key` attribute",
                    ));
                }
                label_key = Some(value.value());
            } else if meta.path.is_ident("aliases") {
                let lit: syn::Expr = meta.value()?.parse()?;
                match &lit {
                    Expr::Array(arr) => {
                        for elem in &arr.elems {
                            if let Expr::Lit(ExprLit {
                                lit: Lit::Str(s), ..
                            }) = elem
                            {
                                aliases.push(s.value());
                            } else {
                                return Err(syn::Error::new(
                                    elem.span(),
                                    "alias must be a string literal",
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(syn::Error::new(
                            lit.span(),
                            "aliases must be an array of string literals",
                        ))
                    }
                }
            } else {
                return Err(meta.error(
                    "unsupported foundry variant attribute; expected key, label_key, or aliases",
                ));
            }
            Ok(())
        })?;
    }

    Ok((key, label_key, aliases))
}

// ---------------------------------------------------------------------------
// Discriminant extraction
// ---------------------------------------------------------------------------

fn extract_discriminant(variant: &Variant) -> syn::Result<Option<i32>> {
    let Some((_, expr)) = &variant.discriminant else {
        return Ok(None);
    };

    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(lit_int),
            ..
        }) => {
            let val: i64 = lit_int.base10_parse::<i64>().map_err(|_| {
                syn::Error::new(lit_int.span(), "discriminant must be a literal integer")
            })?;
            if val < i32::MIN as i64 || val > i32::MAX as i64 {
                return Err(syn::Error::new(
                    lit_int.span(),
                    "discriminant exceeds i32 range",
                ));
            }
            Ok(Some(val as i32))
        }
        _ => Err(syn::Error::new(
            expr.span(),
            "discriminant must be a literal integer",
        )),
    }
}

// ---------------------------------------------------------------------------
// Main expand function
// ---------------------------------------------------------------------------

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = input.ident.clone();

    // Must be an enum
    let data = match &input.data {
        Data::Enum(data) => data,
        _ => {
            return Err(syn::Error::new_spanned(
                &ident,
                "AppEnum can only be derived on enums",
            ))
        }
    };

    // Parse enum-level attributes
    let enum_args = parse_enum_args(&input.attrs)?;
    let enum_id = enum_args
        .id
        .unwrap_or_else(|| to_snake_case(&ident.to_string()));
    let label_prefix = enum_args
        .label_prefix
        .unwrap_or_else(|| format!("enum.{enum_id}"));

    // Classify storage mode and validate
    let has_any_discriminant = data.variants.iter().any(|v| v.discriminant.is_some());
    let all_have_discriminant = data.variants.iter().all(|v| v.discriminant.is_some());

    if has_any_discriminant && !all_have_discriminant {
        return Err(syn::Error::new_spanned(
            &ident,
            "enum has mixed storage modes: either all variants must have integer discriminants, or none",
        ));
    }

    let is_int_backed = all_have_discriminant;
    if is_int_backed && enum_args.id_type.is_some() {
        return Err(syn::Error::new_spanned(
            &ident,
            "id_type is only supported for string-backed AppEnum enums",
        ));
    }

    // Parse each variant
    let mut variant_infos = Vec::new();
    let mut seen_keys: HashSet<String> = HashSet::new();

    for variant in &data.variants {
        // Only unit variants
        if !matches!(&variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                "AppEnum only supports unit variants",
            ));
        }

        let (key_override, label_key_override, aliases) = parse_variant_attrs(variant)?;

        // Int-backed enums cannot have key overrides
        if is_int_backed && key_override.is_some() {
            return Err(syn::Error::new_spanned(
                variant,
                "key override not allowed on int-backed enums",
            ));
        }

        let discriminant =
            if is_int_backed {
                Some(extract_discriminant(variant)?.ok_or_else(|| {
                    syn::Error::new_spanned(variant, "expected integer discriminant")
                })?)
            } else {
                None
            };

        let variant_snake = to_snake_case(&variant.ident.to_string());

        let key_str = if is_int_backed {
            discriminant.unwrap().to_string()
        } else {
            key_override.unwrap_or(variant_snake.clone())
        };

        if key_str.is_empty() {
            return Err(syn::Error::new_spanned(
                variant,
                "AppEnum keys cannot be empty",
            ));
        }

        // Duplicate key check
        if !seen_keys.insert(key_str.clone()) {
            return Err(syn::Error::new_spanned(
                variant,
                format!("duplicate key '{}'", key_str),
            ));
        }

        let label_key_str =
            label_key_override.unwrap_or_else(|| format!("{label_prefix}.{variant_snake}"));

        variant_infos.push(VariantInfo {
            ident: variant.ident.clone(),
            key_str,
            label_key_str,
            aliases,
            discriminant,
        });
    }

    // Keys and aliases share one parsing namespace. Validate after collecting
    // every key so an alias cannot silently shadow a later variant's key.
    let mut parse_values = variant_infos
        .iter()
        .map(|variant| (variant.key_str.clone(), variant.ident.to_string()))
        .collect::<HashMap<_, _>>();
    for (variant, info) in data.variants.iter().zip(&variant_infos) {
        for alias in &info.aliases {
            if alias.is_empty() {
                return Err(syn::Error::new_spanned(
                    variant,
                    "AppEnum aliases cannot be empty",
                ));
            }
            if let Some(existing) = parse_values.insert(alias.clone(), info.ident.to_string()) {
                return Err(syn::Error::new_spanned(
                    variant,
                    format!(
                        "AppEnum alias '{alias}' conflicts with the key or alias for variant `{existing}`"
                    ),
                ));
            }
        }
    }

    // Generate all impl blocks
    let foundry_impl =
        generate_foundry_app_enum_impl(&ident, &enum_id, &variant_infos, is_int_backed)?;
    let to_db = generate_to_db_value_impl(&ident, &variant_infos, is_int_backed)?;
    let from_db = generate_from_db_value_impl(&ident, &variant_infos, is_int_backed)?;
    let serialize = generate_serialize_impl(&ident, &variant_infos, is_int_backed)?;
    let deserialize = generate_deserialize_impl(&ident, &variant_infos, is_int_backed)?;
    let api_schema = generate_api_schema_impl(&ident, &variant_infos, is_int_backed);
    let ts = generate_ts_impl(&ident, &variant_infos, is_int_backed);
    let typed_id = generate_typed_id_impl(&ident, &variant_infos, enum_args.id_type.as_ref());

    Ok(quote! {
        #foundry_impl
        #to_db
        #from_db
        #serialize
        #deserialize
        #api_schema
        #ts
        #typed_id
    })
}

fn ts_string_literal(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn generate_ts_impl(
    ident: &syn::Ident,
    variants: &[VariantInfo],
    is_int_backed: bool,
) -> TokenStream {
    let name = ident.to_string();
    let output_path = format!("{name}.ts");
    let union = if is_int_backed {
        variants
            .iter()
            .map(|variant| {
                variant
                    .discriminant
                    .expect("int-backed variant missing discriminant")
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(" | ")
    } else {
        variants
            .iter()
            .map(|variant| ts_string_literal(&variant.key_str))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let decl = format!("type {name} = {union};");

    quote! {
        impl ::foundry::ts_rs::TS for #ident {
            type WithoutGenerics = Self;

            fn name() -> String {
                #name.to_string()
            }

            fn inline() -> String {
                <Self as ::foundry::ts_rs::TS>::name()
            }

            fn inline_flattened() -> String {
                <Self as ::foundry::ts_rs::TS>::inline()
            }

            fn decl() -> String {
                #decl.to_string()
            }

            fn decl_concrete() -> String {
                <Self as ::foundry::ts_rs::TS>::decl()
            }

            fn output_path() -> ::core::option::Option<&'static ::std::path::Path> {
                ::core::option::Option::Some(::std::path::Path::new(#output_path))
            }
        }
    }
}

fn generate_typed_id_impl(
    ident: &syn::Ident,
    variants: &[VariantInfo],
    id_type: Option<&Path>,
) -> TokenStream {
    let Some(id_type) = id_type else {
        return TokenStream::new();
    };

    let as_str_arms = variants.iter().map(|v| {
        let v_ident = &v.ident;
        let key = &v.key_str;
        quote!(Self::#v_ident => #key)
    });

    quote! {
        impl #ident {
            pub const fn as_str(&self) -> &'static str {
                match self {
                    #(#as_str_arms),*
                }
            }

            pub const fn typed_id(self) -> #id_type {
                #id_type::new(self.as_str())
            }
        }

        impl ::core::convert::From<#ident> for #id_type {
            fn from(value: #ident) -> Self {
                value.typed_id()
            }
        }

        impl ::core::convert::AsRef<str> for #ident {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl ::core::fmt::Display for #ident {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FoundryAppEnum impl
// ---------------------------------------------------------------------------

fn generate_foundry_app_enum_impl(
    ident: &syn::Ident,
    enum_id: &str,
    variants: &[VariantInfo],
    is_int_backed: bool,
) -> syn::Result<TokenStream> {
    // DB_TYPE
    let db_type = if is_int_backed {
        quote!(::foundry::database::DbType::Int32)
    } else {
        quote!(::foundry::database::DbType::Text)
    };

    // key() match arms
    let key_arms = variants.iter().map(|v| {
        let v_ident = &v.ident;
        if is_int_backed {
            let n = v.discriminant.unwrap();
            quote!(Self::#v_ident => ::foundry::app_enum::EnumKey::Int(#n))
        } else {
            let k = &v.key_str;
            quote!(Self::#v_ident => ::foundry::app_enum::EnumKey::String(#k.to_string()))
        }
    });

    // keys() collection
    let key_entries = variants.iter().map(|v| {
        if is_int_backed {
            let n = v.discriminant.unwrap();
            quote!(::foundry::app_enum::EnumKey::Int(#n))
        } else {
            let k = &v.key_str;
            quote!(::foundry::app_enum::EnumKey::String(#k.to_string()))
        }
    });

    // parse_key() match arms
    let parse_arms = generate_parse_arms(variants, is_int_backed);

    // label_key() match arms
    let label_arms = variants.iter().map(|v| {
        let v_ident = &v.ident;
        let l = &v.label_key_str;
        quote!(Self::#v_ident => #l)
    });

    // options() entries
    let option_entries = variants.iter().map(|v| {
        if is_int_backed {
            let n = v.discriminant.unwrap();
            let l = &v.label_key_str;
            quote!(::foundry::app_enum::EnumOption {
                value: ::foundry::app_enum::EnumKey::Int(#n),
                label_key: #l.to_string(),
            })
        } else {
            let k = &v.key_str;
            let l = &v.label_key_str;
            quote!(::foundry::app_enum::EnumOption {
                value: ::foundry::app_enum::EnumKey::String(#k.to_string()),
                label_key: #l.to_string(),
            })
        }
    });

    // key_kind
    let key_kind = if is_int_backed {
        quote!(::foundry::app_enum::EnumKeyKind::Int)
    } else {
        quote!(::foundry::app_enum::EnumKeyKind::String)
    };

    Ok(quote! {
        impl ::foundry::app_enum::FoundryAppEnum for #ident {
            const DB_TYPE: ::foundry::database::DbType = #db_type;

            fn id() -> &'static str { #enum_id }

            fn key(self) -> ::foundry::app_enum::EnumKey {
                match self {
                    #(#key_arms),*
                }
            }

            fn keys() -> ::foundry::support::Collection<::foundry::app_enum::EnumKey> {
                ::foundry::support::Collection::from(vec![
                    #(#key_entries),*
                ])
            }

            fn parse_key(key: &str) -> ::core::option::Option<Self> {
                #parse_arms
            }

            fn label_key(self) -> &'static str {
                match self {
                    #(#label_arms),*
                }
            }

            fn options() -> ::foundry::support::Collection<::foundry::app_enum::EnumOption> {
                ::foundry::support::Collection::from(vec![
                    #(#option_entries),*
                ])
            }

            fn meta() -> ::foundry::app_enum::EnumMeta {
                ::foundry::app_enum::EnumMeta {
                    id: Self::id().to_string(),
                    key_kind: Self::key_kind(),
                    options: Self::options(),
                }
            }

            fn key_kind() -> ::foundry::app_enum::EnumKeyKind {
                #key_kind
            }
        }

        impl ::std::str::FromStr for #ident {
            type Err = ::foundry::foundation::Error;

            fn from_str(value: &str) -> ::std::result::Result<Self, Self::Err> {
                <Self as ::foundry::app_enum::FoundryAppEnum>::parse_key(value)
                    .ok_or_else(|| {
                        ::foundry::foundation::Error::message(format!(
                            "invalid {} value `{}`",
                            <Self as ::foundry::app_enum::FoundryAppEnum>::id(),
                            value
                        ))
                    })
            }
        }
    })
}

fn generate_parse_arms(variants: &[VariantInfo], is_int_backed: bool) -> TokenStream {
    let mut primary_arms = Vec::new();
    let mut alias_arms = Vec::new();

    for v in variants {
        let v_ident = &v.ident;
        let key_str = &v.key_str;

        if is_int_backed {
            let n = v.discriminant.unwrap();
            primary_arms.push(quote! {
                #key_str => Some(Self::#v_ident)
            });
            // Also parse by the numeric string directly
            let num_str = n.to_string();
            if num_str != *key_str {
                primary_arms.push(quote! {
                    #num_str => Some(Self::#v_ident)
                });
            }
        } else {
            primary_arms.push(quote! {
                #key_str => Some(Self::#v_ident)
            });
        }

        for alias in &v.aliases {
            alias_arms.push(quote! {
                #alias => Some(Self::#v_ident)
            });
        }
    }

    quote! {
        match key {
            #(#primary_arms,)*
            #(#alias_arms,)*
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// ToDbValue impl
// ---------------------------------------------------------------------------

fn generate_to_db_value_impl(
    ident: &syn::Ident,
    variants: &[VariantInfo],
    is_int_backed: bool,
) -> syn::Result<TokenStream> {
    let arms = variants.iter().map(|v| {
        let v_ident = &v.ident;
        if is_int_backed {
            let n = v.discriminant.unwrap();
            quote!(Self::#v_ident => ::foundry::database::DbValue::Int32(#n))
        } else {
            let k = &v.key_str;
            quote!(Self::#v_ident => ::foundry::database::DbValue::Text(#k.to_string()))
        }
    });
    let db_type = if is_int_backed {
        quote!(::foundry::database::DbType::Int32)
    } else {
        quote!(::foundry::database::DbType::Text)
    };

    Ok(quote! {
        impl ::foundry::database::ToDbValue for #ident {
            fn to_db_value(self) -> ::foundry::database::DbValue {
                match self {
                    #(#arms),*
                }
            }

            fn db_type() -> ::foundry::database::DbType {
                #db_type
            }
        }
    })
}

// ---------------------------------------------------------------------------
// FromDbValue impl
// ---------------------------------------------------------------------------

fn generate_from_db_value_impl(
    ident: &syn::Ident,
    variants: &[VariantInfo],
    is_int_backed: bool,
) -> syn::Result<TokenStream> {
    let type_name = ident.to_string();

    if is_int_backed {
        let arms = variants.iter().map(|v| {
            let v_ident = &v.ident;
            let n = v.discriminant.unwrap();
            quote!(#n => ::core::result::Result::Ok(Self::#v_ident))
        });

        Ok(quote! {
            impl ::foundry::database::FromDbValue for #ident {
                fn from_db_value(value: &::foundry::database::DbValue) -> ::foundry::foundation::Result<Self> {
                    match value {
                        ::foundry::database::DbValue::Int32(n) => {
                            match *n {
                                #(#arms,)*
                                other => ::core::result::Result::Err(::foundry::foundation::Error::message(
                                    format!("invalid {} key: {}", #type_name, other)
                                )),
                            }
                        }
                        ::foundry::database::DbValue::Text(s) => {
                            <Self as ::foundry::app_enum::FoundryAppEnum>::parse_key(s)
                                .ok_or_else(|| ::foundry::foundation::Error::message(
                                    format!("invalid {} key: {}", #type_name, s)
                                ))
                        }
                        other => ::core::result::Result::Err(::foundry::foundation::Error::message(
                            format!("expected int32 for {}, got {:?}", #type_name, other)
                        )),
                    }
                }
            }
        })
    } else {
        Ok(quote! {
            impl ::foundry::database::FromDbValue for #ident {
                fn from_db_value(value: &::foundry::database::DbValue) -> ::foundry::foundation::Result<Self> {
                    match value {
                        ::foundry::database::DbValue::Text(s) => {
                            <Self as ::foundry::app_enum::FoundryAppEnum>::parse_key(s)
                                .ok_or_else(|| ::foundry::foundation::Error::message(
                                    format!("invalid {} key: {}", #type_name, s)
                                ))
                        }
                        other => ::core::result::Result::Err(::foundry::foundation::Error::message(
                            format!("expected text for {}, got {:?}", #type_name, other)
                        )),
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Serialize impl
// ---------------------------------------------------------------------------

fn generate_serialize_impl(
    ident: &syn::Ident,
    variants: &[VariantInfo],
    is_int_backed: bool,
) -> syn::Result<TokenStream> {
    if is_int_backed {
        let arms = variants.iter().map(|v| {
            let v_ident = &v.ident;
            let n = v.discriminant.unwrap();
            quote!(Self::#v_ident => #n.serialize(serializer))
        });

        Ok(quote! {
            impl serde::Serialize for #ident {
                fn serialize<S: serde::Serializer>(&self, serializer: S) -> ::core::result::Result<S::Ok, S::Error> {
                    match self {
                        #(#arms),*
                    }
                }
            }
        })
    } else {
        let arms = variants.iter().map(|v| {
            let v_ident = &v.ident;
            let k = &v.key_str;
            quote!(Self::#v_ident => #k.serialize(serializer))
        });

        Ok(quote! {
            impl serde::Serialize for #ident {
                fn serialize<S: serde::Serializer>(&self, serializer: S) -> ::core::result::Result<S::Ok, S::Error> {
                    match self {
                        #(#arms),*
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Deserialize impl
// ---------------------------------------------------------------------------

fn generate_deserialize_impl(
    ident: &syn::Ident,
    _variants: &[VariantInfo],
    is_int_backed: bool,
) -> syn::Result<TokenStream> {
    let type_name = ident.to_string();

    if is_int_backed {
        Ok(quote! {
            impl<'de> serde::Deserialize<'de> for #ident {
                fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> ::core::result::Result<Self, D::Error> {
                    let n = i32::deserialize(deserializer)?;
                    <Self as ::foundry::app_enum::FoundryAppEnum>::parse_key(&n.to_string())
                        .ok_or_else(|| serde::de::Error::custom(
                            format!("unknown {} variant: {}", #type_name, n)
                        ))
                }
            }
        })
    } else {
        Ok(quote! {
            impl<'de> serde::Deserialize<'de> for #ident {
                fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> ::core::result::Result<Self, D::Error> {
                    let s = String::deserialize(deserializer)?;
                    <Self as ::foundry::app_enum::FoundryAppEnum>::parse_key(&s)
                        .ok_or_else(|| serde::de::Error::custom(
                            format!("unknown {} variant: {}", #type_name, s)
                        ))
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// ApiSchema impl — auto-generated from AppEnum keys
// ---------------------------------------------------------------------------

fn generate_api_schema_impl(
    ident: &syn::Ident,
    variants: &[VariantInfo],
    is_int_backed: bool,
) -> TokenStream {
    let name_str = ident.to_string();

    let (type_str, enum_values) = if is_int_backed {
        let values: Vec<i32> = variants
            .iter()
            .map(|v| {
                v.discriminant
                    .expect("int-backed variant missing discriminant")
            })
            .collect();
        ("integer", quote! { #(#values),* })
    } else {
        let keys: Vec<&str> = variants.iter().map(|v| v.key_str.as_str()).collect();
        ("string", quote! { #(#keys),* })
    };

    quote! {
        impl ::foundry::openapi::ApiSchema for #ident {
            fn schema() -> ::serde_json::Value {
                ::serde_json::json!({
                    "type": #type_str,
                    "enum": [#enum_values]
                })
            }

            fn schema_name() -> &'static str {
                #name_str
            }
        }

        ::foundry::inventory::submit! {
            ::foundry::openapi::ApiSchemaDefinition {
                name: #name_str,
                schema_fn: <#ident as ::foundry::openapi::ApiSchema>::schema,
            }
        }
    }
}
