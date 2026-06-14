use foundry::prelude::*;

fn main() -> Result<()> {
    let total_users = AggregateProjection::<i64>::count_all("total_users");
    let query = Query::table("users")
        .select(["active"])
        .select_aggregate(total_users)
        .where_eq("active", true)
        .group_by("active")
        .having(Condition::compare(
            Expr::Aggregate(AggregateExpr::count_all()),
            ComparisonOp::Gt,
            Expr::value(0_i64),
        ))
        .order_by(OrderBy::asc("active"))
        .limit(20);

    let compiled = query.compile()?;
    println!("{}", compiled.sql);
    println!("{:?}", compiled.bindings);

    Ok(())
}
