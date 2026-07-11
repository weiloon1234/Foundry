use foundry::prelude::*;
use serde_json::json;

#[tokio::test]
async fn outbound_http_client_surface_is_available_to_consumers() {
    let fake = HttpClientFake::new();
    fake.respond_json(HttpStatus::OK, &json!({ "framework": "foundry" }))
        .unwrap();
    let client = fake
        .client_builder()
        .base_url("https://api.example.test/v1")
        .unwrap()
        .retry_policy(RetryPolicy::none())
        .build()
        .unwrap();

    let response = client
        .request(HttpMethod::GET, "status")
        .query_pair("verbose", true)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), HttpStatus::OK);
    assert_eq!(
        response.json::<serde_json::Value>().unwrap(),
        json!({ "framework": "foundry" })
    );
    fake.assert_sent(|request| {
        request.url().as_str() == "https://api.example.test/v1/status?verbose=true"
    });

    let _headers = HttpHeaderMap::new();
    assert!(HttpClient::new().unwrap().raw().is_some());
}
