use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "merchants")]
struct Merchant {
    id: ModelId<Merchant>,
    tags: Loaded<Vec<Tag>>,
    tag_count: Loaded<i64>,
}

#[derive(foundry::Model)]
#[foundry(table = "tags")]
struct Tag {
    id: ModelId<Tag>,
    name: String,
    link: Loaded<TagLink>,
}

#[derive(Clone, Debug, PartialEq, foundry::Projection)]
struct TagLink {
    #[foundry(source = "role")]
    role: String,
}

impl Merchant {
    fn tags() -> ManyToManyDef<Self, Tag, ()> {
        many_to_many(
            Self::ID,
            "merchant_tags",
            "merchant_id",
            "tag_id",
            Tag::ID,
            |merchant| merchant.id,
            |merchant, tags| merchant.tags = Loaded::new(tags),
        )
    }

    fn tags_with_pivot() -> ManyToManyDef<Self, Tag, TagLink> {
        Self::tags().with_pivot(TagLink::projection_meta(), |tag, link| {
            tag.link = Loaded::new(link)
        })
    }

    fn tag_count() -> RelationAggregateDef<Self, i64> {
        Self::tags().count(|merchant, count| merchant.tag_count = Loaded::new(count))
    }
}

fn main() -> Result<()> {
    let query = Merchant::query()
        .with_aggregate(Merchant::tag_count())
        .with_many_to_many(Merchant::tags_with_pivot());

    println!("{:?}", query.ast());
    Ok(())
}
