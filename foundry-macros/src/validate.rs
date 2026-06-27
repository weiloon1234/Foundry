use std::collections::{HashMap, HashSet};

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::parse::ParseStream;
use syn::spanned::Spanned;
use syn::{DeriveInput, ExprLit, FieldsNamed, Ident, Lit, Type};

use crate::common::{
    apply_serde_rename_all, consume_meta_value, ensure_named_struct,
    reject_duplicate_contract_field_names, reject_serde_flatten_with_deny_unknown_fields,
    require_ident, rust_ident_name, serde_denies_unknown_fields, serde_has_flatten, serde_rename,
    serde_rename_all, serde_skips_deserializing, should_skip_contract_field,
    type_argument_if_last_segment_ident, type_path_last_segment_matches, wire_field_name,
};

// ---------------------------------------------------------------------------
// Struct-level args: #[validate(messages(...), attributes(...), after(...))]
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ValidateArgs {
    messages: Vec<ValidationMessageArg>,
    attributes: Vec<ValidationAttributeArg>,
    after: Vec<syn::Path>, // async hooks called after generated field rules
}

struct ValidationMessageArg {
    field: String,
    rule: String,
    message: String,
    field_span: Span,
    rule_span: Span,
}

struct ValidationAttributeArg {
    field: String,
    name: String,
    field_span: Span,
}

fn parse_validate_args(attrs: &[syn::Attribute]) -> syn::Result<ValidateArgs> {
    let mut args = ValidateArgs::default();
    let mut seen_messages = HashSet::new();
    let mut seen_attributes = HashSet::new();

    for attr in attrs.iter().filter(|a| a.path().is_ident("validate")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("messages") {
                // messages(email(unique = "..."), password(min = "..."))
                meta.parse_nested_meta(|field_meta| {
                    let field_span = field_meta.path.span();
                    let field = field_meta.path.get_ident()
                        .ok_or_else(|| syn::Error::new(field_meta.path.span(), "expected field name"))?
                        .to_string();
                    field_meta.parse_nested_meta(|rule_meta| {
                        let rule_span = rule_meta.path.span();
                        let raw_rule = rule_meta.path.get_ident()
                            .ok_or_else(|| syn::Error::new(rule_meta.path.span(), "expected rule name"))?
                            .to_string();
                        let rule = canonical_validation_message_rule_code(&raw_rule).to_string();
                        let _: syn::Token![=] = rule_meta.input.parse()?;
                        let value: syn::LitStr = rule_meta.input.parse()?;
                        let message = parse_non_blank_lit_str(
                            value,
                            &format!("validation message for field `{field}` rule `{raw_rule}`"),
                        )?;
                        if !seen_messages.insert((field.clone(), rule.clone())) {
                            return Err(syn::Error::new(
                                rule_meta.path.span(),
                                format!("duplicate validation message for field `{field}` rule `{rule}`"),
                            ));
                        }
                        args.messages.push(ValidationMessageArg {
                            field: field.clone(),
                            rule,
                            message,
                            field_span,
                            rule_span,
                        });
                        Ok(())
                    })
                })?;
            } else if meta.path.is_ident("attributes") {
                meta.parse_nested_meta(|inner| {
                    let field_span = inner.path.span();
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
                    let label = parse_non_blank_lit_str(
                        value,
                        &format!("validation attribute label for field `{field}`"),
                    )?;
                    if !seen_attributes.insert(field.clone()) {
                        return Err(syn::Error::new(
                            inner.path.span(),
                            format!("duplicate validation attribute label for field `{field}`"),
                        ));
                    }
                    args.attributes.push(ValidationAttributeArg {
                        field,
                        name: label,
                        field_span,
                    });
                    Ok(())
                })?;
            } else if meta.path.is_ident("after") {
                let content;
                syn::parenthesized!(content in meta.input);
                let hook: syn::Path = content.parse()?;
                if !content.is_empty() {
                    return Err(syn::Error::new(
                        content.span(),
                        "after(...) accepts exactly one validation hook path",
                    ));
                }
                args.after.push(hook);
            } else {
                return Err(meta.error(
                    "unsupported validate struct attribute; expected messages(...), attributes(...), or after(...)",
                ));
            }
            Ok(())
        })?;
    }

    Ok(args)
}

fn canonical_validation_message_rule_code(rule: &str) -> &str {
    match rule {
        "min_length" => "min",
        "max_length" => "max",
        _ => rule,
    }
}

// ---------------------------------------------------------------------------
// Field-level parsing: #[validate(rule1, rule2(params), ...)]
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FieldValidation {
    field_ident: Ident,
    rust_field_name: String,
    field_name: String,
    nested_ty: Option<Type>,
    nested_item_ty: Option<Type>,
    is_option: bool,
    #[allow(dead_code)]
    is_vec: bool,
    is_map: bool,
    is_numeric: bool,
    is_vec_numeric: bool,
    is_json_value: bool,
    is_uploaded_file: bool,
    ty: Type,
    rules: Vec<RuleSpec>,
}

/// Information about every struct field, used for FromMultipart generation.
struct FieldInfo {
    ident: Ident,
    rust_name: String,
    name: String,
    default: SerdeDefault,
    is_option: bool,
    is_vec: bool,
    is_map: bool,
    is_nested: bool,
    is_vec_nested: bool,
    is_json_value: bool,
    is_uploaded_file: bool,
    is_vec_uploaded_file: bool,
    skips_deserializing: bool,
    ty: Type,
}

struct ParsedFieldValidations {
    validations: Vec<FieldValidation>,
    all_fields: Vec<FieldInfo>,
    contract_fields: Vec<String>,
    wire_names: Vec<(String, String)>,
    struct_default: SerdeDefault,
}

#[derive(Clone)]
enum SerdeDefault {
    None,
    Default,
    Function(syn::Path),
}

impl SerdeDefault {
    fn is_some(&self) -> bool {
        !matches!(self, Self::None)
    }

    fn field_expr(&self, ty: &Type) -> Option<TokenStream> {
        match self {
            Self::None => None,
            Self::Default => Some(quote!(<#ty as ::std::default::Default>::default())),
            Self::Function(path) => Some(quote!(#path())),
        }
    }

    fn struct_expr(&self) -> Option<TokenStream> {
        match self {
            Self::None => None,
            Self::Default => Some(quote!(<Self as ::std::default::Default>::default())),
            Self::Function(path) => Some(quote!(#path())),
        }
    }
}

struct RuleGenerationContext<'a> {
    struct_ident: &'a Ident,
    all_field_names: &'a [String],
    all_fields: &'a [FieldInfo],
    field_wire_names: &'a [(String, String)],
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
    Nested,
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

/// Check if a type is `UploadedFile`, `Option<UploadedFile>`, `Vec<UploadedFile>`,
/// or `Option<Vec<UploadedFile>>`.
fn is_or_wraps_uploaded_file(ty: &Type) -> bool {
    if is_uploaded_file_type(ty) {
        return true;
    }

    if let Some(inner) = type_argument_if_last_segment_ident(ty, "Option") {
        return is_or_wraps_uploaded_file(inner);
    }

    if let Some(inner) = type_argument_if_last_segment_ident(ty, "Vec") {
        return is_uploaded_file_type(inner);
    }

    false
}

fn is_json_value_type(ty: &Type) -> bool {
    type_path_last_segment_matches(ty, "Value")
}

fn is_json_validation_type(ty: &Type) -> bool {
    let ty = type_argument_if_last_segment_ident(ty, "Option").unwrap_or(ty);
    is_json_value_type(ty)
}

fn is_numeric_validation_type(ty: &Type) -> bool {
    let ty = type_argument_if_last_segment_ident(ty, "Option").unwrap_or(ty);
    [
        "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
        "f32", "f64",
    ]
    .iter()
    .any(|name| type_path_last_segment_matches(ty, name))
}

fn direct_nested_type(ty: &Type) -> &Type {
    type_argument_if_last_segment_ident(ty, "Option").unwrap_or(ty)
}

fn vec_item_type(ty: &Type) -> Option<&Type> {
    let ty = type_argument_if_last_segment_ident(ty, "Option").unwrap_or(ty);
    type_argument_if_last_segment_ident(ty, "Vec")
}

fn string_keyed_map_value_type(ty: &Type) -> Option<&Type> {
    let ty = type_argument_if_last_segment_ident(ty, "Option").unwrap_or(ty);
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "HashMap" && segment.ident != "BTreeMap" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let mut args = args.args.iter();
    let Some(syn::GenericArgument::Type(key_ty)) = args.next() else {
        return None;
    };
    if !type_path_last_segment_matches(key_ty, "String") {
        return None;
    }
    let Some(syn::GenericArgument::Type(value_ty)) = args.next() else {
        return None;
    };
    Some(value_ty)
}

fn is_string_keyed_map_type(ty: &Type) -> bool {
    string_keyed_map_value_type(ty).is_some()
}

fn is_vec_type(ty: &Type) -> bool {
    vec_item_type(ty).is_some()
}

fn is_uploaded_file_type(ty: &Type) -> bool {
    last_segment_is(ty, "UploadedFile")
}

fn generate_parse_text_value(
    raw_expr: TokenStream,
    target_ty: &Type,
    field_name: &str,
) -> TokenStream {
    if is_json_value_type(target_ty) || is_string_keyed_map_type(target_ty) {
        quote! {
            ::foundry::serde_json::from_str::<#target_ty>(&#raw_expr).map_err(|_| {
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

fn generate_parse_json_value(
    raw_expr: TokenStream,
    target_ty: &Type,
    field_name: &str,
) -> TokenStream {
    quote! {
        ::foundry::serde_json::from_str::<#target_ty>(&#raw_expr).map_err(|_| {
            ::foundry::foundation::Error::message(format!("field '{}' has invalid JSON", #field_name))
        })?
    }
}

fn serde_default(attrs: &[syn::Attribute]) -> syn::Result<SerdeDefault> {
    let mut default = SerdeDefault::None;

    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("default") {
                if default.is_some() {
                    return Err(meta.error("duplicate serde `default` attribute"));
                }

                if meta.input.peek(syn::Token![=]) {
                    let value: syn::LitStr = meta.value()?.parse()?;
                    let path = syn::parse_str::<syn::Path>(&value.value()).map_err(|error| {
                        syn::Error::new(
                            value.span(),
                            format!("invalid serde default path: {error}"),
                        )
                    })?;
                    default = SerdeDefault::Function(path);
                } else {
                    default = SerdeDefault::Default;
                }
            } else {
                consume_meta_value(meta)?;
            }

            Ok(())
        })?;
    }

    Ok(default)
}

fn parse_field_validations(
    fields: &FieldsNamed,
    attrs: &[syn::Attribute],
) -> syn::Result<ParsedFieldValidations> {
    let mut validations = Vec::new();
    let mut all_fields = Vec::new();
    let mut contract_fields = Vec::new();
    let rename_all = serde_rename_all(attrs)?;
    let struct_default = serde_default(attrs)?;
    let mut field_wire_names = Vec::new();

    reject_duplicate_contract_field_names(fields, attrs)?;

    for field in &fields.named {
        let field_ident = require_ident(field)?;
        let rust_name = rust_ident_name(field_ident);
        let field_name = serde_rename(&field.attrs)?
            .unwrap_or_else(|| apply_serde_rename_all(rename_all, &rust_name));
        field_wire_names.push((rust_name, field_name));
    }

    for field in &fields.named {
        let field_ident = require_ident(field)?;
        let rust_name = rust_ident_name(field_ident);
        let field_name = wire_field_name(&rust_name, &field_wire_names);
        let skipped_input_attr = serde_skips_deserializing(&field.attrs)?;
        let is_contract_field = !should_skip_contract_field(field)? && skipped_input_attr.is_none();
        let default = serde_default(&field.attrs)?;
        let field_ty = &field.ty;
        let is_option = type_argument_if_last_segment_ident(field_ty, "Option").is_some();
        let is_vec = is_vec_type(field_ty);
        let is_map = is_string_keyed_map_type(field_ty);
        let is_numeric = is_numeric_validation_type(field_ty);
        let is_vec_numeric = vec_item_type(field_ty)
            .map(is_numeric_validation_type)
            .unwrap_or(false);
        let is_json_value = is_json_validation_type(field_ty);
        let is_uploaded_file = is_or_wraps_uploaded_file(field_ty);

        let mut rules = Vec::new();
        for attr in field.attrs.iter().filter(|a| a.path().is_ident("validate")) {
            let field_rules = parse_field_validate_attr(attr)?;
            rules.extend(field_rules);
        }
        if !rules.is_empty() && serde_has_flatten(&field.attrs)? {
            return Err(syn::Error::new_spanned(
                field,
                "`#[serde(flatten)]` fields cannot carry `#[validate(...)]` rules because validation error paths and generated TypeScript metadata cannot safely represent flattened child fields; move validation to explicit parent fields or avoid flattening this DTO",
            ));
        }
        if !rules.is_empty() {
            if let Some(attr) = skipped_input_attr {
                return Err(syn::Error::new_spanned(
                    field,
                    format!("`#[serde({attr})]` fields cannot carry `#[validate(...)]` rules because serde ignores them during request deserialization; remove the validation rule, remove the serde skip, or move the value into an explicit request DTO field"),
                ));
            }
        }

        let nested_ty = rules
            .iter()
            .any(|rule| matches!(rule, RuleSpec::Nested))
            .then(|| direct_nested_type(field_ty).clone());
        let nested_item_ty = rules
            .iter()
            .filter_map(|rule| match rule {
                RuleSpec::Each { rules } => rules
                    .iter()
                    .any(|rule| matches!(rule, RuleSpec::Nested))
                    .then(|| vec_item_type(field_ty).cloned())
                    .flatten(),
                _ => None,
            })
            .next();

        // Check for Vec<UploadedFile>
        let is_vec_uploaded_file = vec_item_type(field_ty)
            .map(is_uploaded_file_type)
            .unwrap_or(false);

        all_fields.push(FieldInfo {
            ident: field_ident.clone(),
            rust_name: rust_name.clone(),
            name: field_name.clone(),
            default,
            is_option,
            is_vec,
            is_map,
            is_nested: nested_ty.is_some(),
            is_vec_nested: nested_item_ty.is_some(),
            is_json_value,
            is_uploaded_file,
            is_vec_uploaded_file,
            skips_deserializing: skipped_input_attr.is_some(),
            ty: field_ty.clone(),
        });

        if is_contract_field {
            contract_fields.push(field_name.clone());
        }

        if !rules.is_empty() {
            validations.push(FieldValidation {
                field_ident: field_ident.clone(),
                rust_field_name: rust_name.clone(),
                field_name: field_name.clone(),
                nested_ty,
                nested_item_ty,
                is_option,
                is_vec,
                is_map,
                is_numeric,
                is_vec_numeric,
                is_json_value,
                is_uploaded_file,
                ty: field_ty.clone(),
                rules,
            });
        }
    }

    Ok(ParsedFieldValidations {
        validations,
        all_fields,
        contract_fields,
        wire_names: field_wire_names,
        struct_default,
    })
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

fn validate_struct_metadata_targets(
    args: &ValidateArgs,
    parsed_fields: &ParsedFieldValidations,
) -> syn::Result<()> {
    let has_after_hooks = !args.after.is_empty();
    let field_validations = parsed_fields
        .validations
        .iter()
        .map(|field| (field.rust_field_name.as_str(), field))
        .collect::<HashMap<_, _>>();
    let contract_wire_fields = parsed_fields
        .contract_fields
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let contract_fields = parsed_fields
        .all_fields
        .iter()
        .filter(|field| contract_wire_fields.contains(field.name.as_str()))
        .map(|field| field.rust_name.as_str())
        .collect::<HashSet<_>>();

    for message in &args.messages {
        let Some(field) = field_validations.get(message.field.as_str()) else {
            if has_after_hooks && contract_fields.contains(message.field.as_str()) {
                continue;
            }
            return Err(syn::Error::new(
                message.field_span,
                format!(
                    "validation message references unknown field `{}`; add a field-level validation rule, target an after-hook contract field, or remove the custom message",
                    message.field
                ),
            ));
        };

        let mut rule_codes = HashSet::new();
        let mut has_dynamic_custom_rule = false;
        collect_reachable_message_rule_codes(
            &field.rules,
            &mut rule_codes,
            &mut has_dynamic_custom_rule,
        );
        if !rule_codes.contains(&message.rule) && !has_dynamic_custom_rule && !has_after_hooks {
            return Err(syn::Error::new(
                message.rule_span,
                format!(
                    "validation message for field `{}` references rule `{}` but no rule with that code is reachable from the field metadata",
                    message.field, message.rule
                ),
            ));
        }
    }

    for attribute in &args.attributes {
        let targets_validated_field = field_validations.contains_key(attribute.field.as_str());
        let targets_after_hook_field =
            has_after_hooks && contract_fields.contains(attribute.field.as_str());
        if !(targets_validated_field || targets_after_hook_field) {
            return Err(syn::Error::new(
                attribute.field_span,
                format!(
                    "validation attribute references unknown field `{}`; add a field-level validation rule, target an after-hook contract field, or remove the custom attribute",
                    attribute.field
                ),
            ));
        }
    }

    Ok(())
}

fn collect_reachable_message_rule_codes(
    rules: &[RuleSpec],
    codes: &mut HashSet<String>,
    has_dynamic_custom_rule: &mut bool,
) {
    for rule in rules {
        match rule {
            RuleSpec::Simple { name, .. } | RuleSpec::Parametric { name, .. }
                if matches!(name.as_str(), "nullable" | "bail" | "nested") => {}
            RuleSpec::Simple { name, .. } => {
                codes.insert(canonical_validation_message_rule_code(name).to_string());
            }
            RuleSpec::Parametric { name, args, .. } if name == "rule" => {
                if let [arg] = args.as_slice() {
                    if let Ok(rule_name) = extract_string_literal(arg, "rule") {
                        codes.insert(rule_name);
                    } else {
                        *has_dynamic_custom_rule = true;
                    }
                } else {
                    *has_dynamic_custom_rule = true;
                }
            }
            RuleSpec::Parametric { name, .. } => {
                codes.insert(canonical_validation_message_rule_code(name).to_string());
            }
            RuleSpec::Each { rules } => {
                collect_reachable_message_rule_codes(rules, codes, has_dynamic_custom_rule);
            }
            RuleSpec::AppEnum { .. } => {
                codes.insert("app_enum".to_string());
            }
            RuleSpec::Image => {
                codes.insert("image".to_string());
            }
            RuleSpec::MaxFileSize(_) => {
                codes.insert("max_file_size".to_string());
            }
            RuleSpec::MaxDimensions(_, _) => {
                codes.insert("max_dimensions".to_string());
            }
            RuleSpec::MinDimensions(_, _) => {
                codes.insert("min_dimensions".to_string());
            }
            RuleSpec::AllowedMimes(_) => {
                codes.insert("allowed_mimes".to_string());
            }
            RuleSpec::AllowedExtensions(_) => {
                codes.insert("allowed_extensions".to_string());
            }
            RuleSpec::Nested => {}
        }
    }
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
        ensure_rule_args_consumed(&content, "app_enum", "exactly one AppEnum type argument")?;
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
        ensure_rule_args_consumed(&content, "max_file_size", "exactly 1 argument")?;
        return Ok(RuleSpec::MaxFileSize(kb));
    }
    if name == "max_dimensions" {
        let content;
        syn::parenthesized!(content in input);
        let w: syn::Expr = content.parse()?;
        let _: syn::Token![,] = content.parse()?;
        let h: syn::Expr = content.parse()?;
        ensure_rule_args_consumed(&content, "max_dimensions", "exactly 2 arguments")?;
        return Ok(RuleSpec::MaxDimensions(w, h));
    }
    if name == "min_dimensions" {
        let content;
        syn::parenthesized!(content in input);
        let w: syn::Expr = content.parse()?;
        let _: syn::Token![,] = content.parse()?;
        let h: syn::Expr = content.parse()?;
        ensure_rule_args_consumed(&content, "min_dimensions", "exactly 2 arguments")?;
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
        ensure_non_empty_rule_args(&mimes, "allowed_mimes")?;
        ensure_non_blank_trimmed_file_rule_values(&mimes, "allowed_mimes")?;
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
        ensure_non_empty_rule_args(&exts, "allowed_extensions")?;
        ensure_non_blank_trimmed_file_rule_values(&exts, "allowed_extensions")?;
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
        ensure_non_empty_rule_args(&inner_rules, "each")?;
        return Ok(RuleSpec::Each { rules: inner_rules });
    }

    if name == "nested" {
        if input.peek(syn::token::Paren) {
            return Err(syn::Error::new(
                input.span(),
                "nested rule takes no arguments",
            ));
        }
        return Ok(RuleSpec::Nested);
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

fn ensure_rule_args_consumed(
    content: ParseStream<'_>,
    rule_name: &str,
    expected: &str,
) -> syn::Result<()> {
    if !content.is_empty() {
        return Err(syn::Error::new(
            content.span(),
            format!("`{rule_name}` requires {expected}"),
        ));
    }
    Ok(())
}

fn ensure_non_empty_rule_args<T>(args: &[T], rule_name: &str) -> syn::Result<()> {
    if args.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{rule_name}` requires at least 1 argument"),
        ));
    }
    Ok(())
}

fn ensure_non_blank_trimmed_file_rule_values(
    values: &[syn::Expr],
    rule_name: &str,
) -> syn::Result<()> {
    for value in values {
        if let syn::Expr::Lit(ExprLit {
            lit: Lit::Str(lit), ..
        }) = value
        {
            let text = lit.value();
            if text.trim().is_empty() || text.trim() != text {
                return Err(syn::Error::new(
                    lit.span(),
                    format!("`{rule_name}` values must be non-empty and trimmed"),
                ));
            }
        }
    }
    Ok(())
}

fn parse_non_blank_lit_str(value: syn::LitStr, description: &str) -> syn::Result<String> {
    let text = value.value();
    if text.trim().is_empty() {
        return Err(syn::Error::new(
            value.span(),
            format!("{description} must not be blank"),
        ));
    }
    Ok(text)
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
                if message.is_some() {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("validation rule `{name}` declares duplicate `message` override"),
                    ));
                }
                message = Some(parse_non_blank_lit_str(
                    val,
                    &format!("validation rule `{name}` message override"),
                )?);
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
    "date_equals",
];

const CONDITIONAL_VALUE_RULES: &[&str] = &[
    "required_if",
    "required_unless",
    "accepted_if",
    "declined_if",
];

const CONDITIONAL_PROHIBITED_VALUE_RULES: &[&str] = &["prohibited_if", "prohibited_unless"];

const CONDITIONAL_REQUIRED_FIELD_RULES: &[&str] = &[
    "required_if_accepted",
    "required_if_declined",
    "required_with",
    "required_without",
];

const CONDITIONAL_PROHIBITED_FIELD_RULES: &[&str] =
    &["prohibited_if_accepted", "prohibited_if_declined"];

const CONDITIONAL_REQUIRED_ALL_FIELD_RULES: &[&str] =
    &["required_with_all", "required_without_all"];
const CONDITIONAL_PROHIBITED_ALL_FIELD_RULES: &[&str] = &["prohibits"];

const TWO_STRING_PARAM_RULES: &[&str] = &["unique", "exists"];

const MULTI_STRING_PARAM_RULES: &[&str] = &[
    "starts_with",
    "doesnt_start_with",
    "ends_with",
    "doesnt_end_with",
];

const STRING_PARAM_RULES: &[&str] = &["regex", "not_regex", "contains", "doesnt_contain"];

const FLOAT_PARAM_RULES: &[&str] = &[
    "min_numeric",
    "max_numeric",
    "multiple_of",
    "gt",
    "gte",
    "lt",
    "lte",
];

const DIGIT_PARAM_RULES: &[&str] = &["min_digits", "max_digits"];

const COLLECTION_PARAM_RULES: &[&str] = &["min_items", "max_items"];

const COLLECTION_VALUE_RULES: &[&str] = &["contains", "doesnt_contain"];

const COLLECTION_SIMPLE_RULES: &[&str] = &["distinct"];

const MAP_PARAM_RULES: &[&str] = &["required_keys"];

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = input.ident.clone();
    let fields = ensure_named_struct(&input)?;
    let args = parse_validate_args(&input.attrs)?;
    let deny_unknown_fields = serde_denies_unknown_fields(&input.attrs)?;
    if deny_unknown_fields {
        for field in &fields.named {
            if !should_skip_contract_field(field)? {
                reject_serde_flatten_with_deny_unknown_fields(field)?;
            }
        }
    }
    let parsed_fields = parse_field_validations(fields, &input.attrs)?;
    validate_struct_metadata_targets(&args, &parsed_fields)?;

    let field_name_set: Vec<String> = parsed_fields
        .all_fields
        .iter()
        .map(|f| f.rust_name.clone())
        .collect();

    let validate_stmts = generate_validate_body(
        &parsed_fields.validations,
        &ident,
        &field_name_set,
        &parsed_fields.all_fields,
        &parsed_fields.wire_names,
    )?;
    let after_stmts = args.after.iter().map(|hook| {
        quote! {
            #hook(self, validator).await?;
        }
    });
    let messages_body = generate_messages_body(&args.messages, &parsed_fields.wire_names);
    let attributes_body = generate_attributes_body(&args.attributes, &parsed_fields.wire_names);

    let from_multipart_impl = generate_from_multipart_impl(
        &ident,
        &parsed_fields.all_fields,
        &parsed_fields.struct_default,
    )?;
    let ts_validation_registration = generate_ts_validation_registration(
        &ident,
        &parsed_fields.validations,
        &parsed_fields.all_fields,
        &args,
        &parsed_fields.contract_fields,
        &parsed_fields.wire_names,
        deny_unknown_fields,
    )?;

    Ok(quote! {
        #[::foundry::__reexports::async_trait]
        impl ::foundry::validation::RequestValidator for #ident {
            async fn validate(&self, validator: &mut ::foundry::validation::Validator) -> ::foundry::foundation::Result<()> {
                #(#validate_stmts)*
                #(#after_stmts)*
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
    all_fields: &[FieldInfo],
    args: &ValidateArgs,
    contract_fields: &[String],
    field_wire_names: &[(String, String)],
    deny_unknown_fields: bool,
) -> syn::Result<TokenStream> {
    let name = ident.to_string();
    let contract_field_set = contract_fields.iter().collect::<HashSet<_>>();
    let fields = field_validations
        .iter()
        .map(|field| {
            let field_name = &field.field_name;
            let mut rules = generate_ts_validation_rules(
                &field.rules,
                field.is_vec,
                field.is_json_value,
                Some(&field.rust_field_name),
                field.nested_ty.as_ref(),
                field.nested_item_ty.as_ref(),
                field_wire_names,
            )?;
            if should_auto_nullable(field.is_option, &field.rules) {
                rules.insert(
                    0,
                    generate_ts_rule(
                        "nullable",
                        Vec::new(),
                        quote!(Vec::new()),
                        &None,
                        false,
                        Vec::new(),
                    ),
                );
            }
            Ok(quote! {
                ::foundry::typescript::TsValidationField {
                    name: #field_name.to_string(),
                    rules: vec![#(#rules),*],
                }
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let field_value_kinds = all_fields
        .iter()
        .filter(|field| contract_field_set.contains(&field.name))
        .filter_map(generate_ts_field_value_kind_entry)
        .collect::<Vec<_>>();

    let messages = args.messages.iter().map(|message| {
        let field = wire_field_name(&message.field, field_wire_names);
        let rule = &message.rule;
        let message = &message.message;
        quote! {
            ::foundry::typescript::TsValidationMessage {
                field: #field.to_string(),
                rule: #rule.to_string(),
                message: #message.to_string(),
            }
        }
    });
    let attributes = args.attributes.iter().map(|attribute| {
        let field = wire_field_name(&attribute.field, field_wire_names);
        let name = &attribute.name;
        quote! {
            ::foundry::typescript::TsValidationAttribute {
                field: #field.to_string(),
                name: #name.to_string(),
            }
        }
    });
    let schema_rules = args.after.iter().map(|hook| {
        let hook_name = validation_hook_name(hook);
        generate_ts_rule(
            "after",
            vec![("hook", quote!(#hook_name))],
            quote!(Vec::new()),
            &None,
            true,
            Vec::new(),
        )
    });
    let known_fields = if deny_unknown_fields || !args.after.is_empty() {
        quote!(vec![#(#contract_fields.to_string()),*])
    } else {
        quote!(Vec::new())
    };

    let schema = quote! {
        ::foundry::typescript::TsValidationSchema {
            deny_unknown_fields: #deny_unknown_fields,
            known_fields: #known_fields,
            field_value_kinds: vec![#(#field_value_kinds),*],
            rules: vec![#(#schema_rules),*],
            fields: vec![#(#fields),*],
            messages: vec![#(#messages),*],
            attributes: vec![#(#attributes),*],
        }
    };

    Ok(quote! {
        impl ::foundry::typescript::TsValidationSchemaProvider for #ident {
            fn ts_validation_schema() -> ::foundry::typescript::TsValidationSchema {
                #schema
            }
        }

        ::foundry::inventory::submit! {
            ::foundry::typescript::TsValidation {
                name: #name,
                schema_fn: || <#ident as ::foundry::typescript::TsValidationSchemaProvider>::ts_validation_schema(),
            }
        }
    })
}

fn generate_ts_field_value_kind_entry(field: &FieldInfo) -> Option<TokenStream> {
    let kind = if field.is_vec || field.is_vec_uploaded_file {
        quote!(::foundry::typescript::TsValidationFieldValueKind::Array)
    } else if field.is_map {
        quote!(::foundry::typescript::TsValidationFieldValueKind::Map)
    } else if field.is_json_value {
        quote!(::foundry::typescript::TsValidationFieldValueKind::Json)
    } else if field.is_uploaded_file {
        quote!(::foundry::typescript::TsValidationFieldValueKind::File)
    } else if field.is_nested {
        quote!(::foundry::typescript::TsValidationFieldValueKind::Nested)
    } else {
        return None;
    };
    let name = &field.name;
    Some(quote! {
        ::foundry::typescript::TsValidationFieldValueKindEntry {
            field: #name.to_string(),
            kind: #kind,
        }
    })
}

fn generate_ts_validation_rules(
    rules: &[RuleSpec],
    is_collection_context: bool,
    is_json_value_context: bool,
    rust_field_name: Option<&str>,
    nested_ty: Option<&Type>,
    nested_item_ty: Option<&Type>,
    field_wire_names: &[(String, String)],
) -> syn::Result<Vec<TokenStream>> {
    rules
        .iter()
        .map(|rule| {
            generate_ts_validation_rule(
                rule,
                is_collection_context,
                is_json_value_context,
                rust_field_name,
                nested_ty,
                nested_item_ty,
                field_wire_names,
            )
        })
        .collect::<syn::Result<Vec<_>>>()
}

fn generate_ts_validation_rule(
    rule: &RuleSpec,
    is_collection_context: bool,
    is_json_value_context: bool,
    rust_field_name: Option<&str>,
    nested_ty: Option<&Type>,
    nested_item_ty: Option<&Type>,
    field_wire_names: &[(String, String)],
) -> syn::Result<TokenStream> {
    match rule {
        RuleSpec::Simple { name, message } if name == "confirmed" => {
            generate_ts_default_confirmed_rule(rust_field_name, message, field_wire_names)
        }
        RuleSpec::Simple { name, message } if name == "json" && is_json_value_context => {
            Ok(generate_ts_rule(
                name,
                Vec::new(),
                quote!(Vec::new()),
                message,
                true,
                Vec::new(),
            ))
        }
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
        } if name == "confirmed" && args.is_empty() => {
            generate_ts_default_confirmed_rule(rust_field_name, message, field_wire_names)
        }
        RuleSpec::Parametric {
            name,
            args,
            message,
        } => generate_ts_parametric_rule(
            name,
            args,
            message,
            is_collection_context,
            field_wire_names,
        ),
        RuleSpec::Each { rules } => {
            let nested = generate_ts_validation_rules(
                rules,
                false,
                false,
                None,
                nested_item_ty,
                None,
                field_wire_names,
            )?;
            Ok(generate_ts_rule(
                "each",
                Vec::new(),
                quote!(Vec::new()),
                &None,
                false,
                nested,
            ))
        }
        RuleSpec::Nested => generate_ts_nested_rule(nested_ty),
        RuleSpec::AppEnum { type_path } => {
            let values = quote! {{
                <#type_path as ::foundry::FoundryAppEnum>::accepted_keys().into_vec()
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
            false,
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
            quote!(vec![#((#exprs).to_string()),*]),
            &None,
            true,
            Vec::new(),
        )),
        RuleSpec::AllowedExtensions(exprs) => Ok(generate_ts_rule(
            "allowed_extensions",
            Vec::new(),
            quote!(vec![#((#exprs).to_string()),*]),
            &None,
            false,
            Vec::new(),
        )),
    }
}

fn generate_ts_default_confirmed_rule(
    rust_field_name: Option<&str>,
    message: &Option<String>,
    field_wire_names: &[(String, String)],
) -> syn::Result<TokenStream> {
    let rust_field_name = rust_field_name.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "`confirmed` without arguments is only supported on a struct field",
        )
    })?;
    let other_rust_field_name = default_confirmation_field_name(rust_field_name);
    let other_field_name = wire_field_name(&other_rust_field_name, field_wire_names);
    Ok(generate_ts_rule(
        "confirmed",
        vec![("other", quote!(#other_field_name))],
        quote!(Vec::new()),
        message,
        false,
        Vec::new(),
    ))
}

fn generate_ts_parametric_rule(
    name: &str,
    args: &[syn::Expr],
    message: &Option<String>,
    is_collection_context: bool,
    field_wire_names: &[(String, String)],
) -> syn::Result<TokenStream> {
    if MAP_PARAM_RULES.contains(&name) {
        if args.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires at least 1 argument", name),
            ));
        }
        return Ok(generate_ts_rule(
            name,
            Vec::new(),
            quote!(vec![#((#args).to_string()),*]),
            message,
            false,
            Vec::new(),
        ));
    }

    if CONDITIONAL_VALUE_RULES.contains(&name) {
        if args.len() != 2 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 2 arguments (field, value)", name),
            ));
        }
        let other = wire_field_name(&extract_string_literal(&args[0], name)?, field_wire_names);
        let expected = &args[1];
        return Ok(generate_ts_rule(
            name,
            vec![("other", quote!(#other)), ("value", quote!(#expected))],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if CONDITIONAL_PROHIBITED_VALUE_RULES.contains(&name) {
        if args.len() != 2 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 2 arguments (field, value)", name),
            ));
        }
        let other = wire_field_name(&extract_string_literal(&args[0], name)?, field_wire_names);
        let expected = &args[1];
        return Ok(generate_ts_rule(
            name,
            vec![("other", quote!(#other)), ("value", quote!(#expected))],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if CONDITIONAL_REQUIRED_FIELD_RULES.contains(&name) {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument (field)", name),
            ));
        }
        let other = wire_field_name(&extract_string_literal(&args[0], name)?, field_wire_names);
        return Ok(generate_ts_rule(
            name,
            vec![("other", quote!(#other))],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if CONDITIONAL_PROHIBITED_FIELD_RULES.contains(&name) {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument (field)", name),
            ));
        }
        let other = wire_field_name(&extract_string_literal(&args[0], name)?, field_wire_names);
        return Ok(generate_ts_rule(
            name,
            vec![("other", quote!(#other))],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if CONDITIONAL_REQUIRED_ALL_FIELD_RULES.contains(&name) {
        if args.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires at least 1 argument (field)", name),
            ));
        }
        let fields = args
            .iter()
            .map(|arg| extract_string_literal(arg, name))
            .collect::<syn::Result<Vec<_>>>()?;
        let fields = fields
            .iter()
            .map(|field| wire_field_name(field, field_wire_names))
            .collect::<Vec<_>>();
        let other = fields.join(", ");
        return Ok(generate_ts_rule(
            name,
            vec![("other", quote!(#other))],
            quote!(vec![#(#fields.to_string()),*]),
            message,
            false,
            Vec::new(),
        ));
    }

    if CONDITIONAL_PROHIBITED_ALL_FIELD_RULES.contains(&name) {
        if args.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires at least 1 argument (field)", name),
            ));
        }
        let fields = args
            .iter()
            .map(|arg| extract_string_literal(arg, name))
            .collect::<syn::Result<Vec<_>>>()?;
        let fields = fields
            .iter()
            .map(|field| wire_field_name(field, field_wire_names))
            .collect::<Vec<_>>();
        let other = fields.join(", ");
        return Ok(generate_ts_rule(
            name,
            vec![("other", quote!(#other))],
            quote!(vec![#(#fields.to_string()),*]),
            message,
            false,
            Vec::new(),
        ));
    }

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
        let other = wire_field_name(&extract_string_literal(&args[0], name)?, field_wire_names);
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

    if MULTI_STRING_PARAM_RULES.contains(&name) {
        if args.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires at least 1 argument", name),
            ));
        }
        return Ok(generate_ts_rule(
            name,
            vec![(
                "value",
                quote!({
                    let __values: Vec<String> = vec![#((#args).to_string()),*];
                    __values.join(", ")
                }),
            )],
            quote!(vec![#((#args).to_string()),*]),
            message,
            false,
            Vec::new(),
        ));
    }

    if is_collection_context && COLLECTION_VALUE_RULES.contains(&name) {
        if args.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires at least 1 argument", name),
            ));
        }
        return Ok(generate_ts_rule(
            name,
            vec![(
                "value",
                quote!({
                    let __values: Vec<String> = vec![#((#args).to_string()),*];
                    __values.join(", ")
                }),
            )],
            quote!(vec![#((#args).to_string()),*]),
            message,
            false,
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
        if name == "regex" || name == "not_regex" {
            return Ok(generate_ts_regex_rule(name, &args[0], message));
        }
        let arg0 = &args[0];
        return Ok(generate_ts_rule(
            name,
            vec![("value", quote!(#arg0))],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if name == "uuid" {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`uuid` requires exactly 1 argument when constraining the UUID version",
            ));
        }
        let version = &args[0];
        return Ok(generate_ts_rule(
            "uuid",
            vec![("version", quote!((#version) as u8))],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if matches!(name, "min" | "max" | "min_length" | "max_length") {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument", name),
            ));
        }
        let key = if matches!(name, "min" | "min_length") {
            "min"
        } else {
            "max"
        };
        let arg0 = &args[0];
        return Ok(generate_ts_rule(
            key,
            vec![(key, quote!((#arg0) as usize))],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if name == "size" {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`size` requires exactly 1 argument",
            ));
        }
        let arg0 = &args[0];
        let mut params = vec![("size", quote!(#arg0))];
        if is_collection_context {
            params.push(("kind", quote!("array")));
        }
        return Ok(generate_ts_rule(
            "size",
            params,
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
        let key = match name {
            "min_numeric" => "min",
            "max_numeric" => "max",
            "multiple_of" => "value",
            "gt" | "gte" | "lt" | "lte" => "value",
            _ => unreachable!("unknown float-param validation rule"),
        };
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

    if name == "decimal" {
        if !(1..=2).contains(&args.len()) {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`decimal` requires 1 or 2 arguments (min, optional max)",
            ));
        }
        let min = &args[0];
        let max = args.get(1).unwrap_or(min);
        return Ok(generate_ts_rule(
            name,
            vec![
                ("min", quote!((#min) as usize)),
                ("max", quote!((#max) as usize)),
            ],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if DIGIT_PARAM_RULES.contains(&name) {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument", name),
            ));
        }
        let key = if name == "min_digits" { "min" } else { "max" };
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

    if name == "digits_between" {
        if args.len() != 2 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`digits_between` requires exactly 2 arguments (min, max)",
            ));
        }
        let min = &args[0];
        let max = &args[1];
        return Ok(generate_ts_rule(
            name,
            vec![
                ("min", quote!((#min) as usize)),
                ("max", quote!((#max) as usize)),
            ],
            quote!(Vec::new()),
            message,
            false,
            Vec::new(),
        ));
    }

    if COLLECTION_PARAM_RULES.contains(&name) {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument", name),
            ));
        }
        let key = if name == "min_items" { "min" } else { "max" };
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
            quote!(vec![#((#args).to_string()),*]),
            message,
            false,
            Vec::new(),
        ));
    }

    if name == "rule" {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`rule` requires exactly 1 argument (a rule name string or ValidationRuleId expression)",
            ));
        }
        let rule_name = match extract_string_literal(&args[0], "rule") {
            Ok(rule_name) => quote!(#rule_name),
            Err(_) => {
                let rule_id = &args[0];
                quote!((#rule_id).as_str())
            }
        };
        return Ok(generate_ts_rule_ext(
            quote!(#rule_name),
            vec![("rule", quote!(#rule_name))],
            quote!(Vec::new()),
            message,
            quote!(true),
            Vec::new(),
            quote!(None),
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
    generate_ts_rule_ext(
        quote!(#code),
        params,
        values,
        message,
        quote!(#server_only),
        nested_rules,
        quote!(None),
    )
}

fn generate_ts_rule_with_schema(
    code: &str,
    schema: TokenStream,
    message: &Option<String>,
) -> TokenStream {
    generate_ts_rule_ext(
        quote!(#code),
        Vec::new(),
        quote!(Vec::new()),
        message,
        quote!(false),
        Vec::new(),
        quote!(Some(#schema)),
    )
}

fn generate_ts_regex_rule(
    code: &str,
    pattern: &syn::Expr,
    message: &Option<String>,
) -> TokenStream {
    let message = match message {
        Some(message) => quote!(Some(#message.to_string())),
        None => quote!(None),
    };

    quote! {{
        let __foundry_pattern = (#pattern).to_string();
        let mut __params = ::std::collections::BTreeMap::new();
        __params.insert("pattern".to_string(), __foundry_pattern.clone());
        ::foundry::typescript::TsValidationRule {
            code: #code.to_string(),
            params: __params,
            values: Vec::new(),
            message: #message,
            server_only: !::foundry::typescript::rust_regex_is_client_compatible(&__foundry_pattern),
            rules: Vec::new(),
            schema: None,
        }
    }}
}

fn generate_ts_rule_ext(
    code: TokenStream,
    params: Vec<(&str, TokenStream)>,
    values: TokenStream,
    message: &Option<String>,
    server_only: TokenStream,
    nested_rules: Vec<TokenStream>,
    schema: TokenStream,
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
            code: (#code).to_string(),
            params: __params,
            values: #values,
            message: #message,
            server_only: #server_only,
            rules: vec![#(#nested_rules),*],
            schema: #schema,
        }
    }}
}

fn validation_hook_name(hook: &syn::Path) -> String {
    hook.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn generate_ts_nested_rule(nested_ty: Option<&Type>) -> syn::Result<TokenStream> {
    let nested_ty = nested_ty.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "`nested` validation requires a concrete child DTO type",
        )
    })?;
    Ok(generate_ts_rule_with_schema(
        "nested",
        quote!(<#nested_ty as ::foundry::typescript::TsValidationSchemaProvider>::ts_validation_schema()),
        &None,
    ))
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

fn is_nested_rule(rule: &RuleSpec) -> bool {
    matches!(rule, RuleSpec::Nested)
}

fn app_enum_rule_type_path(rules: &[RuleSpec]) -> Option<&syn::Path> {
    rules.iter().find_map(|rule| match rule {
        RuleSpec::AppEnum { type_path } => Some(type_path),
        _ => None,
    })
}

fn type_matches_app_enum_rule(ty: &Type, type_path: &syn::Path) -> bool {
    let ty = type_argument_if_last_segment_ident(ty, "Option").unwrap_or(ty);
    let Some(enum_ident) = type_path.segments.last().map(|segment| &segment.ident) else {
        return false;
    };
    let Type::Path(type_path) = ty else {
        return false;
    };

    type_path
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == *enum_ident)
}

fn vec_item_matches_app_enum_rule(ty: &Type, type_path: &syn::Path) -> bool {
    vec_item_type(ty).is_some_and(|item_ty| type_matches_app_enum_rule(item_ty, type_path))
}

fn app_enum_key_expr(type_path: &syn::Path, value_expr: TokenStream) -> TokenStream {
    quote! {
        match <#type_path as ::foundry::FoundryAppEnum>::key(#value_expr) {
            ::foundry::EnumKey::String(value) => value,
            ::foundry::EnumKey::Int(value) => value.to_string(),
        }
    }
}

fn rule_has_name(rule: &RuleSpec, expected: &str) -> bool {
    match rule {
        RuleSpec::Simple { name, .. } | RuleSpec::Parametric { name, .. } => name == expected,
        RuleSpec::Each { .. }
        | RuleSpec::AppEnum { .. }
        | RuleSpec::Image
        | RuleSpec::MaxFileSize(_)
        | RuleSpec::MaxDimensions(_, _)
        | RuleSpec::MinDimensions(_, _)
        | RuleSpec::AllowedMimes(_)
        | RuleSpec::AllowedExtensions(_) => false,
        RuleSpec::Nested => expected == "nested",
    }
}

fn is_required_presence_rule(rule: &RuleSpec) -> bool {
    rule_has_name(rule, "required")
        || CONDITIONAL_VALUE_RULES
            .iter()
            .any(|name| rule_has_name(rule, name))
        || CONDITIONAL_REQUIRED_FIELD_RULES
            .iter()
            .any(|name| rule_has_name(rule, name))
        || CONDITIONAL_REQUIRED_ALL_FIELD_RULES
            .iter()
            .any(|name| rule_has_name(rule, name))
}

fn is_collection_presence_rule(rule: &RuleSpec) -> bool {
    is_required_presence_rule(rule)
        || rule_has_name(rule, "nullable")
        || rule_has_name(rule, "bail")
        || rule_has_name(rule, "prohibited")
        || CONDITIONAL_PROHIBITED_VALUE_RULES
            .iter()
            .any(|name| rule_has_name(rule, name))
        || CONDITIONAL_PROHIBITED_FIELD_RULES
            .iter()
            .any(|name| rule_has_name(rule, name))
        || CONDITIONAL_PROHIBITED_ALL_FIELD_RULES
            .iter()
            .any(|name| rule_has_name(rule, name))
}

fn should_skip_absent_optional_collection(rules: &[RuleSpec]) -> bool {
    rules.iter().any(|rule| rule_has_name(rule, "nullable"))
        || should_auto_nullable(true, rules.iter())
}

fn is_collection_rule(rule: &RuleSpec) -> bool {
    match rule {
        RuleSpec::Simple { name, .. } => COLLECTION_SIMPLE_RULES.contains(&name.as_str()),
        RuleSpec::Parametric { name, .. } => COLLECTION_PARAM_RULES.contains(&name.as_str()),
        _ => false,
    }
}

fn is_map_rule(rule: &RuleSpec) -> bool {
    matches!(rule, RuleSpec::Parametric { name, .. } if MAP_PARAM_RULES.contains(&name.as_str()))
}

fn is_collection_rule_for_field(rule: &RuleSpec, is_vec: bool) -> bool {
    is_collection_rule(rule)
        || (is_vec && rule_has_name(rule, "filled"))
        || (is_vec && rule_has_name(rule, "size"))
        || (is_vec
            && COLLECTION_VALUE_RULES
                .iter()
                .any(|name| rule_has_name(rule, name)))
}

fn should_auto_nullable<'a>(
    is_option: bool,
    rules: impl IntoIterator<Item = &'a RuleSpec>,
) -> bool {
    if !is_option {
        return false;
    }

    let mut has_nullable = false;
    let mut has_required = false;
    for rule in rules {
        has_nullable = has_nullable || rule_has_name(rule, "nullable");
        has_required = has_required
            || rule_has_name(rule, "required")
            || rule_has_name(rule, "filled")
            || CONDITIONAL_VALUE_RULES
                .iter()
                .any(|name| rule_has_name(rule, name))
            || CONDITIONAL_REQUIRED_FIELD_RULES
                .iter()
                .any(|name| rule_has_name(rule, name))
            || CONDITIONAL_REQUIRED_ALL_FIELD_RULES
                .iter()
                .any(|name| rule_has_name(rule, name));
    }

    !has_nullable && !has_required
}

fn should_skip_absent_optional_map(rules: &[RuleSpec]) -> bool {
    rules.iter().any(|rule| rule_has_name(rule, "nullable"))
        || should_auto_nullable(true, rules.iter())
}

fn generate_validate_body(
    field_validations: &[FieldValidation],
    struct_ident: &Ident,
    all_field_names: &[String],
    all_fields: &[FieldInfo],
    field_wire_names: &[(String, String)],
) -> syn::Result<Vec<TokenStream>> {
    let mut stmts = Vec::new();

    for fv in field_validations {
        let field_ident = &fv.field_ident;
        let field_name = &fv.field_name;

        let file_rules: Vec<&RuleSpec> = fv.rules.iter().filter(|r| is_file_rule(r)).collect();
        let collection_rules: Vec<&RuleSpec> = fv
            .rules
            .iter()
            .filter(|r| is_collection_rule_for_field(r, fv.is_vec))
            .collect();
        let map_rules: Vec<&RuleSpec> = fv.rules.iter().filter(|r| is_map_rule(r)).collect();
        let text_rules: Vec<&RuleSpec> = fv
            .rules
            .iter()
            .filter(|r| {
                !is_file_rule(r)
                    && !is_nested_rule(r)
                    && !is_collection_rule_for_field(r, fv.is_vec)
                    && !is_map_rule(r)
                    && !matches!(r, RuleSpec::Each { .. })
            })
            .collect();
        let has_direct_nested = fv.rules.iter().any(is_nested_rule);

        // If there are file rules on a non-uploaded_file field, emit compile error
        if !file_rules.is_empty() && !fv.is_uploaded_file {
            return Err(syn::Error::new(
                field_ident.span(),
                "file validation rules (image, max_file_size, max_dimensions, min_dimensions, allowed_mimes, allowed_extensions) can only be used on UploadedFile, Option<UploadedFile>, Vec<UploadedFile>, or Option<Vec<UploadedFile>> fields",
            ));
        }

        if has_direct_nested && fv.is_vec {
            return Err(syn::Error::new(
                field_ident.span(),
                "`nested` on Vec<T> or Option<Vec<T>> fields must be declared as `each(nested)`",
            ));
        }

        let has_each = fv.rules.iter().any(|r| matches!(r, RuleSpec::Each { .. }));
        let each_rules: Vec<&RuleSpec> = fv
            .rules
            .iter()
            .filter(|r| matches!(r, RuleSpec::Each { .. }))
            .collect();

        if fv.is_vec && fv.is_uploaded_file {
            if has_each {
                return Err(syn::Error::new(
                    field_ident.span(),
                    "`each(...)` is not supported on UploadedFile collections; put collection rules and file rules directly on the field",
                ));
            }

            if text_rules
                .iter()
                .copied()
                .any(|rule| !is_collection_presence_rule(rule))
            {
                return Err(syn::Error::new(
                    field_ident.span(),
                    "Vec<UploadedFile> and Option<Vec<UploadedFile>> fields only support presence rules, collection size rules, and file validation rules",
                ));
            }

            if !text_rules.is_empty() {
                let rule_chain = generate_rule_chain_from_refs(
                    &text_rules,
                    struct_ident,
                    all_field_names,
                    all_fields,
                    field_wire_names,
                    field_ident,
                    false,
                )?;
                let presence_expr = if fv.is_option {
                    quote! {
                        self.#field_ident
                            .as_ref()
                            .map(|__files| if __files.is_empty() { "" } else { "1" })
                            .unwrap_or("")
                    }
                } else {
                    quote!(if self.#field_ident.is_empty() { "" } else { "1" })
                };
                let nullable_call =
                    if should_auto_nullable(fv.is_option, text_rules.iter().copied()) {
                        quote!(.nullable())
                    } else {
                        quote!()
                    };
                stmts.push(quote! {
                    validator.field(#field_name, #presence_expr)
                        #nullable_call
                        #rule_chain
                        .apply()
                        .await?;
                });
            }

            if !collection_rules.is_empty() {
                let collection_rule_stmts = generate_collection_length_rule_stmts(
                    &collection_rules,
                    quote!(__foundry_upload_files),
                    field_name,
                    "multi-file UploadedFile validation only supports collection presence and size rules: filled, min_items, max_items, and size",
                )?;
                let collection_items = if fv.is_option {
                    quote! {
                        let __foundry_upload_files = self.#field_ident.as_deref().unwrap_or(&[]);
                    }
                } else {
                    quote! {
                        let __foundry_upload_files = self.#field_ident.as_slice();
                    }
                };
                let collection_validation = quote! {
                    {
                        #collection_items
                        #(#collection_rule_stmts)*
                    }
                };
                if fv.is_option && should_skip_absent_optional_collection(&fv.rules) {
                    stmts.push(quote! {
                        if self.#field_ident.is_some() {
                            #collection_validation
                        }
                    });
                } else {
                    stmts.push(collection_validation);
                }
            }

            if !file_rules.is_empty() {
                let file_validation_code = generate_file_validation_code(fv, field_name)?;
                stmts.push(file_validation_code);
            }

            continue;
        }

        if !map_rules.is_empty() {
            if !fv.is_map && !fv.is_json_value {
                return Err(syn::Error::new(
                    field_ident.span(),
                    "map validation rules such as required_keys(...) can only be used on HashMap<String, _>, BTreeMap<String, _>, serde_json::Value, or Option wrappers around those types",
                ));
            }
            if has_each
                || !collection_rules.is_empty()
                || has_direct_nested
                || !file_rules.is_empty()
            {
                return Err(syn::Error::new(
                    field_ident.span(),
                    "required_keys(...) cannot be combined with collection, nested, or file validation rules",
                ));
            }

            let map_presence_rules: Vec<&RuleSpec> = text_rules
                .iter()
                .copied()
                .filter(|rule| is_collection_presence_rule(rule))
                .collect();
            if text_rules.len() != map_presence_rules.len() {
                return Err(syn::Error::new(
                    field_ident.span(),
                    "map validation rules can only be combined with field presence rules such as required, filled, nullable, bail, and prohibited variants",
                ));
            }

            if !map_presence_rules.is_empty() {
                let rule_chain = generate_rule_chain_from_refs(
                    &map_presence_rules,
                    struct_ident,
                    all_field_names,
                    all_fields,
                    field_wire_names,
                    field_ident,
                    false,
                )?;
                let presence_expr = if fv.is_map {
                    if fv.is_option {
                        quote! {
                            self.#field_ident
                                .as_ref()
                                .map(|__map| if __map.is_empty() { "" } else { "1" })
                                .unwrap_or("")
                        }
                    } else {
                        quote!(if self.#field_ident.is_empty() { "" } else { "1" })
                    }
                } else if fv.is_option {
                    quote! {
                        match self.#field_ident.as_ref() {
                            Some(::foundry::serde_json::Value::Object(__map)) if __map.is_empty() => "",
                            Some(::foundry::serde_json::Value::Null) | None => "",
                            Some(_) => "1",
                        }
                    }
                } else {
                    quote! {
                        match &self.#field_ident {
                            ::foundry::serde_json::Value::Object(__map) if __map.is_empty() => "",
                            ::foundry::serde_json::Value::Null => "",
                            _ => "1",
                        }
                    }
                };
                let nullable_call =
                    if should_auto_nullable(fv.is_option, map_presence_rules.iter().copied()) {
                        quote!(.nullable())
                    } else {
                        quote!()
                    };
                stmts.push(quote! {
                    validator.field(#field_name, #presence_expr)
                        #nullable_call
                        #rule_chain
                        .apply()
                        .await?;
                });
            }

            let map_rule_chain = generate_map_rule_chain_from_refs(&map_rules)?;
            let key_validation = if fv.is_map {
                if fv.is_option {
                    quote! {
                        let __foundry_map_keys = self.#field_ident.as_ref().map(|__map| {
                            __map.keys()
                                .map(::std::string::ToString::to_string)
                                .collect::<::std::vec::Vec<_>>()
                        });
                        validator.key_set(#field_name, __foundry_map_keys)
                            #map_rule_chain
                            .apply()
                            .await?;
                    }
                } else {
                    quote! {
                        let __foundry_map_keys = self.#field_ident
                            .keys()
                            .map(::std::string::ToString::to_string)
                            .collect::<::std::vec::Vec<_>>();
                        validator.key_set(#field_name, Some(__foundry_map_keys))
                            #map_rule_chain
                            .apply()
                            .await?;
                    }
                }
            } else if fv.is_option {
                quote! {
                    let __foundry_map_keys = self.#field_ident.as_ref().and_then(|__value| {
                        match __value {
                            ::foundry::serde_json::Value::Object(__map) => Some(
                                __map.keys()
                                    .map(::std::string::ToString::to_string)
                                    .collect::<::std::vec::Vec<_>>(),
                            ),
                            _ => None,
                        }
                    });
                    validator.key_set(#field_name, __foundry_map_keys)
                        #map_rule_chain
                        .apply()
                        .await?;
                }
            } else {
                quote! {
                    let __foundry_map_keys = match &self.#field_ident {
                        ::foundry::serde_json::Value::Object(__map) => Some(
                            __map.keys()
                                .map(::std::string::ToString::to_string)
                                .collect::<::std::vec::Vec<_>>(),
                        ),
                        _ => None,
                    };
                    validator.key_set(#field_name, __foundry_map_keys)
                        #map_rule_chain
                        .apply()
                        .await?;
                }
            };
            if fv.is_option && should_skip_absent_optional_map(&fv.rules) {
                stmts.push(quote! {
                    if self.#field_ident.is_some() {
                        #key_validation
                    }
                });
            } else {
                stmts.push(quote! {
                    #key_validation
                });
            }

            continue;
        }

        if (has_each || !collection_rules.is_empty()) && !fv.is_vec {
            return Err(syn::Error::new(
                field_ident.span(),
                "collection validation rules (each, min_items, max_items, distinct) can only be used on Vec<T> or Option<Vec<T>> fields",
            ));
        }

        if has_each || !collection_rules.is_empty() {
            if each_rules.len() > 1 {
                return Err(syn::Error::new(
                    field_ident.span(),
                    "only one `each(...)` rule is allowed per field",
                ));
            }

            let collection_presence_rules: Vec<&RuleSpec> = text_rules
                .iter()
                .copied()
                .filter(|rule| is_collection_presence_rule(rule))
                .collect();
            if fv.is_option && !collection_presence_rules.is_empty() {
                let rule_chain = generate_rule_chain_from_refs(
                    &collection_presence_rules,
                    struct_ident,
                    all_field_names,
                    all_fields,
                    field_wire_names,
                    field_ident,
                    false,
                )?;
                let presence_expr = quote! {
                    self.#field_ident
                        .as_ref()
                        .map(|__items| if __items.is_empty() { "" } else { "1" })
                        .unwrap_or("")
                };
                stmts.push(quote! {
                    validator.field(#field_name, #presence_expr)
                        #rule_chain
                        .apply()
                        .await?;
                });
            }

            let empty_item_rules = Vec::new();
            let item_rules = if has_each {
                let RuleSpec::Each { rules } = each_rules[0] else {
                    unreachable!()
                };
                rules
            } else {
                &empty_item_rules
            };
            let item_has_nested = item_rules.iter().any(is_nested_rule);

            if item_has_nested {
                if item_rules.len() != 1 {
                    return Err(syn::Error::new(
                        field_ident.span(),
                        "`each(nested)` cannot be combined with scalar item rules",
                    ));
                }

                let collection_rule_stmts = generate_nested_collection_rule_stmts(
                    &collection_rules,
                    quote!(__foundry_collection_items),
                    field_name,
                )?;
                let collection_items = if fv.is_option {
                    quote! {
                        let __foundry_collection_items = self.#field_ident.as_deref().unwrap_or(&[]);
                    }
                } else {
                    quote! {
                        let __foundry_collection_items = self.#field_ident.as_slice();
                    }
                };
                let collection_validation = quote! {
                    #collection_items
                    #(#collection_rule_stmts)*
                    validator.each_nested(#field_name, __foundry_collection_items).await?;
                };
                if fv.is_option && should_skip_absent_optional_collection(&fv.rules) {
                    stmts.push(quote! {
                        if self.#field_ident.is_some() {
                            #collection_validation
                        }
                    });
                } else {
                    stmts.push(quote! {
                        #collection_validation
                    });
                }
                continue;
            }

            let collection_rule_chain = generate_collection_rule_chain_from_refs(
                &collection_rules,
                struct_ident,
                all_field_names,
                all_fields,
                field_wire_names,
                field_ident,
            )?;
            let item_rule_chain = generate_rule_chain(
                item_rules,
                struct_ident,
                all_field_names,
                all_fields,
                field_wire_names,
                field_ident,
                fv.is_vec_numeric && app_enum_rule_type_path(item_rules).is_none(),
            )?;

            if let Some(type_path) = app_enum_rule_type_path(item_rules) {
                let values_ident = format_ident!("__foundry_validate_{}_values", field_ident);
                let collection_items = if fv.is_option {
                    quote! {
                        let __foundry_collection_items = self.#field_ident.as_deref().unwrap_or(&[]);
                    }
                } else {
                    quote! {
                        let __foundry_collection_items = self.#field_ident.as_slice();
                    }
                };
                let collection_validation = if vec_item_matches_app_enum_rule(&fv.ty, type_path) {
                    let key_expr = app_enum_key_expr(type_path, quote!(__value.clone()));
                    quote! {
                        #collection_items
                        let #values_ident: ::std::vec::Vec<::std::string::String> = __foundry_collection_items
                            .iter()
                            .map(|__value| #key_expr)
                            .collect();
                        validator.each(#field_name, &#values_ident)
                            #collection_rule_chain
                            #item_rule_chain
                            .apply()
                            .await?;
                    }
                } else {
                    quote! {
                        #collection_items
                        validator.each(#field_name, __foundry_collection_items)
                            #collection_rule_chain
                            #item_rule_chain
                            .apply()
                            .await?;
                    }
                };
                if fv.is_option && should_skip_absent_optional_collection(&fv.rules) {
                    stmts.push(quote! {
                        if self.#field_ident.is_some() {
                            #collection_validation
                        }
                    });
                } else {
                    stmts.push(quote! {
                        #collection_validation
                    });
                }
            } else {
                let collection_items = if fv.is_option {
                    quote! {
                        let __foundry_collection_items = self.#field_ident.as_deref().unwrap_or(&[]);
                    }
                } else {
                    quote! {
                        let __foundry_collection_items = self.#field_ident.as_slice();
                    }
                };
                let collection_validation = quote! {
                    #collection_items
                    validator.each(#field_name, __foundry_collection_items)
                        #collection_rule_chain
                        #item_rule_chain
                        .apply()
                        .await?;
                };
                if fv.is_option && should_skip_absent_optional_collection(&fv.rules) {
                    stmts.push(quote! {
                        if self.#field_ident.is_some() {
                            #collection_validation
                        }
                    });
                } else {
                    stmts.push(quote! {
                        #collection_validation
                    });
                }
            }
        } else if has_direct_nested {
            if !file_rules.is_empty() || !collection_rules.is_empty() || has_each {
                return Err(syn::Error::new(
                    field_ident.span(),
                    "`nested` can only be combined with field presence rules; use `each(nested)` for Vec<T> or Option<Vec<T>> fields",
                ));
            }

            if !text_rules.is_empty() {
                let presence_expr = if fv.is_option {
                    quote!(if self.#field_ident.is_some() { "1" } else { "" })
                } else {
                    quote!("1")
                };
                let rule_chain = generate_rule_chain_from_refs(
                    &text_rules,
                    struct_ident,
                    all_field_names,
                    all_fields,
                    field_wire_names,
                    field_ident,
                    false,
                )?;
                let nullable_call =
                    if should_auto_nullable(fv.is_option, text_rules.iter().copied()) {
                        quote!(.nullable())
                    } else {
                        quote!()
                    };
                stmts.push(quote! {
                    validator.field(#field_name, #presence_expr)
                        #nullable_call
                        #rule_chain
                        .apply()
                        .await?;
                });
            }

            let nested_stmt = if fv.is_option {
                quote! {
                    if let Some(__foundry_nested_value) = self.#field_ident.as_ref() {
                        validator.nested(#field_name, __foundry_nested_value).await?;
                    }
                }
            } else {
                quote! {
                    validator.nested(#field_name, &self.#field_ident).await?;
                }
            };
            stmts.push(nested_stmt);
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
                    all_fields,
                    field_wire_names,
                    field_ident,
                    false,
                )?;

                let nullable_call =
                    if should_auto_nullable(fv.is_option, text_rules.iter().copied()) {
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
            let value_expr = if let Some(type_path) = app_enum_rule_type_path(&fv.rules) {
                if type_matches_app_enum_rule(&fv.ty, type_path) && fv.is_option {
                    let key_expr = app_enum_key_expr(type_path, quote!(value.clone()));
                    quote! {
                        self.#field_ident
                            .as_ref()
                            .map(|value| #key_expr)
                            .unwrap_or_default()
                    }
                } else if type_matches_app_enum_rule(&fv.ty, type_path) {
                    app_enum_key_expr(type_path, quote!(self.#field_ident.clone()))
                } else if fv.is_option {
                    quote!(self.#field_ident.as_ref().map(::std::string::ToString::to_string).unwrap_or_default())
                } else {
                    quote!(&self.#field_ident)
                }
            } else if fv.is_option {
                quote!(self.#field_ident.as_ref().map(::std::string::ToString::to_string).unwrap_or_default())
            } else {
                quote!(&self.#field_ident)
            };

            let rule_chain = generate_rule_chain(
                &fv.rules,
                struct_ident,
                all_field_names,
                all_fields,
                field_wire_names,
                field_ident,
                fv.is_numeric && app_enum_rule_type_path(&fv.rules).is_none(),
            )?;

            let nullable_call = if should_auto_nullable(fv.is_option, &fv.rules) {
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
    let rule_stmts = generate_file_rule_stmts(&file_rules, quote!(#field_name_lit))?;

    if fv.is_vec {
        let indexed_rule_stmts =
            generate_file_rule_stmts(&file_rules, quote!(__foundry_file_field.as_str()))?;
        let collection_items = if fv.is_option {
            quote! {
                let __foundry_upload_files = self.#field_ident.as_deref().unwrap_or(&[]);
            }
        } else {
            quote! {
                let __foundry_upload_files = self.#field_ident.as_slice();
            }
        };
        let validation = quote! {
            {
                #collection_items
                for (__foundry_file_index, __file) in __foundry_upload_files.iter().enumerate() {
                    let __foundry_file_field = ::std::format!(
                        "{}[{}]",
                        #field_name_lit,
                        __foundry_file_index
                    );
                    #(#indexed_rule_stmts)*
                }
            }
        };
        if fv.is_option && should_skip_absent_optional_collection(&fv.rules) {
            Ok(quote! {
                if self.#field_ident.is_some() {
                    #validation
                }
            })
        } else {
            Ok(validation)
        }
    } else if fv.is_option {
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

fn generate_file_rule_stmts(
    file_rules: &[&RuleSpec],
    field_name_expr: TokenStream,
) -> syn::Result<Vec<TokenStream>> {
    let mut rule_stmts = Vec::new();

    for rule in file_rules {
        let error_field = field_name_expr.clone();
        let stmt = match rule {
            RuleSpec::Image => {
                quote!({
                    let __is_img = ::foundry::validation::file_rules::is_image(__file).await
                        .map_err(|e| ::foundry::foundation::Error::message(e.to_string()))?;
                    if !__is_img {
                        validator.add_error(#error_field, "image", &[]);
                    }
                })
            }
            RuleSpec::MaxFileSize(kb_expr) => {
                quote!({
                    let __max_kb: u64 = (#kb_expr) as u64;
                    if !::foundry::validation::file_rules::check_max_size(__file, __max_kb) {
                        validator.add_error(#error_field, "max_file_size",
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
                        validator.add_error(#error_field, "max_dimensions",
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
                        validator.add_error(#error_field, "min_dimensions",
                            &[("width", &__min_w.to_string()), ("height", &__min_h.to_string())]);
                    }
                })
            }
            RuleSpec::AllowedMimes(exprs) => {
                quote!({
                    let __allowed_mimes: Vec<String> = vec![#((#exprs).to_string()),*];
                    let __is_allowed = ::foundry::validation::file_rules::check_allowed_mimes(__file, &__allowed_mimes).await
                        .map_err(|e| ::foundry::foundation::Error::message(e.to_string()))?;
                    if !__is_allowed {
                        validator.add_error(#error_field, "allowed_mimes",
                            &[("mimes", &__allowed_mimes.join(", "))]);
                    }
                })
            }
            RuleSpec::AllowedExtensions(exprs) => {
                quote!({
                    let __allowed_exts: Vec<String> = vec![#((#exprs).to_string()),*];
                    if !::foundry::validation::file_rules::check_allowed_extensions(__file, &__allowed_exts) {
                        validator.add_error(#error_field, "allowed_extensions",
                            &[("extensions", &__allowed_exts.join(", "))]);
                    }
                })
            }
            _ => unreachable!(),
        };
        rule_stmts.push(stmt);
    }

    Ok(rule_stmts)
}

fn generate_nested_collection_rule_stmts(
    rules: &[&RuleSpec],
    items_expr: TokenStream,
    field_name: &str,
) -> syn::Result<Vec<TokenStream>> {
    generate_collection_length_rule_stmts(
        rules,
        items_expr,
        field_name,
        "`each(nested)` only supports collection presence and size rules: filled, min_items, max_items, and size",
    )
}

fn generate_collection_length_rule_stmts(
    rules: &[&RuleSpec],
    items_expr: TokenStream,
    field_name: &str,
    unsupported_message: &str,
) -> syn::Result<Vec<TokenStream>> {
    let mut stmts = Vec::new();

    for rule in rules {
        match rule {
            RuleSpec::Simple { name, message } if name == "filled" => {
                let message = match message {
                    Some(message) => quote!(Some(#message)),
                    None => quote!(None),
                };
                stmts.push(quote! {
                    if #items_expr.is_empty() {
                        validator.add_error_with_message(#field_name, "filled", &[], #message);
                    }
                });
            }
            RuleSpec::Parametric {
                name,
                args,
                message,
            } if name == "min_items" => {
                let min = args.first().ok_or_else(|| {
                    syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "`min_items` requires exactly 1 argument",
                    )
                })?;
                let message = match message {
                    Some(message) => quote!(Some(#message)),
                    None => quote!(None),
                };
                stmts.push(quote! {
                    {
                        let __foundry_min = (#min) as usize;
                        if #items_expr.len() < __foundry_min {
                            validator.add_error_with_message(
                                #field_name,
                                "min_items",
                                &[("min", &__foundry_min.to_string())],
                                #message,
                            );
                        }
                    }
                });
            }
            RuleSpec::Parametric {
                name,
                args,
                message,
            } if name == "max_items" => {
                let max = args.first().ok_or_else(|| {
                    syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "`max_items` requires exactly 1 argument",
                    )
                })?;
                let message = match message {
                    Some(message) => quote!(Some(#message)),
                    None => quote!(None),
                };
                stmts.push(quote! {
                    {
                        let __foundry_max = (#max) as usize;
                        if #items_expr.len() > __foundry_max {
                            validator.add_error_with_message(
                                #field_name,
                                "max_items",
                                &[("max", &__foundry_max.to_string())],
                                #message,
                            );
                        }
                    }
                });
            }
            RuleSpec::Parametric {
                name,
                args,
                message,
            } if name == "size" => {
                let size = args.first().ok_or_else(|| {
                    syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "`size` requires exactly 1 argument",
                    )
                })?;
                let message = match message {
                    Some(message) => quote!(Some(#message)),
                    None => quote!(None),
                };
                stmts.push(quote! {
                    {
                        let __foundry_size = (#size) as usize;
                        if #items_expr.len() != __foundry_size {
                            validator.add_error_with_message(
                                #field_name,
                                "size",
                                &[("size", &__foundry_size.to_string())],
                                #message,
                            );
                        }
                    }
                });
            }
            _ => {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    unsupported_message,
                ));
            }
        }
    }

    Ok(stmts)
}

fn generate_rule_chain_from_refs(
    rules: &[&RuleSpec],
    struct_ident: &Ident,
    all_field_names: &[String],
    all_fields: &[FieldInfo],
    field_wire_names: &[(String, String)],
    field_ident: &Ident,
    is_numeric_value: bool,
) -> syn::Result<TokenStream> {
    let owned: Vec<RuleSpec> = rules.iter().cloned().cloned().collect();
    generate_rule_chain(
        &owned,
        struct_ident,
        all_field_names,
        all_fields,
        field_wire_names,
        field_ident,
        is_numeric_value,
    )
}

fn generate_collection_rule_chain_from_refs(
    rules: &[&RuleSpec],
    struct_ident: &Ident,
    all_field_names: &[String],
    all_fields: &[FieldInfo],
    field_wire_names: &[(String, String)],
    field_ident: &Ident,
) -> syn::Result<TokenStream> {
    let mut tokens = Vec::new();

    for rule in rules {
        match rule {
            RuleSpec::Simple { name, message } if name == "filled" => {
                let with_msg = generate_with_message(message);
                tokens.push(quote!(.filled_collection() #with_msg));
            }
            RuleSpec::Parametric {
                name,
                args,
                message,
            } if name == "size" => {
                if args.len() != 1 {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "`size` requires exactly 1 argument",
                    ));
                }
                let size = &args[0];
                let with_msg = generate_with_message(message);
                tokens.push(quote!(.size_items(#size as usize) #with_msg));
            }
            RuleSpec::Parametric {
                name,
                args,
                message,
            } if COLLECTION_VALUE_RULES.contains(&name.as_str()) => {
                if args.is_empty() {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!("`{}` requires at least 1 argument", name),
                    ));
                }
                let method = match name.as_str() {
                    "contains" => syn::Ident::new("contains_all", proc_macro2::Span::call_site()),
                    "doesnt_contain" => {
                        syn::Ident::new("doesnt_contain_any", proc_macro2::Span::call_site())
                    }
                    _ => unreachable!("unknown collection value validation rule"),
                };
                let with_msg = generate_with_message(message);
                tokens.push(quote!(.#method(vec![#(#args),*]) #with_msg));
            }
            _ => {
                let owned = (**rule).clone();
                tokens.push(generate_rule_chain(
                    &[owned],
                    struct_ident,
                    all_field_names,
                    all_fields,
                    field_wire_names,
                    field_ident,
                    false,
                )?);
            }
        }
    }

    Ok(quote! {
        #(#tokens)*
    })
}

fn generate_map_rule_chain_from_refs(rules: &[&RuleSpec]) -> syn::Result<TokenStream> {
    let mut tokens = Vec::new();

    for rule in rules {
        match rule {
            RuleSpec::Parametric {
                name,
                args,
                message,
            } if name == "required_keys" => {
                if args.is_empty() {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        "`required_keys` requires at least 1 argument",
                    ));
                }
                let with_msg = generate_with_message(message);
                tokens.push(quote!(.required_keys(vec![#(#args),*]) #with_msg));
            }
            _ => {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "map validation only supports required_keys(...)",
                ));
            }
        }
    }

    Ok(quote! {
        #(#tokens)*
    })
}

fn generate_rule_chain(
    rules: &[RuleSpec],
    struct_ident: &Ident,
    all_field_names: &[String],
    all_fields: &[FieldInfo],
    field_wire_names: &[(String, String)],
    field_ident: &Ident,
    is_numeric_value: bool,
) -> syn::Result<TokenStream> {
    let mut tokens = Vec::new();
    let context = RuleGenerationContext {
        struct_ident,
        all_field_names,
        all_fields,
        field_wire_names,
    };

    for rule in rules {
        match rule {
            RuleSpec::Each { .. } => {
                // Handled at a higher level; skip
            }
            RuleSpec::Nested => {
                // Handled at a higher level; skip
            }
            RuleSpec::AppEnum { type_path } => {
                tokens.push(quote!(.app_enum::<#type_path>()));
            }
            RuleSpec::Simple { name, message } if name == "confirmed" => {
                let call = generate_default_confirmed_rule_call(
                    message,
                    struct_ident,
                    all_field_names,
                    all_fields,
                    field_wire_names,
                    field_ident,
                )?;
                tokens.push(call);
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
                    &context,
                    field_ident,
                    is_numeric_value,
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
        "required" | "filled" | "email" | "numeric" | "boolean" | "accepted" | "declined"
        | "prohibited" | "alpha" | "alpha_dash" | "alpha_num" | "alpha_numeric" | "ascii"
        | "lowercase" | "uppercase" | "digits" | "url" | "uuid" | "ulid" | "hex_color"
        | "mac_address" | "json" | "timezone" | "ip" | "ipv4" | "ipv6" | "date" | "time"
        | "datetime" | "local_datetime" | "integer" | "nullable" | "bail" | "distinct" => {
            let method = syn::Ident::new(name, proc_macro2::Span::call_site());
            Ok(quote!(#method()))
        }
        _ => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("unknown validation rule `{}`", name),
        )),
    }
}

fn default_confirmation_field_name(field_name: &str) -> String {
    let field_name = field_name.strip_prefix("r#").unwrap_or(field_name);
    format!("{field_name}_confirmation")
}

fn find_field_info<'a>(all_fields: &'a [FieldInfo], rust_name: &str) -> Option<&'a FieldInfo> {
    all_fields.iter().find(|field| field.rust_name == rust_name)
}

fn referenced_field_value_expr(
    all_fields: &[FieldInfo],
    rust_name: &str,
    span: proc_macro2::Span,
    struct_ident: &Ident,
) -> syn::Result<TokenStream> {
    let field = find_field_info(all_fields, rust_name).ok_or_else(|| {
        syn::Error::new(
            span,
            format!(
                "field `{}` referenced in validation rule does not exist on struct `{}`",
                rust_name, struct_ident
            ),
        )
    })?;
    Ok(field_value_expr_for_reference(field))
}

fn field_value_expr_for_reference(field: &FieldInfo) -> TokenStream {
    let ident = &field.ident;

    if field.is_vec || field.is_vec_uploaded_file {
        if field.is_option {
            return quote! {
                self.#ident
                    .as_ref()
                    .map(|__items| if __items.is_empty() { "" } else { "1" })
                    .unwrap_or("")
                    .to_string()
            };
        }

        return quote!((if self.#ident.is_empty() { "" } else { "1" }).to_string());
    }

    if field.is_map {
        if field.is_option {
            return quote! {
                self.#ident
                    .as_ref()
                    .map(|__map| if __map.is_empty() { "" } else { "1" })
                    .unwrap_or("")
                    .to_string()
            };
        }

        return quote!((if self.#ident.is_empty() { "" } else { "1" }).to_string());
    }

    if field.is_json_value {
        if field.is_option {
            return quote! {
                match self.#ident.as_ref() {
                    Some(::foundry::serde_json::Value::Object(__map)) if __map.is_empty() => "",
                    Some(::foundry::serde_json::Value::Null) | None => "",
                    Some(_) => "1",
                }
                .to_string()
            };
        }

        return quote! {
            match &self.#ident {
                ::foundry::serde_json::Value::Object(__map) if __map.is_empty() => "",
                ::foundry::serde_json::Value::Null => "",
                _ => "1",
            }
            .to_string()
        };
    }

    if field.is_uploaded_file {
        if field.is_option {
            return quote! {
                self.#ident
                    .as_ref()
                    .and_then(|__file| __file.original_name.as_deref())
                    .unwrap_or("")
                    .to_string()
            };
        }

        return quote!(self.#ident.original_name.as_deref().unwrap_or("").to_string());
    }

    if field.is_nested {
        if field.is_option {
            return quote!((if self.#ident.is_some() { "1" } else { "" }).to_string());
        }

        return quote!("1".to_string());
    }

    if field.is_option {
        return quote! {
            self.#ident
                .as_ref()
                .map(::std::string::ToString::to_string)
                .unwrap_or_default()
        };
    }

    quote!(::std::string::ToString::to_string(&self.#ident))
}

fn generate_default_confirmed_rule_call(
    message: &Option<String>,
    struct_ident: &Ident,
    all_field_names: &[String],
    all_fields: &[FieldInfo],
    field_wire_names: &[(String, String)],
    field_ident: &Ident,
) -> syn::Result<TokenStream> {
    let field_name = rust_ident_name(field_ident);
    let other_field_name = default_confirmation_field_name(&field_name);
    if !all_field_names.contains(&other_field_name) {
        return Err(syn::Error::new(
            field_ident.span(),
            format!(
                "`confirmed` without arguments expects field `{}` on struct `{}`; pass `confirmed(\"field\")` to use a custom confirmation field",
                other_field_name, struct_ident
            ),
        ));
    }

    let other_wire_field_name = wire_field_name(&other_field_name, field_wire_names);
    let other_value_expr = referenced_field_value_expr(
        all_fields,
        &other_field_name,
        field_ident.span(),
        struct_ident,
    )?;
    let with_msg = generate_with_message(message);
    Ok(quote!(.confirmed(#other_wire_field_name, #other_value_expr) #with_msg))
}

fn generate_parametric_rule_call(
    name: &str,
    args: &[syn::Expr],
    message: &Option<String>,
    context: &RuleGenerationContext<'_>,
    _field_ident: &Ident,
    is_numeric_value: bool,
) -> syn::Result<TokenStream> {
    let with_msg = generate_with_message(message);
    let struct_ident = context.struct_ident;
    let all_field_names = context.all_field_names;
    let field_wire_names = context.field_wire_names;

    // Conditional value rules: required_if("field", "value"), accepted_if("field", "value"), etc.
    if CONDITIONAL_VALUE_RULES.contains(&name) {
        if args.len() != 2 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 2 arguments (field, value)", name),
            ));
        }
        let other_field_name = extract_string_literal(&args[0], name)?;

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
        let expected = &args[1];
        let other_wire_field_name = wire_field_name(&other_field_name, field_wire_names);
        let other_value_expr = referenced_field_value_expr(
            context.all_fields,
            &other_field_name,
            args[0].span(),
            struct_ident,
        )?;
        return Ok(
            quote!(.#method(#other_wire_field_name, #other_value_expr, #expected) #with_msg),
        );
    }

    if CONDITIONAL_PROHIBITED_VALUE_RULES.contains(&name) {
        if args.len() != 2 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 2 arguments (field, value)", name),
            ));
        }
        let other_field_name = extract_string_literal(&args[0], name)?;

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
        let expected = &args[1];
        let other_wire_field_name = wire_field_name(&other_field_name, field_wire_names);
        let other_value_expr = referenced_field_value_expr(
            context.all_fields,
            &other_field_name,
            args[0].span(),
            struct_ident,
        )?;
        return Ok(
            quote!(.#method(#other_wire_field_name, #other_value_expr, #expected) #with_msg),
        );
    }

    // Conditional presence rules: required_with("field"), required_without("field")
    if CONDITIONAL_REQUIRED_FIELD_RULES.contains(&name) {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument (field)", name),
            ));
        }
        let other_field_name = extract_string_literal(&args[0], name)?;

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
        let other_wire_field_name = wire_field_name(&other_field_name, field_wire_names);
        let other_value_expr = referenced_field_value_expr(
            context.all_fields,
            &other_field_name,
            args[0].span(),
            struct_ident,
        )?;
        return Ok(quote!(.#method(#other_wire_field_name, #other_value_expr) #with_msg));
    }

    if CONDITIONAL_PROHIBITED_FIELD_RULES.contains(&name) {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument (field)", name),
            ));
        }
        let other_field_name = extract_string_literal(&args[0], name)?;

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
        let other_wire_field_name = wire_field_name(&other_field_name, field_wire_names);
        let other_value_expr = referenced_field_value_expr(
            context.all_fields,
            &other_field_name,
            args[0].span(),
            struct_ident,
        )?;
        return Ok(quote!(.#method(#other_wire_field_name, #other_value_expr) #with_msg));
    }

    // Conditional presence rules: required_with_all("field", ...), required_without_all("field", ...)
    if CONDITIONAL_REQUIRED_ALL_FIELD_RULES.contains(&name) {
        if args.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires at least 1 argument (field)", name),
            ));
        }

        let mut field_pairs = Vec::new();
        for arg in args {
            let other_field_name = extract_string_literal(arg, name)?;

            if !all_field_names.contains(&other_field_name) {
                return Err(syn::Error::new(
                    arg.span(),
                    format!(
                        "field `{}` referenced in `{}` does not exist on struct `{}`",
                        other_field_name, name, struct_ident
                    ),
                ));
            }

            let other_wire_field_name = wire_field_name(&other_field_name, field_wire_names);
            let other_value_expr = referenced_field_value_expr(
                context.all_fields,
                &other_field_name,
                arg.span(),
                struct_ident,
            )?;
            field_pairs.push(quote! {
                (
                    #other_wire_field_name.to_string(),
                    #other_value_expr,
                )
            });
        }

        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        return Ok(quote!(.#method(vec![#(#field_pairs),*]) #with_msg));
    }

    if CONDITIONAL_PROHIBITED_ALL_FIELD_RULES.contains(&name) {
        if args.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires at least 1 argument (field)", name),
            ));
        }

        let mut field_pairs = Vec::new();
        for arg in args {
            let other_field_name = extract_string_literal(arg, name)?;

            if !all_field_names.contains(&other_field_name) {
                return Err(syn::Error::new(
                    arg.span(),
                    format!(
                        "field `{}` referenced in `{}` does not exist on struct `{}`",
                        other_field_name, name, struct_ident
                    ),
                ));
            }

            let other_wire_field_name = wire_field_name(&other_field_name, field_wire_names);
            let other_value_expr = referenced_field_value_expr(
                context.all_fields,
                &other_field_name,
                arg.span(),
                struct_ident,
            )?;
            field_pairs.push(quote! {
                (
                    #other_wire_field_name.to_string(),
                    #other_value_expr,
                )
            });
        }

        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        return Ok(quote!(.#method(vec![#(#field_pairs),*]) #with_msg));
    }

    // Cross-field rules: confirmed("field"), same("field"), etc.
    if CROSS_FIELD_RULES.contains(&name) {
        if name == "confirmed" && args.is_empty() {
            return generate_default_confirmed_rule_call(
                message,
                struct_ident,
                all_field_names,
                context.all_fields,
                field_wire_names,
                _field_ident,
            );
        }

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
        let other_wire_field_name = wire_field_name(&other_field_name, field_wire_names);
        let other_value_expr = referenced_field_value_expr(
            context.all_fields,
            &other_field_name,
            args[0].span(),
            struct_ident,
        )?;
        return Ok(quote!(.#method(#other_wire_field_name, #other_value_expr) #with_msg));
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

    // String-param rules: regex/not_regex plus literal prefix, suffix, and substring checks.
    if MULTI_STRING_PARAM_RULES.contains(&name) {
        if args.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires at least 1 argument", name),
            ));
        }
        let method = syn::Ident::new(&format!("{name}_any"), proc_macro2::Span::call_site());
        return Ok(quote!(.#method(vec![#(#args),*]) #with_msg));
    }

    // String-param rules: regex/not_regex plus substring checks.
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

    // Version-constrained UUID rule: uuid(4), matching Laravel's uuid:4 shape.
    if name == "uuid" {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`uuid` requires exactly 1 argument when constraining the UUID version",
            ));
        }
        let version = &args[0];
        return Ok(quote!(.uuid_version(#version as u8) #with_msg));
    }

    if name == "size" {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`size` requires exactly 1 argument",
            ));
        }
        let arg0 = &args[0];
        if is_numeric_value {
            return Ok(quote!(.size_numeric(#arg0 as f64) #with_msg));
        }
        return Ok(quote!(.size(#arg0 as usize) #with_msg));
    }

    // String length rules: min(N)/min_length(N), max(N)/max_length(N)
    if matches!(name, "min" | "max" | "min_length" | "max_length") {
        if args.len() != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`{}` requires exactly 1 argument", name),
            ));
        }
        let method_name = if matches!(name, "min" | "min_length") {
            "min"
        } else {
            "max"
        };
        let method = syn::Ident::new(method_name, proc_macro2::Span::call_site());
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

    // decimal(M) or decimal(M, N)
    if name == "decimal" {
        if !(1..=2).contains(&args.len()) {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`decimal` requires 1 or 2 arguments (min, optional max)",
            ));
        }
        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        let min = &args[0];
        let max = args.get(1).unwrap_or(min);
        return Ok(quote!(.#method(#min as usize, #max as usize) #with_msg));
    }

    // Digit count rules: min_digits(N), max_digits(N)
    if DIGIT_PARAM_RULES.contains(&name) {
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

    // digits_between(M, N)
    if name == "digits_between" {
        if args.len() != 2 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`digits_between` requires exactly 2 arguments (min, max)",
            ));
        }
        let method = syn::Ident::new(name, proc_macro2::Span::call_site());
        let min = &args[0];
        let max = &args[1];
        return Ok(quote!(.#method(#min as usize, #max as usize) #with_msg));
    }

    // Collection size rules: min_items(N), max_items(N)
    if COLLECTION_PARAM_RULES.contains(&name) {
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
                "`rule` requires exactly 1 argument (a rule name string or ValidationRuleId expression)",
            ));
        }
        let rule_expr = match extract_string_literal(&args[0], "rule") {
            Ok(rule_name) => {
                let rule_name_lit = syn::LitStr::new(&rule_name, args[0].span());
                quote!(::foundry::support::ValidationRuleId::new(#rule_name_lit))
            }
            Err(_) => {
                let rule_id = &args[0];
                quote!(#rule_id)
            }
        };
        return Ok(quote!(.rule(#rule_expr) #with_msg));
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

fn generate_messages_body(
    messages: &[ValidationMessageArg],
    field_wire_names: &[(String, String)],
) -> TokenStream {
    if messages.is_empty() {
        return quote!(Vec::new());
    }

    let entries = messages.iter().map(|message| {
        let field = wire_field_name(&message.field, field_wire_names);
        let code = &message.rule;
        let msg = &message.message;
        quote!((#field.to_string(), #code.to_string(), #msg.to_string()))
    });

    quote! {
        vec![
            #(#entries),*
        ]
    }
}

fn generate_attributes_body(
    attributes: &[ValidationAttributeArg],
    field_wire_names: &[(String, String)],
) -> TokenStream {
    if attributes.is_empty() {
        return quote!(Vec::new());
    }

    let entries = attributes.iter().map(|attribute| {
        let field = wire_field_name(&attribute.field, field_wire_names);
        let name = &attribute.name;
        quote!((#field.to_string(), #name.to_string()))
    });

    quote! {
        vec![
            #(#entries),*
        ]
    }
}

// ---------------------------------------------------------------------------
// FromMultipart implementation generation
// ---------------------------------------------------------------------------

fn missing_multipart_field_expr(
    fi: &FieldInfo,
    struct_default: &SerdeDefault,
) -> Option<TokenStream> {
    if let Some(expr) = fi.default.field_expr(&fi.ty) {
        return Some(expr);
    }

    if struct_default.is_some() {
        let ident = &fi.ident;
        return Some(quote!(__foundry_default.#ident));
    }

    None
}

fn skipped_multipart_field_expr(fi: &FieldInfo, struct_default: &SerdeDefault) -> TokenStream {
    missing_multipart_field_expr(fi, struct_default).unwrap_or_else(|| {
        let ty = &fi.ty;
        quote!(<#ty as ::std::default::Default>::default())
    })
}

fn generate_from_multipart_impl(
    struct_ident: &Ident,
    all_fields: &[FieldInfo],
    struct_default: &SerdeDefault,
) -> syn::Result<TokenStream> {
    // Declare Option<T> vars for each field
    let mut var_decls = Vec::new();
    let mut match_arms = Vec::new();
    let mut field_assignments = Vec::new();
    let mut cleanup_uploads = Vec::new();
    let struct_default_decl = struct_default
        .struct_expr()
        .map(|expr| quote!(let __foundry_default = #expr;))
        .unwrap_or_default();

    for fi in all_fields {
        let ident = &fi.ident;
        let name = &fi.name;
        let var_name = format_ident!("__val_{}", ident);
        let missing_default = missing_multipart_field_expr(fi, struct_default);

        if fi.skips_deserializing {
            let skipped_default = skipped_multipart_field_expr(fi, struct_default);
            field_assignments.push(quote! {
                #ident: #skipped_default
            });
            continue;
        }

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

            if fi.is_option {
                let missing = missing_default.unwrap_or_else(|| quote!(None));
                field_assignments.push(quote! {
                    #ident: {
                        let __items = #var_name.clone();
                        if __items.is_empty() {
                            #missing
                        } else {
                            Some(__items)
                        }
                    }
                });
            } else {
                if let Some(missing) = missing_default {
                    field_assignments.push(quote! {
                        #ident: if #var_name.is_empty() {
                            #missing
                        } else {
                            #var_name.clone()
                        }
                    });
                } else {
                    field_assignments.push(quote! {
                        #ident: #var_name.clone()
                    });
                }
            }
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
                let missing = missing_default.unwrap_or_else(|| quote!(None));
                // Option<UploadedFile>: just take the Option
                field_assignments.push(quote! {
                    #ident: match #var_name.clone() {
                        Some(__file) => Some(__file),
                        None => #missing,
                    }
                });
            } else {
                // UploadedFile (non-Option): error if missing
                let missing = missing_default.unwrap_or_else(|| {
                    quote! {
                        return Err(::foundry::foundation::Error::message(
                            format!("field '{}' is required", #name)
                        ))
                    }
                });
                field_assignments.push(quote! {
                    #ident: match #var_name.clone() {
                        Some(__file) => __file,
                        None => #missing,
                    }
                });
            }
        } else if fi.is_vec_nested {
            // Vec<NestedDto>: collect repeated JSON text fields in request order
            let inner_ty = vec_item_type(&fi.ty).expect("Vec fields should expose an inner type");

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

            let parse_expr = generate_parse_json_value(quote!(__item), inner_ty, name);
            let parsed_items = quote! {{
                    let mut __items = Vec::with_capacity(#var_name.len());
                    for __item in #var_name {
                        __items.push(#parse_expr);
                    }
                    __items
            }};
            if fi.is_option {
                let missing = missing_default.unwrap_or_else(|| quote!(None));
                field_assignments.push(quote! {
                    #ident: if #var_name.is_empty() {
                        #missing
                    } else {
                        Some(#parsed_items)
                    }
                });
            } else {
                if let Some(missing) = missing_default {
                    field_assignments.push(quote! {
                        #ident: if #var_name.is_empty() {
                            #missing
                        } else {
                            #parsed_items
                        }
                    });
                } else {
                    field_assignments.push(quote! {
                        #ident: #parsed_items
                    });
                }
            }
        } else if fi.is_nested {
            // Nested DTO: parse the multipart text field as JSON for that child object
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

            if fi.is_option {
                let inner_ty = type_argument_if_last_segment_ident(&fi.ty, "Option")
                    .expect("Option fields should expose an inner type");
                let parse_expr = generate_parse_json_value(quote!(__val), inner_ty, name);
                let missing = missing_default.unwrap_or_else(|| quote!(None));
                field_assignments.push(quote! {
                    #ident: match #var_name {
                        Some(__val) => Some(#parse_expr),
                        None => #missing,
                    }
                });
            } else {
                let ty = &fi.ty;
                let parse_expr = generate_parse_json_value(quote!(__val), ty, name);
                let missing = missing_default.unwrap_or_else(|| {
                    quote! {
                        return Err(::foundry::foundation::Error::message(
                            format!("field '{}' is required", #name)
                        ))
                    }
                });
                field_assignments.push(quote! {
                    #ident: match #var_name {
                        Some(__val) => #parse_expr,
                        None => #missing,
                    }
                });
            }
        } else if fi.is_vec {
            // Vec<T>: collect repeated text fields in request order
            let inner_ty = vec_item_type(&fi.ty).expect("Vec fields should expose an inner type");

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
            let parsed_items = quote! {{
                    let mut __items = Vec::with_capacity(#var_name.len());
                    for __item in #var_name {
                        __items.push(#parse_expr);
                    }
                    __items
            }};
            if fi.is_option {
                let missing = missing_default.unwrap_or_else(|| quote!(None));
                field_assignments.push(quote! {
                    #ident: if #var_name.is_empty() {
                        #missing
                    } else {
                        Some(#parsed_items)
                    }
                });
            } else {
                if let Some(missing) = missing_default {
                    field_assignments.push(quote! {
                        #ident: if #var_name.is_empty() {
                            #missing
                        } else {
                            #parsed_items
                        }
                    });
                } else {
                    field_assignments.push(quote! {
                        #ident: #parsed_items
                    });
                }
            }
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
            let missing = missing_default.unwrap_or_else(|| quote!(None));
            field_assignments.push(quote! {
                #ident: match #var_name {
                    Some(__val) => Some(#parse_expr),
                    None => #missing,
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
            let missing = missing_default.unwrap_or_else(|| {
                quote! {
                    return Err(::foundry::foundation::Error::message(
                        format!("field '{}' is required", #name)
                    ))
                }
            });
            field_assignments.push(quote! {
                #ident: match #var_name {
                    Some(__val) => #parse_expr,
                    None => #missing,
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
                __multipart: &mut ::foundry::axum::extract::Multipart,
            ) -> ::foundry::foundation::Result<Self> {
                #struct_default_decl
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
