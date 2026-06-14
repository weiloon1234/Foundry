use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::ParseStream;
use syn::spanned::Spanned;
use syn::{DeriveInput, ExprLit, FieldsNamed, Ident, Lit, Type};

use crate::common::{
    ensure_named_struct, require_ident, type_argument_if_last_segment_ident,
    type_path_last_segment_matches,
};

// ---------------------------------------------------------------------------
// Struct-level args: #[validate(messages(...), attributes(...))]
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ValidateArgs {
    messages: Vec<(String, String, String)>, // (field, rule_code, message)
    attributes: Vec<(String, String)>,       // (field, display_name)
}

fn parse_validate_args(attrs: &[syn::Attribute]) -> syn::Result<ValidateArgs> {
    let mut args = ValidateArgs::default();

    for attr in attrs.iter().filter(|a| a.path().is_ident("validate")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("messages") {
                // messages(email(unique = "..."), password(min = "..."))
                meta.parse_nested_meta(|field_meta| {
                    let field = field_meta.path.get_ident()
                        .ok_or_else(|| syn::Error::new(field_meta.path.span(), "expected field name"))?
                        .to_string();
                    field_meta.parse_nested_meta(|rule_meta| {
                        let rule = rule_meta.path.get_ident()
                            .ok_or_else(|| syn::Error::new(rule_meta.path.span(), "expected rule name"))?
                            .to_string();
                        let _: syn::Token![=] = rule_meta.input.parse()?;
                        let value: syn::LitStr = rule_meta.input.parse()?;
                        args.messages.push((field.clone(), rule, value.value()));
                        Ok(())
                    })
                })?;
            } else if meta.path.is_ident("attributes") {
                meta.parse_nested_meta(|inner| {
                    let field = inner
                        .path
                        .get_ident()
                        .ok_or_else(|| {
                            syn::Error::new(
                                inner.path.span(),
                                "attributes key must be a single identifier",
                            )
                        })?
                        .to_string();

                    let _: syn::Token![=] = inner.input.parse()?;
                    let value: syn::LitStr = inner.input.parse()?;
                    args.attributes.push((field, value.value()));
                    Ok(())
                })?;
            } else {
                return Err(meta.error(
                    "unsupported validate struct attribute; expected messages(...) or attributes(...)",
                ));
            }
            Ok(())
        })?;
    }

    Ok(args)
}

// ---------------------------------------------------------------------------
// Field-level parsing: #[validate(rule1, rule2(params), ...)]
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FieldValidation {
    field_ident: Ident,
    field_name: String,
    is_option: bool,
    #[allow(dead_code)]
    is_vec: bool,
    is_uploaded_file: bool,
    rules: Vec<RuleSpec>,
}

/// Information about every struct field, used for FromMultipart generation.
struct FieldInfo {
    ident: Ident,
    name: String,
    is_option: bool,
    is_vec: bool,
    is_uploaded_file: bool,
    is_vec_uploaded_file: bool,
    ty: Type,
}

#[derive(Clone)]
enum RuleSpec {
    Simple {
        name: String,
        message: Option<String>,
    },
    Parametric {
        name: String,
        args: Vec<syn::Expr>,
        message: Option<String>,
    },
    Each {
        rules: Vec<RuleSpec>,
    },
    AppEnum {
        type_path: syn::Path,
    },
    // File validation rules
    Image,
    MaxFileSize(syn::Expr),
    MaxDimensions(syn::Expr, syn::Expr),
    MinDimensions(syn::Expr, syn::Expr),
    AllowedMimes(Vec<syn::Expr>),
    AllowedExtensions(Vec<syn::Expr>),
}

/// Check if a type's last path segment matches the given name.
fn last_segment_is(ty: &Type, name: &str) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .is_some_and(|s| s.ident == name)
}

/// Check if a type is `UploadedFile`, `Option<UploadedFile>`, or `Vec<UploadedFile>`.
fn is_or_wraps_uploaded_file(ty: &Type) -> bool {
    // Direct UploadedFile
    if last_segment_is(ty, "UploadedFile") {
        return true;
    }
    // Option<UploadedFile> or Vec<UploadedFile>
    for wrapper in &["Option", "Vec"] {
        if let Some(inner) = type_argument_if_last_segment_ident(ty, wrapper) {
            if last_segment_is(inner, "UploadedFile") {
                return true;
            }
        }
    }
    false
}

fn is_json_value_type(ty: &Type) -> bool {
    type_path_last_segment_matches(ty, "Value")
}

fn generate_parse_text_value(
    raw_expr: TokenStream,
    target_ty: &Type,
    field_name: &str,
) -> TokenStream {
    if is_json_value_type(target_ty) {
        quote! {
            ::serde_json::from_str::<#target_ty>(&#raw_expr).map_err(|_| {
                ::foundry::foundation::Error::message(format!("field '{}' has invalid JSON", #field_name))
            })?
        }
    } else {
        quote! {
            <#target_ty as ::std::str::FromStr>::from_str(&#raw_expr).map_err(|_| {
                ::foundry::foundation::Error::message(format!("field '{}' has invalid value", #field_name))
            })?
        }
    }
}

fn parse_field_validations(
    fields: &FieldsNamed,
) -> syn::Result<(Vec<FieldValidation>, Vec<FieldInfo>)> {
    let mut validations = Vec::new();
    let mut all_fields = Vec::new();

    for field in &fields.named {
        let field_ident = require_ident(field)?;
        let field_name = field_ident.to_string();
        let field_ty = &field.ty;
        let is_option = type_argument_if_last_segment_ident(field_ty, "Option").is_some();
        let is_vec = type_argument_if_last_segment_ident(field_ty, "Vec").is_some();
        let is_uploaded_file = is_or_wraps_uploaded_file(field_ty);

        // Check for Vec<UploadedFile>
        let is_vec_uploaded_file = is_vec
            && type_argument_if_last_segment_ident(field_ty, "Vec")
                .map(|inner| type_argument_if_last_segment_ident(inner, "UploadedFile").is_some())
                .unwrap_or(false);

        all_fields.push(FieldInfo {
            ident: field_ident.clone(),
            name: field_name.clone(),
            is_option,
            is_vec,
            is_uploaded_file,
            is_vec_uploaded_file,
            ty: field_ty.clone(),
        });

        let mut rules = Vec::new();
        for attr in field.attrs.iter().filter(|a| a.path().is_ident("validate")) {
            let field_rules = parse_field_validate_attr(attr)?;
            rules.extend(field_rules);
        }

        if !rules.is_empty() {
            validations.push(FieldValidation {
                field_ident: field_ident.clone(),
                field_name: field_name.clone(),
                is_option,
                is_vec,
                is_uploaded_file,
                rules,
            });
        }
    }

    Ok((validations, all_fields))
}

fn parse_field_validate_attr(attr: &syn::Attribute) -> syn::Result<Vec<RuleSpec>> {
    let mut rules = Vec::new();

    attr.parse_args_with(|input: ParseStream<'_>| {
        while !input.is_empty() {
            let rule = parse_one_rule(input)?;
            rules.push(rule);
            if !input.is_empty() {
                let _: syn::Token![,] = input.parse()?;
            }
        }
        Ok(())
    })?;

    Ok(rules)
}

fn parse_one_rule(input: ParseStream<'_>) -> syn::Result<RuleSpec> {
    let ident: Ident = input.parse()?;
    let name = ident.to_string();

    if name == "app_enum" {
        if !input.peek(syn::token::Paren) {
            return Err(syn::Error::new(
                ident.span(),
                "`app_enum` requires a type name in parentheses, e.g. `app_enum(UserStatus)`",
            ));
        }
        let content;
        syn::parenthesized!(content in input);
        let type_path: syn::Path = content.parse()?;
        return Ok(RuleSpec::AppEnum { type_path });
    }

    // File validation rules
    if name == "image" {
        if input.peek(syn::token::Paren) {
            return Err(syn::Error::new(
                input.span(),
                "image rule takes no arguments",
            ));
        }
        return Ok(RuleSpec::Image);
    }
    if name == "max_file_size" {
        let content;
        syn::parenthesized!(content in input);
        let kb: syn::Expr = content.parse()?;
        return Ok(RuleSpec::MaxFileSize(kb));
    }
    if name == "max_dimensions" {
        let content;
        syn::parenthesized!(content in input);
        let w: syn::Expr = content.parse()?;
        let _: syn::Token![,] = content.parse()?;
        let h: syn::Expr = content.parse()?;
        return Ok(RuleSpec::MaxDimensions(w, h));
    }
    if name == "min_dimensions" {
        let content;
        syn::parenthesized!(content in input);
        let w: syn::Expr = content.parse()?;
        let _: syn::Token![,] = content.parse()?;
        let h: syn::Expr = content.parse()?;
        return Ok(RuleSpec::MinDimensions(w, h));
    }
    if name == "allowed_mimes" {
        let content;
        syn::parenthesized!(content in input);
        let mut mimes = Vec::new();
        while !content.is_empty() {
            let mime: syn::Expr = content.parse()?;
            mimes.push(mime);
            if !content.is_empty() {
                let _: syn::Token![,] = content.parse()?;
            }
        }
        return Ok(RuleSpec::AllowedMimes(mimes));
    }
    if name == "allowed_extensions" {
        let content;
        syn::parenthesized!(content in input);
        let mut exts = Vec::new();
        while !content.is_empty() {
            let ext: syn::Expr = content.parse()?;
            exts.push(ext);
            if !content.is_empty() {
                let _: syn::Token![,] = content.parse()?;
            }
        }
        return Ok(RuleSpec::AllowedExtensions(exts));
    }

    if name == "each" {
        let content;
        syn::parenthesized!(content in input);
        let mut inner_rules = Vec::new();
        while !content.is_empty() {
            let rule = parse_one_rule(&content)?;
            inner_rules.push(rule);
            if !content.is_empty() {
                let _: syn::Token![,] = content.parse()?;
            }
        }
        return Ok(RuleSpec::Each { rules: inner_rules });
    }

    if input.peek(syn::token::Paren) {
        let content;
        syn::parenthesized!(content in input);
        parse_parametric_rule(name, &content)
    } else {
        Ok(RuleSpec::Simple {
            name,
            message: None,
        })
    }
}

fn parse_parametric_rule(name: String, content: ParseStream<'_>) -> syn::Result<RuleSpec> {
    let mut args = Vec::new();
    let mut message = None;

    while !content.is_empty() {
        // Check for `message = "..."` kwarg
        if content.peek(Ident) && content.peek2(syn::Token![=]) {
            let key: Ident = content.parse()?;
            let _: syn::Token![=] = content.parse()?;
            if key == "message" {
                let val: syn::LitStr = content.parse()?;
                message = Some(val.value());
            } else {
                return Err(syn::Error::new(
                    key.span(),
                    format!("unknown keyword argument `{}`", key),
                ));
            }
        } else {
            let arg: syn::Expr = content.parse()?;
            args.push(arg);
        }

        if !content.is_empty() {
            let _: syn::Token![,] = content.parse()?;
        }
    }

    Ok(RuleSpec::Parametric {
        name,
        args,
        message,
    })
}

// ---------------------------------------------------------------------------
// Code generation
// ---------------------------------------------------------------------------

const CROSS_FIELD_RULES: &[&str] = &[
    "confirmed",
    "same",
    "different",
    "before",
    "after",
    "before_or_equal",
    "after_or_equal",
];

const TWO_STRING_PARAM_RULES: &[&str] = &["unique", "exists"];

const STRING_PARAM_RULES: &[&str] = &["regex", "starts_with", "ends_with"];

const FLOAT_PARAM_RULES: &[&str] = &["min_numeric", "max_numeric"];

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = input.ident.clone();
    let fields = ensure_named_struct(&input)?;
    let args = parse_validate_args(&input.attrs)?;
    let (field_validations, all_fields) = parse_field_validations(fields)?;

    let field_name_set: Vec<String> = all_fields.iter().map(|f| f.name.clone()).collect();

    let validate_stmts = generate_validate_body(&field_validations, &ident, &field_name_set)?;
    let messages_body = generate_messages_body(&args.messages);
    let attributes_body = generate_attributes_body(&args.attributes);

    let from_multipart_impl = generate_from_multipart_impl(&ident, &all_fields)?;
    let ts_validation_registration =
        generate_ts_validation_registration(&ident, &field_validations, &args)?;

    Ok(quote! {
        #[::foundry::__reexports::async_trait]
        impl ::foundry::validation::RequestValidator for #ident {
            async fn validate(&self, validator: &mut ::foundry::validation::Validator) -> ::foundry::foundation::Result<()> {
                #(#validate_stmts)*
                Ok(())
            }

            fn messages(&self) -> Vec<(String, String, String)> {
                #messages_body
            }

            fn attributes(&self) -> Vec<(String, String)> {
                #attributes_body
            }
        }

        #from_multipart_impl
        #ts_validation_registration
    })
}

fn generate_ts_validation_registration(
    ident: &Ident,
    field_validations: &[FieldValidation],
    args: &ValidateArgs,
) -> syn::Result<TokenStream> {
    let name = ident.to_string();
    let fields = field_validations
        .iter()
        .map(|field| {
            let field_name = &field.field_name;
            let rules = generate_ts_validation_rules(&field.rules)?;
            Ok(quote! {
                ::foundry::typescript::TsValidationField {
                    name: #field_name.to_string(),
                    rules: vec![#(#rules),*],
                }
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;

    let messages = args.messages.iter().map(|(field, rule, message)| {
        quote! {
            ::foundry::typescript::TsValidationMessage {
                field: #field.to_string(),
                rule: #rule.to_string(),
                message: #message.to_string(),
            }
        }
    });
    let attributes = args.attributes.iter().map(|(field, name)| {
        quote! {
            ::foundry::typescript::TsValidationAttribute {
                field: #field.to_string(),
                name: #name.to_string(),
            }
        }
    });

    Ok(quote! {
        ::foundry::inventory::submit! {
            ::foundry::typescript::TsValidation {
                name: #name,
                schema_fn: || ::foundry::typescript::TsValidationSchema {
                    fields: vec![#(#fields),*],
                    messages: vec![#(#messages),*],
                    attributes: vec![#(#attributes),*],
                },
            }
        }
    })
}

fn generate_ts_validation_rules(rules: &[RuleSpec]) -> syn::Result<Vec<TokenStream>> {
    rules
        .iter()
        .map(generate_ts_validation_rule)
        .collect::<syn::Result<Vec<_>>>()
}

fn generate_ts_validation_rule(rule: &RuleSpec) -> syn::Result<TokenStream> {
    match rule {
        RuleSpec::Simple { name, message } => Ok(generate_ts_rule(
            name,
            Vec::new(),
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        )),
        RuleSpec::Parametric {
            name,
            args,
            message,
        } => generate_ts_parametric_rule(name, args, message),
        RuleSpec::Each { rules } => {
            let nested = generate_ts_validation_rules(rules)?;
            Ok(generate_ts_rule(
                "each",
                Vec::new(),
                quote!(Vec::new()),
                &None,
                false,
                nested,
            ))
        }
        RuleSpec::AppEnum { type_path } => {
            let values = quote! {{
                let __meta = <#type_path as ::foundry::FoundryAppEnum>::meta();
                __meta
                    .options
                    .iter()
                    .map(|__option| match &__option.value {
                        ::foundry::EnumKey::String(__value) => __value.clone(),
                        ::foundry::EnumKey::Int(__value) => __value.to_string(),
                    })
                    .collect::<Vec<String>>()
            }};
            Ok(generate_ts_rule(
                "app_enum",
                Vec::new(),
                values,
                &None,
                false,
                Vec::new(),
            ))
        }
        RuleSpec::Image => Ok(generate_ts_rule(
            "image",
            Vec::new(),
            quote!(Vec::new()),
            &None,
            true,
            Vec::new(),
        )),
        RuleSpec::MaxFileSize(kb_expr) => Ok(generate_ts_rule(
            "max_file_size",
            vec![("max", quote!((#kb_expr) as u64))],
            quote!(Vec::new()),
            &None,
            true,
            Vec::new(),
        )),
        RuleSpec::MaxDimensions(w_expr, h_expr) => Ok(generate_ts_rule(
            "max_dimensions",
            vec![
                ("width", quote!((#w_expr) as u32)),
                ("height", quote!((#h_expr) as u32)),
            ],
            quote!(Vec::new()),
            &None,
            true,
            Vec::new(),
        )),
        RuleSpec::MinDimensions(w_expr, h_expr) => Ok(generate_ts_rule(
            "min_dimensions",
            vec![
                ("width", quote!((#w_expr) as u32)),
                ("height", quote!((#h_expr) as u32)),
            ],
            quote!(Vec::new()),
            &None,
            true,
            Vec::new(),
        )),
        RuleSpec::AllowedMimes(exprs) => Ok(generate_ts_rule(
            "allowed_mimes",
            Vec::new(),
            quote!(vec![#(#exprs.to_string()),*]),
            &None,
            true,
            Vec::new(),
        )),
        RuleSpec::AllowedExtensions(exprs) => Ok(generate_ts_rule(
            "allowed_extensions",
            Vec::new(),
            quote!(vec![#(#exprs.to_string()),*]),
            &None,
            true,
            Vec::new(),
        )),
    }
}

fn generate_ts_parametric_rule(
    name: &str,
    args: &[syn::Expr],
    message: &Option<String>,
) -> syn::Result<TokenStream> {
    if CROSS_FIELD_RULES.contains(&name) {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "`{}` requires exactly 1 argument (the other field name)",
                    name
                ),
            ));
        }
        let other = extract_string_literal(&args[0], name)?;
        return Ok(generate_ts_rule(
            name,
            vec![("other", quote!(#other))],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if TWO_STRING_PARAM_RULES.contains(&name) {
        if args.len() != 2 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 2 arguments (table, column)", name),
            ));
        }
        let arg0 = &args[0];
        let arg1 = &args[1];
        return Ok(generate_ts_rule(
            name,
            vec![("table", quote!(#arg0)), ("column", quote!(#arg1))],
            quote!(Vec::new()),
            message,
            true,
            Vec::new(),
        ));
    }

    if STRING_PARAM_RULES.contains(&name) {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument", name),
            ));
        }
        let key = if name == "regex" { "pattern" } else { "value" };
        let arg0 = &args[0];
        return Ok(generate_ts_rule(
            name,
            vec![(key, quote!(#arg0))],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if name == "min" || name == "max" {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument", name),
            ));
        }
        let key = name;
        let arg0 = &args[0];
        return Ok(generate_ts_rule(
            name,
            vec![(key, quote!((#arg0) as usize))],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if FLOAT_PARAM_RULES.contains(&name) {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument", name),
            ));
        }
        let key = if name == "min_numeric" { "min" } else { "max" };
        let arg0 = &args[0];
        return Ok(generate_ts_rule(
            name,
            vec![(key, quote!((#arg0) as f64))],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if name == "between" {
        if args.len() != 2 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`between` requires exactly 2 arguments (min, max)",
            ));
        }
        let min = &args[0];
        let max = &args[1];
        return Ok(generate_ts_rule(
            name,
            vec![
                ("min", quote!((#min) as f64)),
                ("max", quote!((#max) as f64)),
            ],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if name == "in_list" || name == "not_in" {
        if args.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires at least 1 argument", name),
            ));
        }
        return Ok(generate_ts_rule(
            name,
            Vec::new(),
            quote!(vec![#(#args.to_string()),*]),
            message,
            false,
            Vec::new(),
        ));
    }

    if name == "rule" {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`rule` requires exactly 1 argument (the rule name string)",
            ));
        }
        let rule_name = extract_string_literal(&args[0], "rule")?;
        return Ok(generate_ts_rule(
            "rule",
            vec![("rule", quote!(#rule_name))],
            quote!(Vec::new()),
            message,
            true,
            Vec::new(),
        ));
    }

    if args.is_empty() {
        return Ok(generate_ts_rule(
            name,
            Vec::new(),
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        format!("unknown parametric validation rule `{}`", name),
    ))
}

fn generate_ts_rule(
    code: &str,
    params: Vec<(&str, TokenStream)>,
    values: TokenStream,
    message: &Option<String>,
    server_only: bool,
    nested_rules: Vec<TokenStream>,
) -> TokenStream {
    let params = params.iter().map(|(name, value)| {
        quote! {
            __params.insert(#name.to_string(), (#value).to_string());
        }
    });
    let message = match message {
        Some(message) => quote!(Some(#message.to_string())),
        None => quote!(None),
    };

    quote! {{
        let mut __params = ::std::collections::BTreeMap::new();
        #(#params)*
        ::foundry::typescript::TsValidationRule {
            code: #code.to_string(),
            params: __params,
            values: #values,
            message: #message,
            server_only: #server_only,
            rules: vec![#(#nested_rules),*],
        }
    }}
}

fn is_file_rule(rule: &RuleSpec) -> bool {
    matches!(
        rule,
        RuleSpec::Image
            | RuleSpec::MaxFileSize(_)
            | RuleSpec::MaxDimensions(_, _)
            | RuleSpec::MinDimensions(_, _)
            | RuleSpec::AllowedMimes(_)
            | RuleSpec::AllowedExtensions(_)
    )
}

fn generate_validate_body(
    field_validations: &[FieldValidation],
    struct_ident: &Ident,
    all_field_names: &[String],
) -> syn::Result<Vec<TokenStream>> {
    let mut stmts = Vec::new();

    for fv in field_validations {
        let field_ident = &fv.field_ident;
        let field_name = &fv.field_name;

        let file_rules: Vec<&RuleSpec> = fv.rules.iter().filter(|r| is_file_rule(r)).collect();
        let text_rules: Vec<&RuleSpec> = fv
            .rules
            .iter()
            .filter(|r| !is_file_rule(r) && !matches!(r, RuleSpec::Each { .. }))
            .collect();

        // If there are file rules on a non-uploaded_file field, emit compile error
        if !file_rules.is_empty() && !fv.is_uploaded_file {
            return Err(syn::Error::new(
                field_ident.span(),
                "file validation rules (image, max_file_size, max_dimensions, min_dimensions, allowed_mimes, allowed_extensions) can only be used on UploadedFile or Option<UploadedFile> fields",
            ));
        }

        let has_each = fv.rules.iter().any(|r| matches!(r, RuleSpec::Each { .. }));
        let each_rules: Vec<&RuleSpec> = fv
            .rules
            .iter()
            .filter(|r| matches!(r, RuleSpec::Each { .. }))
            .collect();

        if has_each {
            if each_rules.len() > 1 {
                return Err(syn::Error::new(
                    field_ident.span(),
                    "only one `each(...)` rule is allowed per field",
                ));
            }

            let RuleSpec::Each { rules } = each_rules[0] else {
                unreachable!()
            };

            let rule_chain =
                generate_rule_chain(rules, struct_ident, all_field_names, field_ident)?;

            stmts.push(quote! {
                validator.each(#field_name, &self.#field_ident)
                    #rule_chain
                    .apply()
                    .await?;
            });
        } else if fv.is_uploaded_file {
            // File field: generate text rule chain (for "required" etc.) + file validation code
            if !text_rules.is_empty() {
                let value_expr = if fv.is_option {
                    quote!(self.#field_ident.as_ref().map(|f| f.original_name.as_deref().unwrap_or("")).unwrap_or(""))
                } else {
                    quote!(self.#field_ident.original_name.as_deref().unwrap_or(""))
                };

                let rule_chain = generate_rule_chain_from_refs(
                    &text_rules,
                    struct_ident,
                    all_field_names,
                    field_ident,
                )?;

                let has_nullable = text_rules.iter().any(|r| match r {
                    RuleSpec::Simple { name, .. } => name == "nullable",
                    RuleSpec::Parametric { name, .. } => name == "nullable",
                    _ => false,
                });

                let nullable_call = if fv.is_option && !has_nullable {
                    quote!(.nullable())
                } else {
                    quote!()
                };

                stmts.push(quote! {
                    validator.field(#field_name, #value_expr)
                        #nullable_call
                        #rule_chain
                        .apply()
                        .await?;
                });
            }

            // Generate file validation code
            if !file_rules.is_empty() {
                let file_validation_code = generate_file_validation_code(fv, field_name)?;
                stmts.push(file_validation_code);
            }
        } else {
            let value_expr = if fv.is_option {
                quote!(self.#field_ident.as_deref().unwrap_or(""))
            } else {
                quote!(&self.#field_ident)
            };

            let rule_chain =
                generate_rule_chain(&fv.rules, struct_ident, all_field_names, field_ident)?;

            let has_nullable = fv.rules.iter().any(|r| match r {
                RuleSpec::Simple { name, .. } => name == "nullable",
                RuleSpec::Parametric { name, .. } => name == "nullable",
                RuleSpec::Each { .. } => false,
                RuleSpec::AppEnum { .. } => false,
                _ => false,
            });

            let nullable_call = if fv.is_option && !has_nullable {
                quote!(.nullable())
            } else {
                quote!()
            };

            stmts.push(quote! {
                validator.field(#field_name, #value_expr)
                    #nullable_call
                    #rule_chain
                    .apply()
                    .await?;
            });
        }
    }

    Ok(stmts)
}

fn generate_file_validation_code(
    fv: &FieldValidation,
    field_name: &str,
) -> syn::Result<TokenStream> {
    let field_ident = &fv.field_ident;
    let field_name_lit = field_name;

    let file_rules: Vec<&RuleSpec> = fv.rules.iter().filter(|r| is_file_rule(r)).collect();
    let mut rule_stmts = Vec::new();

    for rule in &file_rules {
        let stmt = match rule {
            RuleSpec::Image => {
                quote!({
                    let __is_img = ::foundry::validation::file_rules::is_image(__file).await
                        .map_err(|e| ::foundry::foundation::Error::message(e.to_string()))?;
                    if !__is_img {
                        validator.add_error(#field_name_lit, "image", &[]);
                    }
                })
            }
            RuleSpec::MaxFileSize(kb_expr) => {
                quote!({
                    let __max_kb: u64 = (#kb_expr) as u64;
                    if !::foundry::validation::file_rules::check_max_size(__file, __max_kb) {
                        validator.add_error(#field_name_lit, "max_file_size",
                            &[("max", &__max_kb.to_string())]);
                    }
                })
            }
            RuleSpec::MaxDimensions(w_expr, h_expr) => {
                quote!({
                    let __max_w: u32 = (#w_expr) as u32;
                    let __max_h: u32 = (#h_expr) as u32;
                    let (__w, __h) = ::foundry::validation::file_rules::get_image_dimensions(__file).await
                        .map_err(|e| ::foundry::foundation::Error::message(e.to_string()))?;
                    if __w > __max_w || __h > __max_h {
                        validator.add_error(#field_name_lit, "max_dimensions",
                            &[("width", &__max_w.to_string()), ("height", &__max_h.to_string())]);
                    }
                })
            }
            RuleSpec::MinDimensions(w_expr, h_expr) => {
                quote!({
                    let __min_w: u32 = (#w_expr) as u32;
                    let __min_h: u32 = (#h_expr) as u32;
                    let (__w, __h) = ::foundry::validation::file_rules::get_image_dimensions(__file).await
                        .map_err(|e| ::foundry::foundation::Error::message(e.to_string()))?;
                    if __w < __min_w || __h < __min_h {
                        validator.add_error(#field_name_lit, "min_dimensions",
                            &[("width", &__min_w.to_string()), ("height", &__min_h.to_string())]);
                    }
                })
            }
            RuleSpec::AllowedMimes(exprs) => {
                quote!({
                    let __allowed_mimes: Vec<String> = vec![#(#exprs.to_string()),*];
                    let __is_allowed = ::foundry::validation::file_rules::check_allowed_mimes(__file, &__allowed_mimes).await
                        .map_err(|e| ::foundry::foundation::Error::message(e.to_string()))?;
                    if !__is_allowed {
                        validator.add_error(#field_name_lit, "allowed_mimes",
                            &[("mimes", &__allowed_mimes.join(", "))]);
                    }
                })
            }
            RuleSpec::AllowedExtensions(exprs) => {
                quote!({
                    let __allowed_exts: Vec<String> = vec![#(#exprs.to_string()),*];
                    if !::foundry::validation::file_rules::check_allowed_extensions(__file, &__allowed_exts) {
                        validator.add_error(#field_name_lit, "allowed_extensions",
                            &[("extensions", &__allowed_exts.join(", "))]);
                    }
                })
            }
            _ => unreachable!(),
        };
        rule_stmts.push(stmt);
    }

    if fv.is_option {
        // Option<UploadedFile>: wrap in if let Some
        Ok(quote! {
            if let Some(ref __file) = self.#field_ident {
                #(#rule_stmts)*
            }
        })
    } else {
        // UploadedFile (non-Option): direct reference
        Ok(quote! {
            {
                let __file: &::foundry::storage::UploadedFile = &self.#field_ident;
                #(#rule_stmts)*
            }
        })
    }
}

fn generate_rule_chain_from_refs(
    rules: &[&RuleSpec],
    struct_ident: &Ident,
    all_field_names: &[String],
    field_ident: &Ident,
) -> syn::Result<TokenStream> {
    let owned: Vec<RuleSpec> = rules.iter().cloned().cloned().collect();
    generate_rule_chain(&owned, struct_ident, all_field_names, field_ident)
}

fn generate_rule_chain(
    rules: &[RuleSpec],
    struct_ident: &Ident,
    all_field_names: &[String],
    field_ident: &Ident,
) -> syn::Result<TokenStream> {
    let mut tokens = Vec::new();

    for rule in rules {
        match rule {
            RuleSpec::Each { .. } => {
                // Handled at a higher level; skip
            }
            RuleSpec::AppEnum { type_path } => {
                tokens.push(quote!(.app_enum::<#type_path>()));
            }
            RuleSpec::Simple { name, message } => {
                let method = generate_simple_rule_call(name)?;
                let with_msg = generate_with_message(message);
                tokens.push(quote!(.#method #with_msg));
            }
            RuleSpec::Parametric {
                name,
                args,
                message,
            } => {
                let call = generate_parametric_rule_call(
                    name,
                    args,
                    message,
                    struct_ident,
                    all_field_names,
                    field_ident,
                )?;
                tokens.push(call);
            }
            // File rules are handled separately in generate_file_validation_code; skip here
            RuleSpec::Image
            | RuleSpec::MaxFileSize(_)
            | RuleSpec::MaxDimensions(_, _)
            | RuleSpec::MinDimensions(_, _)
            | RuleSpec::AllowedMimes(_)
            | RuleSpec::AllowedExtensions(_) => {}
        }
    }

    Ok(quote! {
        #(#tokens)*
    })
}

fn generate_simple_rule_call(name: &str) -> syn::Result<TokenStream> {
    match name {
        "required" | "email" | "numeric" | "alpha" | "alpha_numeric" | "digits" | "url"
        | "uuid" | "json" | "timezone" | "ip" | "ipv4" | "ipv6" | "date" | "time" | "datetime"
        | "local_datetime" | "integer" | "nullable" | "bail" => {
            let method = syn::Ident::new(name, proc_macro2::Span::call_site());
            Ok(quote!(#method()))
        }
        _ => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("unknown validation rule `{}`", name),
        )),
    }
}

fn generate_parametric_rule_call(
    name: &str,
    args: &[syn::Expr],
    message: &Option<String>,
    struct_ident: &Ident,
    all_field_names: &[String],
    _field_ident: &Ident,
) -> syn::Result<TokenStream> {
    let with_msg = generate_with_message(message);

    // Cross-field rules: confirmed("field"), same("field"), etc.
    if CROSS_FIELD_RULES.contains(&name) {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "`{}` requires exactly 1 argument (the other field name)",
                    name
                ),
            ));
        }
        let other_field_name = extract_string_literal(&args[0], name)?;
        let other_field_ident = syn::Ident::new(&other_field_name, args[0].span());

        if !all_field_names.contains(&other_field_name) {
            return Err(syn::Error::new(
                args[0].span(),
                format!(
                    "field `{}` referenced in `{}` does not exist on struct `{}`",
                    other_field_name, name, struct_ident
                ),
            ));
        }

        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        return Ok(quote!(.#method(#other_field_name, &self.#other_field_ident) #with_msg));
    }

    // Two-string-param rules: unique("table", "col"), exists("table", "col")
    if TWO_STRING_PARAM_RULES.contains(&name) {
        if args.len() != 2 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 2 arguments (table, column)", name),
            ));
        }
        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        let arg0 = &args[0];
        let arg1 = &args[1];
        return Ok(quote!(.#method(#arg0, #arg1) #with_msg));
    }

    // String-param rules: regex("pattern"), starts_with("prefix"), ends_with("suffix")
    if STRING_PARAM_RULES.contains(&name) {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument", name),
            ));
        }
        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        let arg0 = &args[0];
        return Ok(quote!(.#method(#arg0) #with_msg));
    }

    // Numeric size rules: min(N), max(N)
    if name == "min" || name == "max" {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument", name),
            ));
        }
        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        let arg0 = &args[0];
        return Ok(quote!(.#method(#arg0 as usize) #with_msg));
    }

    // Float-param rules: min_numeric(N), max_numeric(N)
    if FLOAT_PARAM_RULES.contains(&name) {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument", name),
            ));
        }
        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        let arg0 = &args[0];
        return Ok(quote!(.#method(#arg0 as f64) #with_msg));
    }

    // between(M, N)
    if name == "between" {
        if args.len() != 2 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`between` requires exactly 2 arguments (min, max)",
            ));
        }
        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        let arg0 = &args[0];
        let arg1 = &args[1];
        return Ok(quote!(.#method(#arg0 as f64, #arg1 as f64) #with_msg));
    }

    // in_list("a", "b", ...), not_in("a", "b", ...)
    if name == "in_list" || name == "not_in" {
        if args.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires at least 1 argument", name),
            ));
        }
        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        return Ok(quote!(.#method(vec![#(#args),*]) #with_msg));
    }

    // Custom rule: rule("rule_name") or rule("rule_name", message = "...")
    if name == "rule" {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`rule` requires exactly 1 argument (the rule name string)",
            ));
        }
        let rule_name = extract_string_literal(&args[0], "rule")?;
        let rule_name_lit = syn::LitStr::new(&rule_name, args[0].span());
        return Ok(quote!(
            .rule(::foundry::support::ValidationRuleId::new(#rule_name_lit))
            #with_msg
        ));
    }

    // Simple rules with message kwarg: required(message = "..."), email(message = "..."), etc.
    if args.is_empty() {
        let call = generate_simple_rule_call(name)?;
        return Ok(quote!(.#call #with_msg));
    }

    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        format!("unknown parametric validation rule `{}`", name),
    ))
}

fn generate_with_message(message: &Option<String>) -> TokenStream {
    match message {
        Some(msg) => quote!(.with_message(#msg)),
        None => quote!(),
    }
}

fn extract_string_literal(expr: &syn::Expr, rule_name: &str) -> syn::Result<String> {
    if let syn::Expr::Lit(ExprLit {
        lit: Lit::Str(s), ..
    }) = expr
    {
        Ok(s.value())
    } else {
        Err(syn::Error::new(
            expr.span(),
            format!("`{}` expects a string literal argument", rule_name),
        ))
    }
}

fn generate_messages_body(messages: &[(String, String, String)]) -> TokenStream {
    if messages.is_empty() {
        return quote!(Vec::new());
    }

    let entries = messages.iter().map(
        |(field, code, msg)| quote!((#field.to_string(), #code.to_string(), #msg.to_string())),
    );

    quote! {
        vec![
            #(#entries),*
        ]
    }
}

fn generate_attributes_body(attributes: &[(String, String)]) -> TokenStream {
    if attributes.is_empty() {
        return quote!(Vec::new());
    }

    let entries = attributes
        .iter()
        .map(|(field, name)| quote!((#field.to_string(), #name.to_string())));

    quote! {
        vec![
            #(#entries),*
        ]
    }
}

// ---------------------------------------------------------------------------
// FromMultipart implementation generation
// ---------------------------------------------------------------------------

fn generate_from_multipart_impl(
    struct_ident: &Ident,
    all_fields: &[FieldInfo],
) -> syn::Result<TokenStream> {
    // Declare Option<T> vars for each field
    let mut var_decls = Vec::new();
    let mut match_arms = Vec::new();
    let mut field_assignments = Vec::new();
    let mut cleanup_uploads = Vec::new();

    for fi in all_fields {
        let ident = &fi.ident;
        let name = &fi.name;
        let var_name = format_ident!("__val_{}", ident);

        if fi.is_vec_uploaded_file {
            // Vec<UploadedFile>: accumulate from multipart
            var_decls.push(quote! {
                let mut #var_name: Vec<::foundry::storage::UploadedFile> = Vec::new();
            });
            cleanup_uploads.push(quote! {
                ::foundry::storage::upload::cleanup_uploaded_files(#var_name.iter()).await;
            });

            match_arms.push(quote! {
                #name => {
                    if let Some(__file) = ::foundry::storage::UploadedFile::from_multipart_field(
                        __field_name,
                        __field,
                        &mut __upload_counters,
                    )
                    .await?
                    {
                        #var_name.push(__file);
                    }
                }
            });

            field_assignments.push(quote! {
                #ident: #var_name.clone()
            });
        } else if fi.is_uploaded_file {
            // UploadedFile or Option<UploadedFile>
            var_decls.push(quote! {
                let mut #var_name: Option<::foundry::storage::UploadedFile> = None;
            });
            cleanup_uploads.push(quote! {
                if let Some(__file) = #var_name.as_ref() {
                    ::foundry::storage::upload::remove_uploaded_temp_file(__file).await;
                }
            });

            match_arms.push(quote! {
                #name => {
                    if let Some(__file) = ::foundry::storage::UploadedFile::from_multipart_field(
                        __field_name,
                        __field,
                        &mut __upload_counters,
                    )
                    .await?
                    {
                        #var_name = Some(__file);
                    }
                }
            });

            if fi.is_option {
                // Option<UploadedFile>: just take the Option
                field_assignments.push(quote! {
                    #ident: #var_name.clone()
                });
            } else {
                // UploadedFile (non-Option): error if missing
                field_assignments.push(quote! {
                    #ident: #var_name.clone().ok_or_else(|| ::foundry::foundation::Error::message(
                        format!("field '{}' is required", #name)
                    ))?
                });
            }
        } else if fi.is_vec {
            // Vec<T>: collect repeated text fields in request order
            let inner_ty = type_argument_if_last_segment_ident(&fi.ty, "Vec")
                .expect("Vec fields should expose an inner type");

            var_decls.push(quote! {
                let mut #var_name: Vec<String> = Vec::new();
            });

            match_arms.push(quote! {
                #name => {
                    let __text = __field.text().await
                        .map_err(|e| ::foundry::foundation::Error::message(format!("field error: {e}")))?;
                    #var_name.push(__text);
                }
            });

            let parse_expr = generate_parse_text_value(quote!(__item), inner_ty, name);
            field_assignments.push(quote! {
                #ident: {
                    let mut __items = Vec::with_capacity(#var_name.len());
                    for __item in #var_name {
                        __items.push(#parse_expr);
                    }
                    __items
                }
            });
        } else if fi.is_option {
            // Option<T>: keep the last text field, parse it when present
            let inner_ty = type_argument_if_last_segment_ident(&fi.ty, "Option")
                .expect("Option fields should expose an inner type");

            var_decls.push(quote! {
                let mut #var_name: Option<String> = None;
            });

            match_arms.push(quote! {
                #name => {
                    let __text = __field.text().await
                        .map_err(|e| ::foundry::foundation::Error::message(format!("field error: {e}")))?;
                    #var_name = Some(__text);
                }
            });

            let parse_expr = generate_parse_text_value(quote!(__val), inner_ty, name);
            field_assignments.push(quote! {
                #ident: match #var_name {
                    Some(__val) => Some(#parse_expr),
                    None => None,
                }
            });
        } else {
            // Text fields (String, i32, bool, JSON, etc.): keep the last text field
            var_decls.push(quote! {
                let mut #var_name: Option<String> = None;
            });

            match_arms.push(quote! {
                #name => {
                    let __text = __field.text().await
                        .map_err(|e| ::foundry::foundation::Error::message(format!("field error: {e}")))?;
                    #var_name = Some(__text);
                }
            });

            let ty = &fi.ty;
            let parse_expr = generate_parse_text_value(quote!(__val), ty, name);
            field_assignments.push(quote! {
                #ident: match #var_name {
                    Some(__val) => #parse_expr,
                    None => ::std::default::Default::default(),
                }
            });
        }
    }

    let parse_error_handling = if cleanup_uploads.is_empty() {
        quote! {
            __parse_result?;
        }
    } else {
        quote! {
            if let Err(__error) = __parse_result {
                #(#cleanup_uploads)*
                return Err(__error);
            }
        }
    };

    let build_result_handling = if cleanup_uploads.is_empty() {
        quote! {
            __build_result
        }
    } else {
        quote! {
            match __build_result {
                Ok(__value) => Ok(__value),
                Err(__error) => {
                    #(#cleanup_uploads)*
                    Err(__error)
                }
            }
        }
    };

    Ok(quote! {
        #[::foundry::__reexports::async_trait]
        impl ::foundry::validation::FromMultipart for #struct_ident {
            async fn from_multipart(
                __multipart: &mut axum::extract::Multipart,
            ) -> ::foundry::foundation::Result<Self> {
                #(#var_decls)*
                let mut __upload_counters = ::foundry::storage::UploadCounters::default();

                let __parse_result: ::foundry::foundation::Result<()> = async {
                    while let Some(__field) = __multipart.next_field().await
                        .map_err(|e| ::foundry::foundation::Error::message(format!("multipart error: {e}")))?
                    {
                        let __field_name = __field.name().unwrap_or("").to_string();
                        match __field_name.as_str() {
                            #(#match_arms)*
                            _ => {}
                        }
                    }
                    Ok(())
                }.await;

                #parse_error_handling

                let __build_result: ::foundry::foundation::Result<Self> = (|| {
                    Ok(Self {
                        #(#field_assignments),*
                    })
                })();

                #build_result_handling
            }
        }
    })
}
