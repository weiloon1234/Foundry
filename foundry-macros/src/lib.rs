use proc_macro::TokenStream;

mod app_enum;
mod common;
mod foundry_id;
mod model;
mod openapi;
mod projection;
mod typescript;
mod validate;

#[proc_macro_derive(Model, attributes(foundry))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    expand(input, model::expand)
}

#[proc_macro_derive(Projection, attributes(foundry))]
pub fn derive_projection(input: TokenStream) -> TokenStream {
    expand(input, projection::expand)
}

#[proc_macro_derive(AppEnum, attributes(foundry))]
pub fn derive_app_enum(input: TokenStream) -> TokenStream {
    expand_enum_with_ts(input, app_enum::expand)
}

#[proc_macro_derive(FoundryId, attributes(foundry))]
pub fn derive_foundry_id(input: TokenStream) -> TokenStream {
    expand(input, foundry_id::expand)
}

#[proc_macro_derive(Validate, attributes(validate, serde))]
pub fn derive_validate(input: TokenStream) -> TokenStream {
    expand(input, validate::expand)
}

#[proc_macro_derive(ApiSchema, attributes(serde, validate))]
pub fn derive_api_schema(input: TokenStream) -> TokenStream {
    expand(input, openapi::expand)
}

#[proc_macro_derive(TS)]
pub fn derive_ts(input: TokenStream) -> TokenStream {
    expand(input, typescript::expand)
}

fn expand(
    input: TokenStream,
    f: fn(syn::DeriveInput) -> syn::Result<proc_macro2::TokenStream>,
) -> TokenStream {
    match syn::parse(input).and_then(f) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Like `expand`, but also registers enum values for runtime TS export.
fn expand_enum_with_ts(
    input: TokenStream,
    f: fn(syn::DeriveInput) -> syn::Result<proc_macro2::TokenStream>,
) -> TokenStream {
    match syn::parse::<syn::DeriveInput>(input) {
        Ok(parsed) => {
            let app_enum_tokens = typescript::expand_app_enum(&parsed);
            match f(parsed) {
                Ok(main_tokens) => {
                    let combined = quote::quote! {
                        #main_tokens
                        #app_enum_tokens
                    };
                    combined.into()
                }
                Err(error) => error.to_compile_error().into(),
            }
        }
        Err(error) => error.to_compile_error().into(),
    }
}
