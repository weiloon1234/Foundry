use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "users")]
struct User {
    id: ModelId<User>,
    merchants: Loaded<Vec<Merchant>>,
    merchant_count: Loaded<i64>,
}

#[derive(foundry::Model)]
#[foundry(table = "merchants")]
struct Merchant {
    id: ModelId<Merchant>,
    user_id: ModelId<User>,
    orders: Loaded<Vec<Order>>,
    order_total: Loaded<Option<i64>>,
}

#[derive(foundry::Model)]
#[foundry(table = "orders")]
struct Order {
    id: ModelId<Order>,
    merchant_id: ModelId<Merchant>,
    total: i64,
    items: Loaded<Vec<OrderItem>>,
}

#[derive(foundry::Model)]
#[foundry(table = "order_items")]
struct OrderItem {
    id: ModelId<OrderItem>,
    order_id: ModelId<Order>,
    product_id: ModelId<Product>,
    product: Loaded<Option<Product>>,
}

#[derive(foundry::Model)]
#[foundry(table = "products")]
struct Product {
    id: ModelId<Product>,
}

impl User {
    fn merchants() -> RelationDef<Self, Merchant> {
        has_many(
            Self::ID,
            Merchant::USER_ID,
            |user| user.id,
            |user, merchants| user.merchants = Loaded::new(merchants),
        )
    }

    fn merchant_count() -> RelationAggregateDef<Self, i64> {
        Self::merchants().count(|user, count| user.merchant_count = Loaded::new(count))
    }
}

impl Merchant {
    fn orders() -> RelationDef<Self, Order> {
        has_many(
            Self::ID,
            Order::MERCHANT_ID,
            |merchant| merchant.id,
            |merchant, orders| merchant.orders = Loaded::new(orders),
        )
    }

    fn order_total() -> RelationAggregateDef<Self, Option<i64>> {
        Self::orders().sum(Order::TOTAL, |merchant, total| {
            merchant.order_total = Loaded::new(total)
        })
    }
}

impl Order {
    fn items() -> RelationDef<Self, OrderItem> {
        has_many(
            Self::ID,
            OrderItem::ORDER_ID,
            |order| order.id,
            |order, items| order.items = Loaded::new(items),
        )
    }
}

impl OrderItem {
    fn product() -> RelationDef<Self, Product> {
        belongs_to(
            Self::PRODUCT_ID,
            Product::ID,
            |item| Some(item.product_id),
            |item, product| item.product = Loaded::new(product),
        )
    }
}

fn main() -> Result<()> {
    let query = User::query().with_aggregate(User::merchant_count()).with(
        User::merchants()
            .with_aggregate(Merchant::order_total())
            .with(Merchant::orders().with(Order::items().with(OrderItem::product()))),
    );

    println!("{:?}", query.ast());
    Ok(())
}
