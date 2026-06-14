use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::address::EmailAddress;
use super::attachment::EmailAttachment;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmailMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) from: Option<EmailAddress>,
    pub(crate) to: Vec<EmailAddress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) cc: Vec<EmailAddress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) bcc: Vec<EmailAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reply_to: Option<EmailAddress>,
    pub(crate) subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) text_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) html_body: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) attachments: Vec<EmailAttachment>,
}

impl EmailMessage {
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            from: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: None,
            subject: subject.into(),
            text_body: None,
            html_body: None,
            headers: BTreeMap::new(),
            attachments: Vec::new(),
        }
    }

    pub fn from(mut self, addr: impl Into<EmailAddress>) -> Self {
        self.from = Some(addr.into());
        self
    }

    pub fn to(mut self, addr: impl Into<EmailAddress>) -> Self {
        self.to.push(addr.into());
        self
    }

    pub fn cc(mut self, addr: impl Into<EmailAddress>) -> Self {
        self.cc.push(addr.into());
        self
    }

    pub fn bcc(mut self, addr: impl Into<EmailAddress>) -> Self {
        self.bcc.push(addr.into());
        self
    }

    pub fn reply_to(mut self, addr: impl Into<EmailAddress>) -> Self {
        self.reply_to = Some(addr.into());
        self
    }

    pub fn text_body(mut self, body: impl Into<String>) -> Self {
        self.text_body = Some(body.into());
        self
    }

    pub fn html_body(mut self, body: impl Into<String>) -> Self {
        self.html_body = Some(body.into());
        self
    }

    /// Render an email template and set the body.
    ///
    /// Loads `{template_name}.html` and `{template_name}.txt` from the template
    /// directory, replaces `{{variable}}` placeholders with the provided values.
    ///
    /// ```ignore
    /// let msg = EmailMessage::new("Welcome!")
    ///     .to(&user.email)
    ///     .template("welcome", "templates/emails", json!({
    ///         "name": user.name,
    ///         "app_name": "MyApp",
    ///     }))
    ///     .await?;
    /// ```
    pub async fn template(
        self,
        template_name: &str,
        template_path: &str,
        variables: serde_json::Value,
    ) -> crate::foundation::Result<Self> {
        let renderer = crate::email::template::TemplateRenderer::new(template_path);
        let rendered = renderer.render_async(template_name, &variables).await?;
        let mut msg = self;
        if let Some(html) = rendered.html {
            msg = msg.html_body(html);
        }
        if let Some(text) = rendered.text {
            msg = msg.text_body(text);
        }
        Ok(msg)
    }

    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    pub fn attach(mut self, attachment: EmailAttachment) -> Self {
        self.attachments.push(attachment);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn new_creates_message_with_subject() {
        let msg = EmailMessage::new("Test Subject");
        assert_eq!(msg.subject, "Test Subject");
        assert!(msg.from.is_none());
        assert!(msg.to.is_empty());
        assert!(msg.cc.is_empty());
        assert!(msg.bcc.is_empty());
        assert!(msg.reply_to.is_none());
        assert!(msg.text_body.is_none());
        assert!(msg.html_body.is_none());
        assert!(msg.headers.is_empty());
        assert!(msg.attachments.is_empty());
    }

    #[test]
    fn builder_chain_sets_all_fields() {
        let attachment = EmailAttachment::from_path("/tmp/test.txt").with_name("test.txt");

        let msg = EmailMessage::new("Full Test")
            .from("sender@example.com")
            .to("recipient@example.com")
            .cc("cc1@example.com")
            .bcc("bcc1@example.com")
            .reply_to("reply@example.com")
            .text_body("Hello World")
            .html_body("<p>Hello World</p>")
            .header("X-Custom", "value")
            .attach(attachment.clone());

        assert_eq!(msg.from.unwrap().to_string(), "sender@example.com");
        assert_eq!(msg.to[0].to_string(), "recipient@example.com");
        assert_eq!(msg.cc[0].to_string(), "cc1@example.com");
        assert_eq!(msg.bcc[0].to_string(), "bcc1@example.com");
        assert_eq!(msg.reply_to.unwrap().to_string(), "reply@example.com");
        assert_eq!(msg.text_body.unwrap(), "Hello World");
        assert_eq!(msg.html_body.unwrap(), "<p>Hello World</p>");
        assert_eq!(msg.headers.get("X-Custom"), Some(&"value".to_string()));
        assert_eq!(msg.attachments.len(), 1);
        assert!(matches!(msg.attachments[0], EmailAttachment::Path { .. }));
    }

    #[test]
    fn to_accepts_str() {
        let msg = EmailMessage::new("Test").to("user@example.com");
        assert_eq!(msg.to[0].to_string(), "user@example.com");
    }

    #[test]
    fn multiple_to_recipients() {
        let msg = EmailMessage::new("Test")
            .to("user1@example.com")
            .to("user2@example.com")
            .to("user3@example.com");

        assert_eq!(msg.to.len(), 3);
        assert_eq!(msg.to[0].to_string(), "user1@example.com");
        assert_eq!(msg.to[1].to_string(), "user2@example.com");
        assert_eq!(msg.to[2].to_string(), "user3@example.com");
    }

    #[test]
    fn serialization_roundtrip() {
        let original = EmailMessage::new("Full Test")
            .from("sender@example.com")
            .to("recipient@example.com")
            .cc("cc1@example.com")
            .bcc("bcc1@example.com")
            .reply_to("reply@example.com")
            .text_body("Hello World")
            .html_body("<p>Hello World</p>")
            .header("X-Custom", "value")
            .header("X-Priority", "high")
            .attach(EmailAttachment::from_path("/tmp/test.txt").with_name("test.txt"));

        let json = serde_json::to_string(&original).expect("Failed to serialize");
        let deserialized: EmailMessage =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(original.from, deserialized.from);
        assert_eq!(original.to, deserialized.to);
        assert_eq!(original.cc, deserialized.cc);
        assert_eq!(original.bcc, deserialized.bcc);
        assert_eq!(original.reply_to, deserialized.reply_to);
        assert_eq!(original.subject, deserialized.subject);
        assert_eq!(original.text_body, deserialized.text_body);
        assert_eq!(original.html_body, deserialized.html_body);
        assert_eq!(original.headers, deserialized.headers);
        assert_eq!(original.attachments, deserialized.attachments);
    }

    #[test]
    fn serialization_omits_empty_fields() {
        let msg = EmailMessage::new("Minimal Test")
            .to("user@example.com")
            .text_body("Simple message");

        let json = serde_json::to_string(&msg).expect("Failed to serialize");

        assert!(json.contains("\"subject\":\"Minimal Test\""));
        assert!(json.contains("user@example.com"));
        assert!(json.contains("\"text_body\":\"Simple message\""));
        assert!(!json.contains("\"from\":"));
        assert!(!json.contains("\"cc\":"));
        assert!(!json.contains("\"bcc\":"));
        assert!(!json.contains("\"reply_to\":"));
        assert!(!json.contains("\"html_body\":"));
        assert!(!json.contains("\"headers\":"));
        assert!(!json.contains("\"attachments\":"));
    }
}
