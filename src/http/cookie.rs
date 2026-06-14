use axum::http::{header, HeaderMap, HeaderValue};

pub use axum_extra::extract::cookie::{Cookie, SameSite};
pub use axum_extra::extract::CookieJar;

use crate::foundation::{Error, Result};

/// Extract a cookie value by name from the `Cookie` request header.
pub fn extract_cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    if validate_cookie_name(name).is_err() {
        return None;
    }

    for cookie_header in headers.get_all(header::COOKIE).iter() {
        let Ok(cookie_header) = cookie_header.to_str() else {
            continue;
        };

        for part in cookie_header.split(';') {
            let Some((candidate, value)) = part.trim().split_once('=') else {
                continue;
            };
            if candidate.trim() != name {
                continue;
            }
            let value = unquote_cookie_value(value.trim());
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

pub(crate) fn parse_same_site(value: &str) -> Result<SameSite> {
    match value.trim().to_ascii_lowercase().as_str() {
        "lax" => Ok(SameSite::Lax),
        "strict" => Ok(SameSite::Strict),
        "none" => Ok(SameSite::None),
        _ => Err(Error::message(format!(
            "invalid cookie SameSite value `{value}`; expected lax, strict, or none"
        ))),
    }
}

pub(crate) fn build_cookie_header_value(options: CookieHeaderOptions<'_>) -> Result<HeaderValue> {
    validate_cookie_name(options.name)?;
    validate_cookie_value(options.value)?;
    validate_cookie_path(options.path)?;
    let domain = normalize_cookie_domain(options.domain)?;
    if matches!(options.same_site, SameSite::None) && !options.secure {
        return Err(Error::message("cookie SameSite=None requires Secure=true"));
    }

    let mut builder = Cookie::build((options.name, options.value))
        .same_site(options.same_site)
        .path(options.path);
    if options.http_only {
        builder = builder.http_only(true);
    }
    if options.secure {
        builder = builder.secure(true);
    }
    if let Some(domain) = domain {
        builder = builder.domain(domain);
    }

    let mut cookie = builder.build().to_string();
    if let Some(max_age_secs) = options.max_age_secs {
        cookie.push_str("; Max-Age=");
        cookie.push_str(&max_age_secs.to_string());
    }

    HeaderValue::from_str(&cookie)
        .map_err(|error| Error::message(format!("invalid Set-Cookie header value: {error}")))
}

pub(crate) fn clear_cookie_header_value(
    options: ClearCookieHeaderOptions<'_>,
) -> Result<HeaderValue> {
    validate_cookie_name(options.name)?;
    validate_cookie_path(options.path)?;
    let domain = normalize_cookie_domain(options.domain)?;
    if matches!(options.same_site, SameSite::None) && !options.secure {
        return Err(Error::message("cookie SameSite=None requires Secure=true"));
    }

    let mut builder = Cookie::build(options.name)
        .http_only(options.http_only)
        .same_site(options.same_site)
        .path(options.path);
    if options.secure {
        builder = builder.secure(true);
    }
    if let Some(domain) = domain {
        builder = builder.domain(domain);
    }
    let mut cookie = builder.build();
    cookie.make_removal();
    HeaderValue::from_str(&cookie.to_string())
        .map_err(|error| Error::message(format!("invalid Set-Cookie header value: {error}")))
}

pub(crate) struct CookieHeaderOptions<'a> {
    pub name: &'a str,
    pub value: &'a str,
    pub http_only: bool,
    pub secure: bool,
    pub path: &'a str,
    pub same_site: SameSite,
    pub domain: Option<&'a str>,
    pub max_age_secs: Option<u64>,
}

pub(crate) struct ClearCookieHeaderOptions<'a> {
    pub name: &'a str,
    pub http_only: bool,
    pub secure: bool,
    pub path: &'a str,
    pub same_site: SameSite,
    pub domain: Option<&'a str>,
}

fn unquote_cookie_value(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

fn validate_cookie_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::message("cookie name cannot be empty"));
    }
    if !name.bytes().all(is_cookie_token_byte) {
        return Err(Error::message(format!(
            "invalid cookie name `{name}`: names must be HTTP token characters"
        )));
    }
    Ok(())
}

fn validate_cookie_value(value: &str) -> Result<()> {
    if value
        .bytes()
        .any(|byte| byte.is_ascii_control() || byte == b';')
    {
        return Err(Error::message(
            "invalid cookie value: values cannot contain control characters or semicolons",
        ));
    }
    Ok(())
}

fn validate_cookie_path(path: &str) -> Result<()> {
    if !path.starts_with('/') {
        return Err(Error::message(format!(
            "invalid cookie path `{path}`: paths must start with `/`"
        )));
    }
    if path
        .bytes()
        .any(|byte| byte.is_ascii_control() || byte == b';')
    {
        return Err(Error::message(
            "invalid cookie path: paths cannot contain control characters or semicolons",
        ));
    }
    Ok(())
}

fn normalize_cookie_domain(domain: Option<&str>) -> Result<Option<&str>> {
    let Some(domain) = domain.map(str::trim).filter(|domain| !domain.is_empty()) else {
        return Ok(None);
    };
    if domain.bytes().any(|byte| {
        byte.is_ascii_control()
            || byte.is_ascii_whitespace()
            || matches!(byte, b';' | b'/' | b'\\' | b':')
    }) {
        return Err(Error::message(format!(
            "invalid cookie domain `{domain}`: domains cannot contain whitespace, path separators, ports, or control characters"
        )));
    }
    Ok(Some(domain))
}

fn is_cookie_token_byte(byte: u8) -> bool {
    byte.is_ascii_graphic()
        && !matches!(
            byte,
            b'(' | b')'
                | b'<'
                | b'>'
                | b'@'
                | b','
                | b';'
                | b':'
                | b'\\'
                | b'"'
                | b'/'
                | b'['
                | b']'
                | b'?'
                | b'='
                | b'{'
                | b'}'
        )
}

/// Helpers for building secure session cookies.
pub struct SessionCookie;

impl SessionCookie {
    /// Build a session cookie with secure defaults:
    /// HttpOnly, SameSite=Lax, Path=/, and optionally Secure.
    pub fn build<'a>(name: &'a str, value: &'a str, secure: bool) -> Cookie<'a> {
        Self::build_with_path(name, value, secure, "/")
    }

    pub(crate) fn build_with_path<'a>(
        name: &'a str,
        value: &'a str,
        secure: bool,
        path: &'a str,
    ) -> Cookie<'a> {
        let mut builder = Cookie::build((name, value))
            .http_only(true)
            .same_site(SameSite::Lax)
            .path(path);

        if secure {
            builder = builder.secure(true);
        }

        builder.build()
    }

    /// Build an expired removal cookie (clears the cookie on the client).
    pub fn clear(name: &str) -> Cookie<'_> {
        Self::clear_with_path(name, "/")
    }

    pub(crate) fn clear_with_path<'a>(name: &'a str, path: &'a str) -> Cookie<'a> {
        let mut cookie = Cookie::build(name)
            .http_only(true)
            .same_site(SameSite::Lax)
            .path(path)
            .build();
        cookie.make_removal();
        cookie
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_exact_cookie_name_across_multiple_headers() {
        let mut headers = HeaderMap::new();
        headers.append(
            header::COOKIE,
            "session_id=wrong; foundry=skip".parse().unwrap(),
        );
        headers.append(
            header::COOKIE,
            "foundry_session=\"abc_123\"; other=value".parse().unwrap(),
        );

        assert_eq!(
            extract_cookie_value(&headers, "foundry_session").as_deref(),
            Some("abc_123")
        );
        assert_eq!(extract_cookie_value(&headers, "session").as_deref(), None);
    }

    #[test]
    fn validates_cookie_header_options() {
        let value = build_cookie_header_value(CookieHeaderOptions {
            name: "foundry_session",
            value: "abc_123",
            http_only: true,
            secure: true,
            path: "/admin",
            same_site: SameSite::None,
            domain: Some("example.com"),
            max_age_secs: Some(60),
        })
        .unwrap();
        let value = value.to_str().unwrap();

        assert!(value.contains("HttpOnly"));
        assert!(value.contains("Secure"));
        assert!(value.contains("SameSite=None"));
        assert!(value.contains("Path=/admin"));
        assert!(value.contains("Domain=example.com"));
        assert!(value.contains("Max-Age=60"));
    }

    #[test]
    fn rejects_insecure_same_site_none_cookie() {
        let error = build_cookie_header_value(CookieHeaderOptions {
            name: "foundry_session",
            value: "abc_123",
            http_only: true,
            secure: false,
            path: "/",
            same_site: SameSite::None,
            domain: None,
            max_age_secs: None,
        })
        .unwrap_err();

        assert!(error.to_string().contains("SameSite=None requires Secure"));
    }
}
