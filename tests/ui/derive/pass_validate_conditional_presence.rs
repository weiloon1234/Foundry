use foundry::Validate;
use serde::Deserialize;

#[derive(Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
struct ConditionalRequest {
    mode: String,
    email_address: Option<String>,
    #[validate(required_if("mode", "publish"))]
    published_at: Option<String>,
    #[validate(required_unless("mode", "draft"))]
    draft_reason: Option<String>,
    #[validate(required_with("email_address"))]
    contact_note: Option<String>,
    attachments: Vec<String>,
    #[validate(required_with("attachments"))]
    attachment_note: Option<String>,
    #[validate(present)]
    token: Option<String>,
    #[validate(sometimes, email)]
    reply_to: Option<String>,
    #[validate(prohibited)]
    internal_note: Option<String>,
    #[validate(boolean)]
    enabled: bool,
    #[validate(distinct, each(required))]
    tags: Vec<String>,
}

fn main() {}
