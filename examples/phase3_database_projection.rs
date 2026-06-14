use foundry::prelude::*;

#[derive(Clone, Debug, PartialEq, foundry::Projection)]
struct UserPreferenceRow {
    #[foundry(source = "email")]
    email: String,
    #[foundry(source = "status_label")]
    status_label: String,
    #[foundry(source = "theme")]
    theme: String,
}

#[derive(Clone, Debug, PartialEq, foundry::Projection)]
struct CombinedLabelRow {
    label: String,
    kind: String,
}

fn main() -> Result<()> {
    let active_users = Query::table("users")
        .select_expr(
            ColumnRef::new("users", "email"),
            UserPreferenceRow::EMAIL.alias(),
        )
        .select_expr(
            Case::when(
                Condition::compare(
                    Expr::column(ColumnRef::new("users", "active")),
                    ComparisonOp::Eq,
                    Expr::value(true),
                ),
                Expr::value("active"),
            )
            .else_(Expr::value("inactive")),
            UserPreferenceRow::STATUS_LABEL.alias(),
        )
        .select_expr(
            Expr::column(ColumnRef::new("users", "metadata").typed(DbType::Json))
                .json()
                .key("theme")
                .as_text(),
            UserPreferenceRow::THEME.alias(),
        );

    let preferences = ProjectionQuery::<UserPreferenceRow>::table("active_users")
        .with_cte(Cte::new("active_users", active_users))
        .select_source(UserPreferenceRow::EMAIL, "active_users")
        .select_source(UserPreferenceRow::STATUS_LABEL, "active_users")
        .select_source(UserPreferenceRow::THEME, "active_users");

    let combined = ProjectionQuery::<CombinedLabelRow>::table("users")
        .select_field(CombinedLabelRow::LABEL, ColumnRef::new("users", "email"))
        .select_field(CombinedLabelRow::KIND, Expr::value("user"))
        .union_all(
            ProjectionQuery::<CombinedLabelRow>::table("tags")
                .select_field(CombinedLabelRow::LABEL, ColumnRef::new("tags", "name"))
                .select_field(CombinedLabelRow::KIND, Expr::value("tag")),
        )
        .order_by(OrderBy::asc(CombinedLabelRow::LABEL.alias()));

    let joined_users = Query::table("users")
        .left_join(
            "tags",
            Condition::compare(
                Expr::column(ColumnRef::new("tags", "user_id")),
                ComparisonOp::Eq,
                Expr::column(ColumnRef::new("users", "id")),
            ),
        )
        .select_expr(ColumnRef::new("users", "email"), "email")
        .select_expr(ColumnRef::new("tags", "name"), "tag_name");

    let user_tag_pairs = Query::table("users")
        .cross_join("tags")
        .select_expr(ColumnRef::new("users", "email"), "email")
        .select_expr(ColumnRef::new("tags", "name"), "tag_name");

    println!("{:?}", preferences.ast());
    println!("{:?}", combined.ast());
    println!("{:?}", joined_users.ast());
    println!("{:?}", user_tag_pairs.ast());
    Ok(())
}
