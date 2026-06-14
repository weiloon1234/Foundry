use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{DeriveInput, LitStr};

use crate::common::{
    ensure_named_struct, field_name_literal, helper_ident, infer_or_explicit_db_type,
    loaded_inner_type, option_inner_type, parse_field_args, parse_model_args, require_ident,
    screaming_const_ident, static_ident, type_argument_if_last_segment_ident,
    type_path_last_segment_matches,
};

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = input.ident.clone();
    let fields = ensure_named_struct(&input)?;
    let args = parse_model_args(&input.attrs)?;

    let table = args.table.ok_or_else(|| {
        syn::Error::new_spanned(&ident, "missing #[foundry(table = ...)] attribute")
    })?;
    let explicit_primary_key = args.primary_key.is_some();
    let primary_key = args
        .primary_key
        .unwrap_or_else(|| LitStr::new("id", Span::call_site()));
    let primary_key_strategy = args
        .primary_key_strategy
        .unwrap_or_else(|| LitStr::new("uuid_v7", Span::call_site()));
    let primary_key_strategy_tokens = match primary_key_strategy.value().as_str() {
        "uuid_v7" => quote!(::foundry::ModelPrimaryKeyStrategy::UuidV7),
        "manual" => quote!(::foundry::ModelPrimaryKeyStrategy::Manual),
        _ => {
            return Err(syn::Error::new_spanned(
                &primary_key_strategy,
                "primary_key_strategy must be either \"uuid_v7\" or \"manual\"",
            ))
        }
    };
    let lifecycle = args
        .lifecycle
        .unwrap_or_else(|| syn::parse_quote!(::foundry::NoModelLifecycle));
    let audit_enabled = match args.audit.as_ref().map(|value| value.value()) {
        Some(true) | None => quote!(true),
        Some(false) => quote!(false),
    };
    let timestamps_setting = match args.timestamps.as_ref().map(|value| value.value()) {
        Some(true) => quote!(::foundry::ModelFeatureSetting::Enabled),
        Some(false) => quote!(::foundry::ModelFeatureSetting::Disabled),
        None => quote!(::foundry::ModelFeatureSetting::Default),
    };
    let soft_deletes_setting = match args.soft_deletes.as_ref().map(|value| value.value()) {
        Some(true) => quote!(::foundry::ModelFeatureSetting::Enabled),
        Some(false) => quote!(::foundry::ModelFeatureSetting::Disabled),
        None => quote!(::foundry::ModelFeatureSetting::Default),
    };

    let mut column_names = BTreeSet::new();
    let mut persisted_column_names = Vec::new();
    let mut const_defs = Vec::new();
    let mut column_info_entries = Vec::new();
    let mut clone_fields = Vec::new();
    let mut hydrate_fields = Vec::new();
    let mut accessor_methods = Vec::new();
    let mut write_mutator_helpers = Vec::new();
    let mut audit_excluded_fields = Vec::new();
    let mut primary_key_field_ident = None;
    let mut primary_key_const_ident = None;
    let mut primary_key_is_model_id_for_self = false;
    let mut has_created_at = false;
    let mut has_updated_at = false;
    let mut has_deleted_at = false;

    for field in &fields.named {
        let field_ident = require_ident(field)?;
        let field_ty = &field.ty;
        let field_args = parse_field_args(field)?;
        clone_fields.push(quote!(#field_ident: ::core::clone::Clone::clone(&self.#field_ident)));

        if loaded_inner_type(field_ty).is_some() {
            if field_args.column.is_some()
                || field_args.alias.is_some()
                || field_args.source.is_some()
                || field_args.db_type.is_some()
                || field_args.write_mutator.is_some()
                || field_args.read_accessor.is_some()
                || field_args.audit_exclude
            {
                return Err(syn::Error::new_spanned(
                    field,
                    "Loaded<T> fields cannot declare foundry field attributes",
                ));
            }
            hydrate_fields.push(quote!(#field_ident: ::foundry::Loaded::Unloaded));
            continue;
        }

        if field_args.alias.is_some() || field_args.source.is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "Model derive does not support #[foundry(alias = ...)] or #[foundry(source = ...)]",
            ));
        }

        let column_name = field_name_literal(field_ident, &field_args.column);
        if !column_names.insert(column_name.value()) {
            return Err(syn::Error::new_spanned(
                &column_name,
                format!("duplicate column name `{}`", column_name.value()),
            ));
        }
        persisted_column_names.push(column_name.value());
        if field_args.audit_exclude {
            audit_excluded_fields.push(column_name.clone());
        }

        let (db_type, db_type_tokens) =
            match infer_or_explicit_db_type(field_ty, field_args.db_type) {
                Ok(spec) => (Some(spec), spec.tokens()),
                Err(_) => {
                    // Fallback: assume FoundryAppEnum, let compiler verify
                    let ty = field_ty;
                    (
                        None,
                        quote!(<#ty as ::foundry::app_enum::FoundryAppEnum>::DB_TYPE),
                    )
                }
            };
        let const_ident = screaming_const_ident(field_ident);
        let is_optional = option_inner_type(field_ty).is_some();
        let field_name = column_name.value();
        let mut column_info_entry = quote!(#ident::#const_ident.info());

        if let Some(write_mutator) = field_args.write_mutator.as_ref() {
            let write_mutator_ident = syn::parse_str::<syn::Ident>(&write_mutator.value())
                .map_err(|_| {
                    syn::Error::new_spanned(
                        write_mutator,
                        "write_mutator must be a valid method name string",
                    )
                })?;
            let helper_ident = helper_ident(&format!("write_mutator_{}", field_ident), &ident);
            write_mutator_helpers.push(quote! {
                fn #helper_ident<'a>(
                    context: &'a ::foundry::ModelHookContext<'a>,
                    value: ::foundry::DbValue,
                ) -> ::core::pin::Pin<
                    ::std::boxed::Box<
                        dyn ::core::future::Future<Output = ::foundry::Result<::foundry::DbValue>> + Send + 'a
                    >
                > {
                    ::std::boxed::Box::pin(async move {
                        let typed_value: #field_ty =
                            <#field_ty as ::foundry::FromDbValue>::from_db_value(&value)?;
                        let transformed: #field_ty =
                            #ident::#write_mutator_ident(context, typed_value).await?;
                        Ok(<#field_ty as ::foundry::IntoFieldValue<#field_ty>>::into_field_value(
                            transformed,
                            #db_type_tokens,
                        ))
                    })
                }
            });
            column_info_entry = quote!(#column_info_entry.with_write_mutator(#helper_ident));
        }

        if let Some(read_accessor) = field_args.read_accessor.as_ref() {
            let read_accessor_ident = syn::parse_str::<syn::Ident>(&read_accessor.value())
                .map_err(|_| {
                    syn::Error::new_spanned(
                        read_accessor,
                        "read_accessor must be a valid method name string",
                    )
                })?;
            let accessed_ident = format_ident!("{}_accessed", field_ident);
            accessor_methods.push(quote! {
                pub fn #accessed_ident(&self) -> #field_ty {
                    #ident::#read_accessor_ident(self)
                }
            });
        }

        if field_name == "created_at" {
            has_created_at = true;
            if db_type != Some(crate::common::DbTypeSpec::TimestampTz) {
                return Err(syn::Error::new_spanned(
                    field,
                    "`created_at` must use foundry::DateTime",
                ));
            }
        }

        if field_name == "updated_at" {
            has_updated_at = true;
            if db_type != Some(crate::common::DbTypeSpec::TimestampTz) {
                return Err(syn::Error::new_spanned(
                    field,
                    "`updated_at` must use foundry::DateTime",
                ));
            }
        }

        if field_name == "deleted_at" {
            has_deleted_at = true;
            if !is_optional || db_type != Some(crate::common::DbTypeSpec::TimestampTz) {
                return Err(syn::Error::new_spanned(
                    field,
                    "`deleted_at` must use Option<foundry::DateTime>",
                ));
            }
        }

        let is_primary_key = column_name.value() == primary_key.value();
        if is_primary_key {
            primary_key_field_ident = Some(field_ident.clone());
            primary_key_const_ident = Some(const_ident.clone());
            let primary_key_is_model_id =
                type_argument_if_last_segment_ident(field_ty, "ModelId").is_some();
            primary_key_is_model_id_for_self = primary_key_targets_self(field_ty, &ident);
            if primary_key_is_model_id && !primary_key_is_model_id_for_self {
                return Err(syn::Error::new_spanned(
                    field,
                    format!(
                        "primary key `{}` must use ModelId<{}> or ModelId<Self>",
                        primary_key.value(),
                        ident
                    ),
                ));
            }
        }

        if is_primary_key && is_optional {
            return Err(syn::Error::new_spanned(
                field,
                "primary key fields cannot use Option<T> on Model derives",
            ));
        }

        const_defs.push(quote! {
            pub const #const_ident: ::foundry::Column<Self, #field_ty> =
                ::foundry::Column::new(#table, #column_name, #db_type_tokens);
        });
        column_info_entries.push(column_info_entry);
        hydrate_fields.push(quote!(#field_ident: record.decode_column(#ident::#const_ident)?));
    }

    if args.timestamps.as_ref().map(|value| value.value()) == Some(true)
        && (!has_created_at || !has_updated_at)
    {
        return Err(syn::Error::new_spanned(
            &ident,
            "#[foundry(timestamps = true)] requires `created_at: foundry::DateTime` and `updated_at: foundry::DateTime`",
        ));
    }

    if args.soft_deletes.as_ref().map(|value| value.value()) == Some(true) && !has_deleted_at {
        return Err(syn::Error::new_spanned(
            &ident,
            "#[foundry(soft_deletes = true)] requires `deleted_at: Option<foundry::DateTime>`",
        ));
    }

    if !persisted_column_names
        .iter()
        .any(|name| name == &primary_key.value())
    {
        if explicit_primary_key {
            return Err(syn::Error::new_spanned(
                &primary_key,
                format!(
                    "primary_key `{}` does not match any persisted field",
                    primary_key.value()
                ),
            ));
        }

        return Err(syn::Error::new_spanned(
            &ident,
            if primary_key_strategy.value() == "manual" {
                "manual primary_key_strategy requires an `id` field or an explicit #[foundry(primary_key = \"...\")] attribute"
            } else {
                "Model derive requires an `id` field or an explicit #[foundry(primary_key = \"...\")] attribute"
            },
        ));
    }

    let columns_static = static_ident("COLUMNS", &ident);
    let hydrate_fn = helper_ident("hydrate", &ident);
    let column_count = column_info_entries.len();
    let primary_key_field_ident = primary_key_field_ident.ok_or_else(|| {
        syn::Error::new_spanned(
            &ident,
            "Model derive requires a resolvable primary key field",
        )
    })?;
    let primary_key_const_ident = primary_key_const_ident.ok_or_else(|| {
        syn::Error::new_spanned(
            &ident,
            "Model derive requires a resolvable primary key column constant",
        )
    })?;

    if primary_key_strategy.value() == "uuid_v7" && !primary_key_is_model_id_for_self {
        let message = if explicit_primary_key {
            format!(
                "default primary_key_strategy = \"uuid_v7\" requires `{}` to use ModelId<{}> or ModelId<Self>; use #[foundry(primary_key_strategy = \"manual\")] to opt out",
                primary_key.value(),
                ident
            )
        } else {
            format!(
                "Model derive defaults to `id: ModelId<{}>`; add that field or use #[foundry(primary_key_strategy = \"manual\")] to opt out",
                ident
            )
        };
        return Err(syn::Error::new_spanned(&ident, message));
    }

    Ok(quote! {
        impl ::core::clone::Clone for #ident {
            fn clone(&self) -> Self {
                Self {
                    #(#clone_fields),*
                }
            }
        }

        impl #ident {
            #(#const_defs)*
            #(#accessor_methods)*

            pub fn query() -> ::foundry::ModelQuery<Self> {
                <Self as ::foundry::Model>::model_query()
            }

            pub fn create() -> ::foundry::CreateModel<Self> {
                <Self as ::foundry::Model>::model_create()
            }

            pub fn create_many() -> ::foundry::CreateManyModel<Self> {
                <Self as ::foundry::Model>::model_create_many()
            }

            pub fn update() -> ::foundry::UpdateModel<Self> {
                <Self as ::foundry::Model>::model_update()
            }

            pub fn delete() -> ::foundry::DeleteModel<Self> {
                <Self as ::foundry::Model>::model_delete()
            }

            pub fn force_delete() -> ::foundry::DeleteModel<Self> {
                <Self as ::foundry::Model>::model_force_delete()
            }

            pub fn restore() -> ::foundry::RestoreModel<Self> {
                <Self as ::foundry::Model>::model_restore()
            }
        }

        static #columns_static: [::foundry::ColumnInfo; #column_count] = [#(#column_info_entries),*];

        #(#write_mutator_helpers)*

        fn #hydrate_fn(record: &::foundry::DbRecord) -> ::foundry::Result<#ident> {
            Ok(#ident {
                #(#hydrate_fields),*
            })
        }

        impl ::foundry::Model for #ident {
            type Lifecycle = #lifecycle;

            fn table_meta() -> &'static ::foundry::TableMeta<Self> {
                static TABLE: ::std::sync::OnceLock<::foundry::TableMeta<#ident>> =
                    ::std::sync::OnceLock::new();
                TABLE.get_or_init(|| {
                    ::foundry::TableMeta::new(
                        #table,
                        &#columns_static,
                        #primary_key,
                        #primary_key_strategy_tokens,
                        ::foundry::ModelBehavior::new(#timestamps_setting, #soft_deletes_setting),
                        #hydrate_fn,
                    )
                })
            }

            fn audit_enabled() -> bool {
                #audit_enabled
            }

            fn audit_excluded_fields() -> &'static [&'static str] {
                &[#(#audit_excluded_fields),*]
            }
        }

        impl ::foundry::PersistedModel for #ident {
            fn persisted_condition(&self) -> ::foundry::Condition {
                #ident::#primary_key_const_ident
                    .eq(::core::clone::Clone::clone(&self.#primary_key_field_ident))
            }
        }
    })
}

fn primary_key_targets_self(ty: &syn::Type, ident: &syn::Ident) -> bool {
    let Some(inner) = type_argument_if_last_segment_ident(ty, "ModelId") else {
        return false;
    };

    type_path_last_segment_matches(inner, &ident.to_string())
        || type_path_last_segment_matches(inner, "Self")
}
