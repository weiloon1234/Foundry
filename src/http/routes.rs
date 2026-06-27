use std::collections::HashMap;
use std::fmt::Write as _;

use crate::foundation::{Error, Result};
use crate::support::RouteId;

/// Registry of named routes mapping names to their path patterns.
///
/// Used for URL generation: `app.route_url(Route::UsersShow, &[("id", "123")])`.
#[derive(Clone, Debug, Default)]
pub struct RouteRegistry {
    routes: HashMap<RouteId, String>,
}

impl RouteRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a named route with its path pattern.
    pub fn register(&mut self, name: impl Into<RouteId>, pattern: impl Into<String>) {
        self.routes.insert(name.into(), pattern.into());
    }

    /// Generate a URL from a named route, replacing `:param`, `{param}`, and `{*param}` segments.
    ///
    /// ```ignore
    /// let url = registry.url(Route::UsersShow, &[("id", "123")])?;
    /// // Returns: "/api/v1/users/123"
    /// ```
    pub fn url<I>(&self, name: I, params: &[(&str, &str)]) -> Result<String>
    where
        I: Into<RouteId>,
    {
        let name = name.into();
        let pattern = self
            .routes
            .get(&name)
            .ok_or_else(|| Error::message(format!("route '{}' not found", name.as_str())))?;
        super::ensure_route_path_params_are_valid(pattern, &format!("route '{}'", name.as_str()))?;

        let mut url = pattern.clone();
        let route_params = super::route_path_params(pattern);
        ensure_unique_route_path_params(name.as_str(), params, &route_params)?;
        let params_by_key = params.iter().copied().collect::<HashMap<_, _>>();
        for param in &route_params {
            let value = params_by_key.get(param.as_str()).ok_or_else(|| {
                Error::message(format!(
                    "route '{}' is missing required parameter `{param}`",
                    name.as_str()
                ))
            })?;
            let encoded = if super::route_path_param_is_wildcard(pattern, param) {
                percent_encode_path(value)
            } else {
                percent_encode(value)
            };
            url = replace_route_param(&url, param, &encoded);
        }

        Ok(append_query_params(url, params, &route_params))
    }

    /// Check if a named route exists.
    pub fn has<I>(&self, name: I) -> bool
    where
        I: Into<RouteId>,
    {
        self.routes.contains_key(&name.into())
    }

    /// Iterate over all registered routes.
    pub fn iter(&self) -> impl Iterator<Item = (&RouteId, &String)> {
        self.routes.iter()
    }

    /// Generate a signed URL with HMAC-SHA256 signature and expiry timestamp.
    pub fn signed_url(
        &self,
        name: impl Into<RouteId>,
        params: &[(&str, &str)],
        signing_key: &[u8],
        expires_at: crate::support::DateTime,
    ) -> Result<String> {
        let name = name.into();
        let pattern = self
            .routes
            .get(&name)
            .ok_or_else(|| Error::message(format!("route '{}' not found", name.as_str())))?;
        super::ensure_route_path_params_are_valid(pattern, &format!("route '{}'", name.as_str()))?;
        let route_params = super::route_path_params(pattern);
        for (key, _) in params {
            if route_params.iter().any(|param| param == key) {
                continue;
            }
            if signed_url_query_param_is_reserved(key) {
                return Err(Error::message(format!(
                    "signed route '{}' cannot include reserved query parameter `{key}`; signed URLs append `expires` and `signature` internally",
                    name.as_str()
                )));
            }
        }

        let mut url = self.url(name, params)?;
        let expiry = expires_at.as_chrono().timestamp();
        let sep = if url.contains('?') { "&" } else { "?" };
        url = format!("{url}{sep}expires={expiry}");
        let signature = crate::support::hmac::hmac_sha256_hex(signing_key, &url);
        Ok(format!("{url}&signature={signature}"))
    }

    /// Verify a signed URL's signature and expiry.
    pub fn verify_signature(url: &str, signing_key: &[u8]) -> Result<()> {
        let signed = split_signed_url(url)?;

        let expected =
            crate::support::hmac::hmac_sha256_hex(signing_key, &signed.url_without_signature);

        if !crate::support::hmac::constant_time_eq(signed.signature.as_bytes(), expected.as_bytes())
        {
            return Err(Error::http(403, "invalid signature"));
        }

        let now = chrono::Utc::now().timestamp();
        if now > signed.expires {
            return Err(Error::http(403, "signed URL has expired"));
        }

        Ok(())
    }
}

fn replace_route_param(url: &str, param: &str, value: &str) -> String {
    let axum_param = format!("{{{param}}}");
    let axum_wildcard_param = format!("{{*{param}}}");
    let legacy_param = format!(":{param}");

    url.split('/')
        .map(|segment| {
            if segment == axum_param || segment == axum_wildcard_param || segment == legacy_param {
                value
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn percent_encode_path(value: &str) -> String {
    value
        .split('/')
        .map(percent_encode)
        .collect::<Vec<_>>()
        .join("/")
}

fn append_query_params(
    mut url: String,
    params: &[(&str, &str)],
    route_params: &[String],
) -> String {
    let query = params
        .iter()
        .filter(|(key, _)| !route_params.iter().any(|param| param == key))
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<_>>();

    if query.is_empty() {
        return url;
    }

    let separator = if url.contains('?') { '&' } else { '?' };
    url.push(separator);
    url.push_str(&query.join("&"));
    url
}

fn ensure_unique_route_path_params(
    route_name: &str,
    params: &[(&str, &str)],
    route_params: &[String],
) -> Result<()> {
    for route_param in route_params {
        let count = params
            .iter()
            .filter(|(key, _)| key == route_param)
            .take(2)
            .count();
        if count > 1 {
            return Err(Error::message(format!(
                "route '{route_name}' contains duplicate path parameter `{route_param}`; provide each path parameter once"
            )));
        }
    }

    Ok(())
}

fn signed_url_query_param_is_reserved(key: &str) -> bool {
    matches!(key, "expires" | "signature")
}

struct SignedUrlParts {
    url_without_signature: String,
    signature: String,
    expires: i64,
}

fn split_signed_url(url: &str) -> Result<SignedUrlParts> {
    let Some((base, query)) = url.split_once('?') else {
        return Err(Error::http(403, "missing signature"));
    };
    if query.is_empty() {
        return Err(Error::http(403, "missing signature"));
    }

    let params = query.split('&').collect::<Vec<_>>();
    let mut signature = None;
    let mut signature_index = None;
    let mut expires = None;
    let mut signature_count = 0;
    let mut expires_count = 0;

    for (index, param) in params.iter().enumerate() {
        let (key, value) = param.split_once('=').unwrap_or((param, ""));
        match key {
            "signature" => {
                signature_count += 1;
                signature_index = Some(index);
                if !is_valid_signature_value(value) {
                    return Err(Error::http(403, "invalid signature"));
                }
                signature = Some(value.to_string());
            }
            "expires" => {
                expires_count += 1;
                let parsed = value
                    .parse::<i64>()
                    .map_err(|_| Error::http(403, "invalid expires parameter"))?;
                expires = Some(parsed);
            }
            _ => {}
        }
    }

    if signature_count == 0 {
        return Err(Error::http(403, "missing signature"));
    }
    if signature_count > 1 || signature_index != Some(params.len() - 1) {
        return Err(Error::http(403, "invalid signature"));
    }
    if expires_count == 0 {
        return Err(Error::http(403, "missing expires parameter"));
    }
    if expires_count > 1 {
        return Err(Error::http(403, "invalid expires parameter"));
    }

    let signature_index = signature_index.expect("signature presence checked above");
    let query_without_signature = params[..signature_index].join("&");
    let url_without_signature = if query_without_signature.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{query_without_signature}")
    };

    Ok(SignedUrlParts {
        url_without_signature,
        signature: signature.expect("signature presence checked above"),
        expires: expires.expect("expires presence checked above"),
    })
}

fn is_valid_signature_value(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::{DateTime, RouteId};

    fn registry() -> RouteRegistry {
        let mut registry = RouteRegistry::new();
        registry.register(RouteId::new("reset"), "/reset/{token}");
        registry
    }

    fn signing_key() -> &'static [u8] {
        b"test-signing-key"
    }

    fn error_message(error: Error) -> String {
        error.to_string()
    }

    #[test]
    fn route_url_percent_encodes_like_generated_typescript_helpers() {
        let url = registry()
            .url(RouteId::new("reset"), &[("token", "AZaz09-_.~!'()* /")])
            .unwrap();

        assert_eq!(url, "/reset/AZaz09-_.~%21%27%28%29%2A%20%2F");
    }

    #[test]
    fn route_url_appends_non_path_params_as_query_params() {
        let url = registry()
            .url(
                RouteId::new("reset"),
                &[
                    ("token", "abc"),
                    ("next", "/dashboard"),
                    ("utm source", "email campaign"),
                ],
            )
            .unwrap();

        assert_eq!(
            url,
            "/reset/abc?next=%2Fdashboard&utm%20source=email%20campaign"
        );

        let url = registry()
            .url(
                RouteId::new("reset"),
                &[("token", "abc"), ("tag", "rust"), ("tag", "foundry")],
            )
            .unwrap();

        assert_eq!(url, "/reset/abc?tag=rust&tag=foundry");
    }

    #[test]
    fn route_url_rejects_duplicate_path_params() {
        let error = registry()
            .url(
                RouteId::new("reset"),
                &[("token", "abc"), ("token", "override")],
            )
            .expect_err("duplicate path params should fail");

        assert!(
            error_message(error).contains(
                "route 'reset' contains duplicate path parameter `token`; provide each path parameter once"
            ),
            "unexpected error"
        );
    }

    #[test]
    fn route_url_rejects_malformed_path_param_tokens() {
        let mut registry = RouteRegistry::new();
        registry.register(RouteId::new("users.show"), "/users/{}");

        let error = registry
            .url(RouteId::new("users.show"), &[])
            .expect_err("malformed route path params should fail");

        assert!(
            error_message(error)
                .contains("route 'users.show' contains invalid route path parameter segment `{}`"),
            "unexpected error"
        );
    }

    #[test]
    fn signed_url_signs_extra_query_params_before_signature() {
        let url = registry()
            .signed_url(
                RouteId::new("reset"),
                &[("token", "abc"), ("next", "/dashboard")],
                signing_key(),
                DateTime::now().add_days(1),
            )
            .unwrap();

        assert!(url.starts_with("/reset/abc?next=%2Fdashboard&expires="));
        RouteRegistry::verify_signature(&url, signing_key()).unwrap();
    }

    #[test]
    fn signed_url_rejects_reserved_extra_query_params_before_signing() {
        let registry = registry();

        for reserved in ["expires", "signature"] {
            let error = registry
                .signed_url(
                    RouteId::new("reset"),
                    &[("token", "abc"), (reserved, "override")],
                    signing_key(),
                    DateTime::now().add_days(1),
                )
                .expect_err("reserved signed URL query params should fail before signing");

            assert!(
                error_message(error).contains(&format!(
                    "cannot include reserved query parameter `{reserved}`"
                )),
                "unexpected error for {reserved}"
            );
        }

        let mut registry = RouteRegistry::new();
        registry.register(RouteId::new("token.expires"), "/tokens/{expires}");
        let url = registry
            .signed_url(
                RouteId::new("token.expires"),
                &[("expires", "abc")],
                signing_key(),
                DateTime::now().add_days(1),
            )
            .expect("path params named expires should not be treated as query params");

        assert!(url.starts_with("/tokens/abc?expires="));
        RouteRegistry::verify_signature(&url, signing_key()).unwrap();
    }

    #[test]
    fn signed_url_roundtrips_and_rejects_tampering() {
        let url = registry()
            .signed_url(
                RouteId::new("reset"),
                &[("token", "abc 123")],
                signing_key(),
                DateTime::now().add_days(1),
            )
            .unwrap();

        RouteRegistry::verify_signature(&url, signing_key()).unwrap();

        let tampered = url.replace("abc%20123", "abc%20456");
        let error = RouteRegistry::verify_signature(&tampered, signing_key()).unwrap_err();
        assert!(error_message(error).contains("invalid signature"));
    }

    #[test]
    fn signed_url_rejects_duplicate_signature_and_expires_params() {
        let url = registry()
            .signed_url(
                RouteId::new("reset"),
                &[("token", "abc")],
                signing_key(),
                DateTime::now().add_days(1),
            )
            .unwrap();

        let duplicate_signature = format!("{url}&signature=abc");
        let error =
            RouteRegistry::verify_signature(&duplicate_signature, signing_key()).unwrap_err();
        assert!(error_message(error).contains("invalid signature"));

        let duplicate_expires = url.replacen("expires=", "expires=1&expires=", 1);
        let error = RouteRegistry::verify_signature(&duplicate_expires, signing_key()).unwrap_err();
        assert!(error_message(error).contains("invalid expires parameter"));
    }

    #[test]
    fn signed_url_rejects_unsigned_query_after_signature() {
        let url = registry()
            .signed_url(
                RouteId::new("reset"),
                &[("token", "abc")],
                signing_key(),
                DateTime::now().add_days(1),
            )
            .unwrap();

        let error = RouteRegistry::verify_signature(&format!("{url}&admin=true"), signing_key())
            .unwrap_err();

        assert!(error_message(error).contains("invalid signature"));
    }

    #[test]
    fn signed_url_rejects_missing_invalid_and_expired_values() {
        let url = registry()
            .signed_url(
                RouteId::new("reset"),
                &[("token", "abc")],
                signing_key(),
                DateTime::now().sub_days(1),
            )
            .unwrap();
        let error = RouteRegistry::verify_signature(&url, signing_key()).unwrap_err();
        assert!(error_message(error).contains("signed URL has expired"));

        let error =
            RouteRegistry::verify_signature("/reset/abc?expires=1", signing_key()).unwrap_err();
        assert!(error_message(error).contains("missing signature"));

        let error = RouteRegistry::verify_signature(
            "/reset/abc?expires=nope&signature=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            signing_key(),
        )
        .unwrap_err();
        assert!(error_message(error).contains("invalid expires parameter"));

        let error =
            RouteRegistry::verify_signature("/reset/abc?expires=1&signature=abc", signing_key())
                .unwrap_err();
        assert!(error_message(error).contains("invalid signature"));
    }
}
