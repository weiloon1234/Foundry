use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput};

use crate::common::{
    consume_meta_value, reject_directional_serde_skip, reject_duplicate_contract_field_names,
    reject_duplicate_contract_variant_names, serde_has_default, serde_has_skip_serializing_if,
    should_skip_contract_field, ts_has_optional, validate_has_required_rule,
};

/// Expands `#[derive(foundry::TS)]` to register the type for TypeScript export.
pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    reject_direct_ts_rs_export(&input.attrs)?;
    reject_duplicate_enum_variant_names(&input)?;
    reject_omittable_fields_without_ts_optional(&input)?;

    let ident = &input.ident;
    let name = ident.to_string();

    Ok(quote! {
        ::foundry::inventory::submit! {
            ::foundry::typescript::TsType {
                name: #name,
                export_fn: |dir| <#ident as ::foundry::ts_rs::TS>::export_all_to(dir),
                output_path_fn: || <#ident as ::foundry::ts_rs::TS>::output_path(),
            }
        }
    })
}

fn reject_duplicate_enum_variant_names(input: &DeriveInput) -> syn::Result<()> {
    let Data::Enum(data) = &input.data else {
        return Ok(());
    };

    reject_duplicate_contract_variant_names(data, &input.attrs)
}

fn reject_direct_ts_rs_export(attrs: &[syn::Attribute]) -> syn::Result<()> {
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("ts")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("export") {
                return Err(meta.error(
                    "Foundry owns TypeScript export output through `types:export`; remove `export` and run the Foundry exporter instead",
                ));
            }

            consume_meta_value(meta)?;

            Ok(())
        })?;
    }

    Ok(())
}

fn reject_omittable_fields_without_ts_optional(input: &DeriveInput) -> syn::Result<()> {
    let Data::Struct(data) = &input.data else {
        return Ok(());
    };

    if let syn::Fields::Named(fields) = &data.fields {
        reject_duplicate_contract_field_names(fields, &input.attrs)?;
    }

    let struct_has_default = serde_has_default(&input.attrs)?;
    for field in data.fields.iter() {
        if should_skip_contract_field(field)? {
            continue;
        }

        reject_directional_serde_skip(field)?;

        let ts_optional = ts_has_optional(&field.attrs)?;
        let has_default = struct_has_default || serde_has_default(&field.attrs)?;

        if has_default && !validate_has_required_rule(&field.attrs)? && !ts_optional {
            return Err(syn::Error::new_spanned(
                field,
                "`#[serde(default)]` makes this field optional at runtime; add `#[ts(optional, as = \"Option<_>\")]` so generated TypeScript matches, or add `#[validate(required)]` if omission should fail validation",
            ));
        }

        if serde_has_skip_serializing_if(&field.attrs)? && !ts_optional {
            return Err(syn::Error::new_spanned(
                field,
                "`#[serde(skip_serializing_if)]` can omit this field at runtime; add `#[ts(optional)]` so generated TypeScript matches",
            ));
        }
    }

    Ok(())
}

/// Additional registration for AppEnum types — includes runtime metadata.
pub fn expand_app_enum(input: &DeriveInput) -> TokenStream {
    let ident = &input.ident;
    let name = ident.to_string();

    quote! {
        ::foundry::inventory::submit! {
            ::foundry::typescript::TsAppEnum {
                name: #name,
                meta_fn: || <#ident as ::foundry::FoundryAppEnum>::meta(),
            }
        }
    }
}
