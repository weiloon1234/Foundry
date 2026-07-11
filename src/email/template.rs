use std::path::PathBuf;

use crate::foundation::{Error, Result};

/// Simple email template renderer using `{{variable}}` replacement.
///
/// Templates are loaded from the filesystem at `templates/emails/` by default.
/// Each template can have `.html` and `.txt` variants.
/// HTML variables are escaped by default; `{{{variable}}}` opts trusted values
/// into raw HTML. Text templates always render values without HTML escaping.
///
/// ```ignore
/// let renderer = TemplateRenderer::new("templates/emails");
/// let (html, text) = renderer.render("welcome", &json!({
///     "name": "Alice",
///     "app_name": "MyApp",
/// }))?;
/// ```
pub struct TemplateRenderer {
    base_path: PathBuf,
}

impl TemplateRenderer {
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
        }
    }

    /// Render a template by name with the given variables.
    ///
    /// Returns `(Option<html>, Option<text>)` — at least one will be `Some` if the
    /// template exists.
    pub fn render(
        &self,
        template_name: &str,
        variables: &serde_json::Value,
    ) -> Result<RenderedTemplate> {
        validate_template_name(template_name)?;

        let html_path = self.base_path.join(format!("{template_name}.html"));
        let text_path = self.base_path.join(format!("{template_name}.txt"));

        let html = match std::fs::read_to_string(&html_path) {
            Ok(content) => Some(replace_variables(&content, variables, TemplateMode::Html)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(Error::message(format!(
                    "failed to read email template '{}': {e}",
                    html_path.display()
                )))
            }
        };

        let text = match std::fs::read_to_string(&text_path) {
            Ok(content) => Some(replace_variables(&content, variables, TemplateMode::Text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(Error::message(format!(
                    "failed to read email template '{}': {e}",
                    text_path.display()
                )))
            }
        };

        if html.is_none() && text.is_none() {
            return Err(Error::message(format!(
                "email template '{template_name}' not found (checked {}.html and {}.txt in {})",
                template_name,
                template_name,
                self.base_path.display()
            )));
        }

        Ok(RenderedTemplate { html, text })
    }

    /// Render a template on Tokio's blocking thread pool.
    ///
    /// Use this from async contexts: [`render`](Self::render) reads template
    /// files synchronously, which would stall the async runtime under load.
    pub async fn render_async(
        &self,
        template_name: &str,
        variables: &serde_json::Value,
    ) -> Result<RenderedTemplate> {
        let renderer = Self {
            base_path: self.base_path.clone(),
        };
        let template_name = template_name.to_string();
        let variables = variables.clone();
        crate::support::run_blocking("email.template_render", move || {
            renderer.render(&template_name, &variables)
        })
        .await
    }

    /// Check if a template exists (either .html or .txt variant).
    pub fn exists(&self, template_name: &str) -> bool {
        if validate_template_name(template_name).is_err() {
            return false;
        }

        let html_path = self.base_path.join(format!("{template_name}.html"));
        let text_path = self.base_path.join(format!("{template_name}.txt"));
        html_path.exists() || text_path.exists()
    }
}

fn validate_template_name(template_name: &str) -> Result<()> {
    if template_name.is_empty() {
        return Err(Error::message("email template name cannot be empty"));
    }
    if template_name.contains('\\') {
        return Err(Error::message(
            "email template name cannot contain backslash separators",
        ));
    }
    if template_name.chars().any(|ch| ch.is_control()) {
        return Err(Error::message(
            "email template name cannot contain control characters",
        ));
    }
    for segment in template_name.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(Error::message(
                "email template name must be a safe relative template path",
            ));
        }
    }
    Ok(())
}

/// Result of rendering an email template.
pub struct RenderedTemplate {
    pub html: Option<String>,
    pub text: Option<String>,
}

#[derive(Clone, Copy)]
enum TemplateMode {
    Html,
    Text,
}

/// Replace placeholders in one pass over the original template.
///
/// Supports nested access via dot notation. Unmatched placeholders are left
/// unchanged, and replacement values are never parsed as template syntax.
fn replace_variables(content: &str, variables: &serde_json::Value, mode: TemplateMode) -> String {
    let mut result = String::with_capacity(content.len());
    let mut cursor = 0;

    while let Some(offset) = content[cursor..].find("{{") {
        let start = cursor + offset;
        result.push_str(&content[cursor..start]);

        let raw = content[start..].starts_with("{{{");
        let (opening_len, closing) = if raw { (3, "}}}") } else { (2, "}}") };
        let value_start = start + opening_len;
        let Some(close_offset) = content[value_start..].find(closing) else {
            result.push_str(&content[start..]);
            return result;
        };
        let value_end = value_start + close_offset;
        let end = value_end + closing.len();
        let key = content[value_start..value_end].trim();

        match resolve_json_path(variables, key) {
            Some(value) if matches!(mode, TemplateMode::Html) && !raw => {
                push_html_escaped(&mut result, &value);
            }
            Some(value) => result.push_str(&value),
            None => result.push_str(&content[start..end]),
        }
        cursor = end;
    }

    result.push_str(&content[cursor..]);
    result
}

/// Resolve a dot-notation path in a JSON value.
fn resolve_json_path(value: &serde_json::Value, path: &str) -> Option<String> {
    let mut current = value;
    for segment in path.split('.') {
        match current.get(segment) {
            Some(v) => current = v,
            None => return None,
        }
    }
    Some(match current {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    })
}

fn push_html_escaped(output: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#x27;"),
            _ => output.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replace_simple_variables() {
        let content = "Hello {{name}}, welcome to {{app}}!";
        let vars = json!({"name": "Alice", "app": "MyApp"});
        let result = replace_variables(content, &vars, TemplateMode::Text);
        assert_eq!(result, "Hello Alice, welcome to MyApp!");
    }

    #[test]
    fn replace_nested_variables() {
        let content = "Hello {{user.name}}!";
        let vars = json!({"user": {"name": "Bob"}});
        let result = replace_variables(content, &vars, TemplateMode::Text);
        assert_eq!(result, "Hello Bob!");
    }

    #[test]
    fn unmatched_variables_preserved() {
        let content = "Hello {{name}}, your {{unknown}} is here.";
        let vars = json!({"name": "Alice"});
        let result = replace_variables(content, &vars, TemplateMode::Text);
        assert_eq!(result, "Hello Alice, your {{unknown}} is here.");
    }

    #[test]
    fn whitespace_in_variable_names() {
        let content = "Hello {{ name }}!";
        let vars = json!({"name": "Alice"});
        let result = replace_variables(content, &vars, TemplateMode::Text);
        assert_eq!(result, "Hello Alice!");
    }

    #[test]
    fn html_variables_are_escaped_and_raw_variables_are_explicit() {
        let content = "<p>{{value}}</p><div>{{{trusted}}}</div>";
        let vars = json!({
            "value": "<script>alert(\"x\")</script> & '",
            "trusted": "<strong>safe</strong>"
        });

        let result = replace_variables(content, &vars, TemplateMode::Html);

        assert_eq!(
            result,
            "<p>&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; &#x27;</p><div><strong>safe</strong></div>"
        );
    }

    #[test]
    fn text_variables_remain_unescaped() {
        let content = "Value: {{value}} / {{{value}}}";
        let vars = json!({"value": "<hello> & goodbye"});

        let result = replace_variables(content, &vars, TemplateMode::Text);

        assert_eq!(result, "Value: <hello> & goodbye / <hello> & goodbye");
    }

    #[test]
    fn replacement_values_are_not_reinterpreted_as_placeholders() {
        let content = "{{first}} {{second}}";
        let vars = json!({"first": "{{second}}", "second": "resolved"});

        let result = replace_variables(content, &vars, TemplateMode::Text);

        assert_eq!(result, "{{second}} resolved");
    }

    #[test]
    fn renderer_uses_html_and_text_specific_escaping() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("welcome.html"), "<p>{{name}}</p>").unwrap();
        std::fs::write(dir.path().join("welcome.txt"), "Hello {{name}}").unwrap();
        let renderer = TemplateRenderer::new(dir.path());

        let rendered = renderer
            .render("welcome", &json!({"name": "<Admin>"}))
            .unwrap();

        assert_eq!(rendered.html.as_deref(), Some("<p>&lt;Admin&gt;</p>"));
        assert_eq!(rendered.text.as_deref(), Some("Hello <Admin>"));
    }

    #[test]
    fn template_names_reject_traversal_and_unsafe_paths() {
        let renderer = TemplateRenderer::new("templates/emails");

        assert!(renderer.render("../secret", &json!({})).is_err());
        assert!(!renderer.exists("../secret"));
        assert!(renderer.render("/absolute", &json!({})).is_err());
        assert!(renderer.render("auth\\welcome", &json!({})).is_err());
        assert!(renderer.render("auth//welcome", &json!({})).is_err());
    }

    #[test]
    fn nested_relative_template_names_are_allowed() {
        assert!(validate_template_name("auth/welcome").is_ok());
        assert!(validate_template_name("marketing/order_shipped").is_ok());
    }
}
