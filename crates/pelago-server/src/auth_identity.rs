use axum::http::HeaderMap;
use pelago_api::{AuthPrincipal, AuthRuntime};
use sha2::{Digest, Sha256};
use tonic::Request;

pub fn resolve_principal_from_http_headers(
    runtime: &AuthRuntime,
    mtls_subject_header: &str,
    headers: &HeaderMap,
) -> Option<AuthPrincipal> {
    if let Some(fingerprint) = header_value(headers, "x-mtls-fingerprint") {
        if let Some(principal) = runtime.principal_for_mtls_fingerprint(&fingerprint) {
            return Some(principal);
        }
    }

    if let Some(subject) = header_value(headers, mtls_subject_header) {
        if let Some(principal) = runtime.principal_for_mtls_subject(&subject) {
            return Some(principal);
        }
    }

    if let Some(api_key) = header_value(headers, "x-api-key") {
        if let Some(principal) = runtime.principal_for_api_key(&api_key) {
            return Some(principal);
        }
    }

    if let Some(auth) = header_value(headers, "authorization") {
        let token = auth.strip_prefix("Bearer ").unwrap_or(&auth).trim();
        if !token.is_empty() {
            if let Some(principal) = runtime.validate_bearer_token_blocking(token) {
                return Some(principal);
            }
        }
    }

    None
}

pub fn resolve_principal_from_tonic_request(
    runtime: &AuthRuntime,
    mtls_subject_header: &str,
    req: &Request<()>,
) -> Option<AuthPrincipal> {
    if let Some(fingerprint) = request_peer_cert_fingerprint(req) {
        if let Some(principal) = runtime.principal_for_mtls_fingerprint(&fingerprint) {
            return Some(principal);
        }
    }

    if let Some(subject) = req
        .metadata()
        .get(mtls_subject_header)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(principal) = runtime.principal_for_mtls_subject(subject) {
            return Some(principal);
        }
    }

    if let Some(key) = req
        .metadata()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
    {
        if let Some(principal) = runtime.principal_for_api_key(key) {
            return Some(principal);
        }
    }

    if let Some(auth) = req
        .metadata()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
    {
        let token = auth.strip_prefix("Bearer ").unwrap_or(auth).trim();
        if !token.is_empty() {
            if let Some(principal) = runtime.validate_bearer_token_blocking(token) {
                return Some(principal);
            }
        }
    }

    None
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn request_peer_cert_fingerprint(req: &Request<()>) -> Option<String> {
    let certs = req.peer_certs()?;
    let first = certs.first()?;
    Some(sha256_fingerprint(first.as_ref()))
}

fn sha256_fingerprint(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::from("sha256:");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_value_empty_returns_none() {
        let headers = HeaderMap::new();
        assert!(header_value(&headers, "x-api-key").is_none());
    }
}
