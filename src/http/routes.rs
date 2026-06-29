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

        let params_by_key = params.iter().copied().collect::<HashMap<_, _>>();
        let mut url = pattern.clone();
        for param in super::route_path_params(pattern) {
            let value = params_by_key.get(param.as_str()).ok_or_else(|| {
                Error::message(format!(
                    "route '{}' is missing required parameter `{param}`",
                    name.as_str()
                ))
            })?;
            let encoded = percent_encode(value);
            url = replace_route_param(&url, &param, &encoded);
        }
        Ok(url)
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
    let mut replaced = String::with_capacity(url.len() + value.len());
    for (index, segment) in url.split('/').enumerate() {
        if index > 0 {
            replaced.push('/');
        }

        if route_segment_param(segment).is_some_and(|candidate| candidate == param) {
            replaced.push_str(value);
        } else {
            replaced.push_str(segment);
        }
    }
    replaced
}

pub(crate) fn route_segment_param(segment: &str) -> Option<&str> {
    if let Some(inner) = segment
        .strip_prefix('{')
        .and_then(|inner| inner.strip_suffix('}'))
    {
        Some(inner.strip_prefix('*').unwrap_or(inner))
    } else {
        segment.strip_prefix(':')
    }
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
    fn url_replaces_only_complete_legacy_param_segments() {
        let mut registry = RouteRegistry::new();
        registry.register(
            RouteId::new("tokens.show"),
            "/tokens/:id/identities/:identifier",
        );

        let url = registry
            .url(
                RouteId::new("tokens.show"),
                &[("id", "42"), ("identifier", "abc")],
            )
            .unwrap();

        assert_eq!(url, "/tokens/42/identities/abc");
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
