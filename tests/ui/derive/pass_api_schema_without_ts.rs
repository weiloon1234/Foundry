#[derive(foundry::ApiSchema)]
struct OpenApiOnly {
    name: String,
}

fn main() {
    let schema = <OpenApiOnly as foundry::ApiSchema>::schema();
    assert_eq!(schema["properties"]["name"]["type"], "string");
}
