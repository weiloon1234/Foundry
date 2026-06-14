use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "payments", primary_key_strategy = "manual")]
struct Payment {
    id: i64,
    amount: Numeric,
}

fn main() {
    let _ = Payment::AMOUNT.eq("12.50");
    let _ = Payment::AMOUNT.in_list(["10.00", "20.00"]);
}
