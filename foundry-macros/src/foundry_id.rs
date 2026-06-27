use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr, Path};

use crate::common::to_snake_case;

#[derive(Default)]
struct FoundryIdArgs {
    id: Option<Path>,
    rename_all: Option<LitStr>,
}

#[derive(Default)]
struct VariantArgs {
    value: Option<LitStr>,
}

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "FoundryId does not support generic enums",
        ));
    }

    let ident = input.ident;
    let args = parse_enum_args(&input.attrs)?;
    let id_path = args
        .id
        .ok_or_else(|| syn::Error::new_spanned(&ident, "missing #[foundry(id = ...)] attribute"))?;

    let rename_all = match args.rename_all {
        Some(value) if value.value() == "snake_case" => Some(value),
        Some(value) => {
            return Err(syn::Error::new_spanned(
                value,
                "unsupported rename_all value; expected \"snake_case\"",
            ))
        }
        None => None,
    };

    let Data::Enum(data) = input.data else {
        return Err(syn::Error::new_spanned(
            ident,
            "FoundryId can only be derived for enums",
        ));
    };

    let mut seen_values = HashSet::new();
    let mut match_arms = Vec::new();

    for variant in data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                "FoundryId can only be derived for fieldless enum variants",
            ));
        }

        let variant_args = parse_variant_args(&variant.attrs)?;
        let value = match variant_args.value {
            Some(value) => value,
            None if rename_all.is_some() => {
                LitStr::new(&to_snake_case(&variant.ident.to_string()), variant.ident.span())
            }
            None => {
                return Err(syn::Error::new_spanned(
                    variant.ident,
                    "missing #[foundry(value = \"...\")] variant attribute; add rename_all = \"snake_case\" to the enum to derive values automatically",
                ))
            }
        };

        if !seen_values.insert(value.value()) {
            return Err(syn::Error::new_spanned(value, "duplicate FoundryId value"));
        }

        let variant_ident = variant.ident;
        match_arms.push(quote!(Self::#variant_ident => #value));
    }

    Ok(quote! {
        impl #ident {
            pub const fn as_str(&self) -> &'static str {
                match self {
                    #(#match_arms),*
                }
            }

            pub const fn id(self) -> #id_path {
                #id_path::new(self.as_str())
            }
        }

        impl ::core::convert::From<#ident> for #id_path {
            fn from(value: #ident) -> Self {
                value.id()
            }
        }

        impl ::core::fmt::Display for #ident {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    })
}

fn parse_enum_args(attrs: &[syn::Attribute]) -> syn::Result<FoundryIdArgs> {
    let mut args = FoundryIdArgs::default();

    for attr in attrs.iter().filter(|attr| attr.path().is_ident("foundry")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") {
                set_once_path(&mut args.id, "id", meta.value()?)?;
            } else if meta.path.is_ident("rename_all") {
                set_once_lit_str(&mut args.rename_all, "rename_all", meta.value()?)?;
            } else {
                return Err(meta.error(
                    "unsupported foundry attribute for FoundryId derive; expected id = ... or rename_all = \"snake_case\"",
                ));
            }
            Ok(())
        })?;
    }

    Ok(args)
}

fn parse_variant_args(attrs: &[syn::Attribute]) -> syn::Result<VariantArgs> {
    let mut args = VariantArgs::default();

    for attr in attrs.iter().filter(|attr| attr.path().is_ident("foundry")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("value") {
                set_once_lit_str(&mut args.value, "value", meta.value()?)?;
            } else {
                return Err(meta.error(
                    "unsupported foundry variant attribute for FoundryId derive; expected value = \"...\"",
                ));
            }
            Ok(())
        })?;
    }

    Ok(args)
}

fn set_once_path(
    slot: &mut Option<Path>,
    name: &str,
    value: syn::parse::ParseStream<'_>,
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

fn set_once_lit_str(
    slot: &mut Option<LitStr>,
    name: &str,
    value: syn::parse::ParseStream<'_>,
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
