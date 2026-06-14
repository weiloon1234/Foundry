use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use crate::app_enum::{EnumKey, EnumOption, FoundryAppEnum};
use crate::support::Collection;

use super::column::DatatableFieldRef;
use super::request::DatatableFilterOp;

// ---------------------------------------------------------------------------
// Filter kind
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, ts_rs::TS, foundry_macros::TS)]
#[serde(rename_all = "snake_case")]
pub enum DatatableFilterKind {
    Text,
    Number,
    Select,
    Checkbox,
    Date,
    DateTime,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, ts_rs::TS, foundry_macros::TS)]
#[serde(rename_all = "snake_case")]
pub enum DatatableFilterValueKind {
    Text,
    Boolean,
    Integer,
    Decimal,
    Date,
    DateTime,
    Values,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq, ts_rs::TS, foundry_macros::TS)]
pub struct DatatableFilterBinding {
    pub field: String,
    pub op: DatatableFilterOp,
    pub value_kind: DatatableFilterValueKind,
}

impl DatatableFilterBinding {
    pub fn new(
        field: impl Into<String>,
        op: DatatableFilterOp,
        value_kind: DatatableFilterValueKind,
    ) -> Self {
        Self {
            field: field.into(),
            op,
            value_kind,
        }
    }
}

// ---------------------------------------------------------------------------
// Select option
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug, PartialEq, Eq, ts_rs::TS, foundry_macros::TS)]
pub struct DatatableFilterOption {
    pub value: String,
    /// For AppEnum-backed filters this carries the translation key.
    pub label: String,
}

#[derive(Serialize, Clone, Debug, Default, ts_rs::TS, foundry_macros::TS)]
struct DatatableFilterOptions {
    pub items: Vec<DatatableFilterOption>,
}

impl DatatableFilterOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

impl From<EnumOption> for DatatableFilterOption {
    fn from(option: EnumOption) -> Self {
        let value = match option.value {
            EnumKey::String(value) => value,
            EnumKey::Int(value) => value.to_string(),
        };

        Self {
            value,
            label: option.label_key,
        }
    }
}

impl From<Collection<EnumOption>> for Collection<DatatableFilterOption> {
    fn from(options: Collection<EnumOption>) -> Self {
        options.map_into(Into::into)
    }
}

// ---------------------------------------------------------------------------
// Filter field
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, ts_rs::TS, foundry_macros::TS)]
pub struct DatatableFilterField {
    pub name: String,
    pub kind: DatatableFilterKind,
    pub label: String,
    pub binding: DatatableFilterBinding,
    #[ts(optional)]
    pub placeholder: Option<String>,
    #[ts(optional)]
    pub help: Option<String>,
    pub nullable: bool,
    #[ts(as = "DatatableFilterOptions")]
    pub options: Collection<DatatableFilterOption>,
}

impl Serialize for DatatableFilterField {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct(
            "DatatableFilterField",
            6 + usize::from(self.placeholder.is_some()) + usize::from(self.help.is_some()),
        )?;

        state.serialize_field("name", &self.name)?;
        state.serialize_field("kind", &self.kind)?;
        state.serialize_field("label", &self.label)?;
        state.serialize_field("binding", &self.binding)?;
        if let Some(placeholder) = &self.placeholder {
            state.serialize_field("placeholder", placeholder)?;
        }
        if let Some(help) = &self.help {
            state.serialize_field("help", help)?;
        }
        state.serialize_field("nullable", &self.nullable)?;
        state.serialize_field("options", &self.options)?;
        state.end()
    }
}

impl DatatableFilterField {
    fn new(name: impl Into<String>, label: impl Into<String>, kind: DatatableFilterKind) -> Self {
        let name = name.into();
        Self {
            binding: Self::default_binding(name.as_str(), kind),
            name,
            kind,
            label: label.into(),
            placeholder: None,
            help: None,
            nullable: false,
            options: Collection::new(),
        }
    }

    fn new_with_binding(
        name: impl Into<String>,
        label: impl Into<String>,
        kind: DatatableFilterKind,
        op: DatatableFilterOp,
        value_kind: DatatableFilterValueKind,
    ) -> Self {
        let name = name.into();
        Self {
            binding: DatatableFilterBinding::new(name.clone(), op, value_kind),
            name,
            kind,
            label: label.into(),
            placeholder: None,
            help: None,
            nullable: false,
            options: Collection::new(),
        }
    }

    fn default_binding(name: &str, kind: DatatableFilterKind) -> DatatableFilterBinding {
        match kind {
            DatatableFilterKind::Text => DatatableFilterBinding::new(
                name,
                DatatableFilterOp::Eq,
                DatatableFilterValueKind::Text,
            ),
            DatatableFilterKind::Number => DatatableFilterBinding::new(
                name,
                DatatableFilterOp::Eq,
                DatatableFilterValueKind::Integer,
            ),
            DatatableFilterKind::Select => DatatableFilterBinding::new(
                name,
                DatatableFilterOp::Eq,
                DatatableFilterValueKind::Text,
            ),
            DatatableFilterKind::Checkbox => DatatableFilterBinding::new(
                name,
                DatatableFilterOp::Eq,
                DatatableFilterValueKind::Boolean,
            ),
            DatatableFilterKind::Date => DatatableFilterBinding::new(
                name,
                DatatableFilterOp::Date,
                DatatableFilterValueKind::Date,
            ),
            DatatableFilterKind::DateTime => DatatableFilterBinding::new(
                name,
                DatatableFilterOp::Datetime,
                DatatableFilterValueKind::DateTime,
            ),
        }
    }

    pub fn text(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(name, label, DatatableFilterKind::Text)
    }

    pub fn text_like(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new_with_binding(
            name,
            label,
            DatatableFilterKind::Text,
            DatatableFilterOp::Like,
            DatatableFilterValueKind::Text,
        )
    }

    pub fn text_search(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new_with_binding(
            name,
            label,
            DatatableFilterKind::Text,
            DatatableFilterOp::LikeAny,
            DatatableFilterValueKind::Text,
        )
    }

    pub fn text_search_fields<Row, I, F>(
        name: impl Into<String>,
        label: impl Into<String>,
        fields: I,
    ) -> Self
    where
        Row: 'static,
        I: IntoIterator<Item = F>,
        F: Into<DatatableFieldRef<Row>>,
    {
        Self::text_search(name, label).server_fields(fields)
    }

    pub fn number(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(name, label, DatatableFilterKind::Number)
    }

    pub fn decimal_min(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new_with_binding(
            name,
            label,
            DatatableFilterKind::Number,
            DatatableFilterOp::Gte,
            DatatableFilterValueKind::Decimal,
        )
    }

    pub fn decimal_max(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new_with_binding(
            name,
            label,
            DatatableFilterKind::Number,
            DatatableFilterOp::Lte,
            DatatableFilterValueKind::Decimal,
        )
    }

    pub fn select(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(name, label, DatatableFilterKind::Select)
    }

    pub fn checkbox(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(name, label, DatatableFilterKind::Checkbox)
    }

    pub fn date(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(name, label, DatatableFilterKind::Date)
    }

    pub fn date_from(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new_with_binding(
            name,
            label,
            DatatableFilterKind::Date,
            DatatableFilterOp::DateFrom,
            DatatableFilterValueKind::Date,
        )
    }

    pub fn date_to(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new_with_binding(
            name,
            label,
            DatatableFilterKind::Date,
            DatatableFilterOp::DateTo,
            DatatableFilterValueKind::Date,
        )
    }

    pub fn datetime(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(name, label, DatatableFilterKind::DateTime)
    }

    // -- builder helpers ---------------------------------------------------

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    pub fn options<I>(mut self, options: I) -> Self
    where
        I: Into<Collection<DatatableFilterOption>>,
    {
        self.options = options.into();
        self
    }

    /// Populate select options directly from an `AppEnum`.
    pub fn enum_options<E: FoundryAppEnum>(self) -> Self {
        self.options(E::options())
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn nullable(mut self) -> Self {
        self.nullable = true;
        self
    }

    pub fn server_field<Row, F>(self, field: F) -> Self
    where
        Row: 'static,
        F: Into<DatatableFieldRef<Row>>,
    {
        self.server_fields(std::iter::once(field))
    }

    pub fn server_fields<Row, I, F>(mut self, fields: I) -> Self
    where
        Row: 'static,
        I: IntoIterator<Item = F>,
        F: Into<DatatableFieldRef<Row>>,
    {
        assert!(
            self.binding.op == DatatableFilterOp::LikeAny,
            "server_fields(...) is only supported for LikeAny search filters; use bind(...) for non-search filter targets"
        );

        self.binding.field = join_server_fields(fields);
        self
    }

    pub fn bind(
        mut self,
        field: impl Into<String>,
        op: DatatableFilterOp,
        value_kind: DatatableFilterValueKind,
    ) -> Self {
        self.binding = DatatableFilterBinding::new(field, op, value_kind);
        self
    }

    /// Create a select filter with options auto-populated from an `AppEnum`.
    ///
    /// The option label preserves the enum's label metadata unchanged. For
    /// default `AppEnum` usage this means the datatable payload carries the
    /// translation key such as `enum.order_status.pending`.
    ///
    /// Works with both string-backed (`{ Pending, Completed }`) and
    /// int-backed (`{ Pending = 0, Completed = 1 }`) enums.
    ///
    /// ```ignore
    /// DatatableFilterField::enum_select::<CountryStatus>("status", "Status")
    /// ```
    pub fn enum_select<E: FoundryAppEnum>(
        name: impl Into<String>,
        label: impl Into<String>,
    ) -> Self {
        Self::select(name, label).options(E::options())
    }
}

fn join_server_fields<Row, I, F>(fields: I) -> String
where
    Row: 'static,
    I: IntoIterator<Item = F>,
    F: Into<DatatableFieldRef<Row>>,
{
    let field_names: Vec<String> = fields.into_iter().map(|field| field.into().name).collect();

    assert!(
        !field_names.is_empty(),
        "datatable search filters require at least one server field"
    );

    for field_name in &field_names {
        assert!(
            !field_name.contains('|'),
            "datatable search field `{}` cannot contain `|`",
            field_name
        );
    }

    field_names.join("|")
}

// ---------------------------------------------------------------------------
// Filter row (layout)
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug, ts_rs::TS, foundry_macros::TS)]
pub struct DatatableFilterRow {
    pub fields: Vec<DatatableFilterField>,
}

impl DatatableFilterRow {
    pub fn single(field: DatatableFilterField) -> Self {
        Self {
            fields: vec![field],
        }
    }

    pub fn pair(left: DatatableFilterField, right: DatatableFilterField) -> Self {
        Self {
            fields: vec![left, right],
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::app_enum::FoundryAppEnum;
    use crate::database::{DbType, ProjectionField};
    use crate::datatable::{DatatableFilterOp, DatatableFilterValueKind};
    use crate::logging::catch_sync_panic;

    use super::DatatableFilterField;

    #[derive(Clone, Debug, PartialEq, Eq, crate::AppEnum)]
    enum PaymentStatus {
        Pending,
        Completed,
    }

    #[derive(Clone, Debug, PartialEq, Eq, crate::AppEnum)]
    enum PaymentPriority {
        Low = 1,
        High = 2,
    }

    #[derive(Debug, serde::Serialize, crate::Model)]
    #[foundry(table = "datatable_filter_models", primary_key_strategy = "manual")]
    struct SearchModel {
        id: i64,
        name: String,
        email: String,
        amount: i64,
    }

    #[derive(Clone, serde::Serialize)]
    struct SearchProjection {
        title: String,
        slug: String,
    }

    impl SearchProjection {
        const TITLE: ProjectionField<Self, String> = ProjectionField::new("title", DbType::Text);
        const SLUG: ProjectionField<Self, String> = ProjectionField::new("slug", DbType::Text);
        const INVALID: ProjectionField<Self, String> =
            ProjectionField::new("slug|title", DbType::Text);
    }

    #[test]
    fn serializes_optional_metadata_only_when_present() {
        let filter = DatatableFilterField::text("status", "Status");
        assert_eq!(
            serde_json::to_value(&filter).unwrap(),
            json!({
                "name": "status",
                "kind": "text",
                "label": "Status",
                "binding": {
                    "field": "status",
                    "op": "eq",
                    "value_kind": "text"
                },
                "nullable": false,
                "options": {
                    "items": []
                }
            })
        );

        let filter = DatatableFilterField::text("status", "Status")
            .placeholder("Search status")
            .help("Filters by status");
        assert_eq!(
            serde_json::to_value(&filter).unwrap(),
            json!({
                "name": "status",
                "kind": "text",
                "label": "Status",
                "binding": {
                    "field": "status",
                    "op": "eq",
                    "value_kind": "text"
                },
                "placeholder": "Search status",
                "help": "Filters by status",
                "nullable": false,
                "options": {
                    "items": []
                }
            })
        );
    }

    #[test]
    fn bind_overrides_server_field_operator_and_value_kind() {
        let filter = DatatableFilterField::number("minimum_amount", "Minimum Amount").bind(
            "amount",
            DatatableFilterOp::Gte,
            DatatableFilterValueKind::Decimal,
        );

        assert_eq!(
            serde_json::to_value(&filter).unwrap(),
            json!({
                "name": "minimum_amount",
                "kind": "number",
                "label": "Minimum Amount",
                "binding": {
                    "field": "amount",
                    "op": "gte",
                    "value_kind": "decimal"
                },
                "nullable": false,
                "options": {
                    "items": []
                }
            })
        );
    }

    #[test]
    fn semantic_helpers_provide_expected_default_bindings() {
        let text_like = DatatableFilterField::text_like("status", "Status");
        assert_eq!(text_like.binding.field, "status");
        assert_eq!(text_like.binding.op, DatatableFilterOp::Like);
        assert_eq!(text_like.binding.value_kind, DatatableFilterValueKind::Text);

        let text_search = DatatableFilterField::text_search("query", "Search");
        assert_eq!(text_search.binding.field, "query");
        assert_eq!(text_search.binding.op, DatatableFilterOp::LikeAny);
        assert_eq!(
            text_search.binding.value_kind,
            DatatableFilterValueKind::Text
        );

        let date_from = DatatableFilterField::date_from("starts_on", "Starts On");
        assert_eq!(date_from.binding.field, "starts_on");
        assert_eq!(date_from.binding.op, DatatableFilterOp::DateFrom);
        assert_eq!(date_from.binding.value_kind, DatatableFilterValueKind::Date);

        let decimal_min = DatatableFilterField::decimal_min("minimum_total", "Minimum Total");
        assert_eq!(decimal_min.binding.field, "minimum_total");
        assert_eq!(decimal_min.binding.op, DatatableFilterOp::Gte);
        assert_eq!(
            decimal_min.binding.value_kind,
            DatatableFilterValueKind::Decimal
        );
    }

    #[test]
    fn select_options_accepts_enum_options_directly() {
        let filter =
            DatatableFilterField::select("status", "Status").options(PaymentStatus::options());

        assert_eq!(
            serde_json::to_value(&filter).unwrap()["options"],
            json!({
                "items": [
                    {
                        "value": "pending",
                        "label": "enum.payment_status.pending"
                    },
                    {
                        "value": "completed",
                        "label": "enum.payment_status.completed"
                    }
                ]
            })
        );
    }

    #[test]
    fn enum_options_builder_uses_enum_metadata() {
        let filter =
            DatatableFilterField::select("status", "Status").enum_options::<PaymentStatus>();

        assert_eq!(filter.options[0].label, "enum.payment_status.pending");
        assert_eq!(filter.options[1].label, "enum.payment_status.completed");
    }

    #[test]
    fn enum_select_preserves_label_keys_and_stringifies_int_values() {
        let filter = DatatableFilterField::enum_select::<PaymentPriority>("priority", "Priority");

        assert_eq!(
            serde_json::to_value(&filter).unwrap()["options"],
            json!({
                "items": [
                    {
                        "value": "1",
                        "label": "enum.payment_priority.low"
                    },
                    {
                        "value": "2",
                        "label": "enum.payment_priority.high"
                    }
                ]
            })
        );
    }

    #[test]
    fn server_field_overrides_only_the_binding_field() {
        let filter =
            DatatableFilterField::text_search("search", "Search").server_field(SearchModel::EMAIL);

        assert_eq!(filter.name, "search");
        assert_eq!(filter.binding.field, "email");
        assert_eq!(filter.binding.op, DatatableFilterOp::LikeAny);
        assert_eq!(filter.binding.value_kind, DatatableFilterValueKind::Text);
    }

    #[test]
    fn text_search_fields_joins_model_columns_into_like_any_binding() {
        let filter = DatatableFilterField::text_search_fields(
            "search",
            "Search",
            [SearchModel::NAME, SearchModel::EMAIL],
        );

        assert_eq!(filter.binding.field, "name|email");
        assert_eq!(filter.binding.op, DatatableFilterOp::LikeAny);
        assert_eq!(filter.binding.value_kind, DatatableFilterValueKind::Text);
    }

    #[test]
    fn text_search_fields_joins_projection_fields_into_like_any_binding() {
        let filter = DatatableFilterField::text_search_fields(
            "search",
            "Search",
            [SearchProjection::TITLE, SearchProjection::SLUG],
        );

        assert_eq!(filter.binding.field, "title|slug");
        assert_eq!(filter.binding.op, DatatableFilterOp::LikeAny);
        assert_eq!(filter.binding.value_kind, DatatableFilterValueKind::Text);
    }

    #[test]
    fn server_fields_rejects_empty_field_lists() {
        let result = catch_sync_panic(|| {
            DatatableFilterField::text_search("search", "Search")
                .server_fields::<SearchModel, _, crate::database::Column<SearchModel, String>>(
                    std::iter::empty(),
                )
        });

        assert!(result.is_err(), "empty search fields should panic");
    }

    #[test]
    fn server_fields_rejects_field_names_containing_pipe() {
        let result = catch_sync_panic(|| {
            DatatableFilterField::text_search("search", "Search")
                .server_fields([SearchProjection::INVALID])
        });

        assert!(result.is_err(), "pipe-delimited field names should panic");
    }

    #[test]
    fn server_fields_rejects_non_search_filters() {
        let result = catch_sync_panic(|| {
            DatatableFilterField::text("status", "Status").server_field(SearchModel::NAME)
        });

        assert!(result.is_err(), "non-LikeAny filters should panic");
    }
}
