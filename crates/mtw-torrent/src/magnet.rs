//! Tiny helpers to parse `magnet:?xt=urn:btih:...&dn=...` URIs without
//! pulling another dependency. Shared between `mock` and
//! `librqbit_engine` so synthetic-detail construction is consistent.

/// Extract a 40-hex infohash from a `magnet:?xt=urn:btih:HASH&...` URI.
/// Returns lowercase hex on success. Rejects 32-char base32 forms; the
/// caller is expected to normalise to hex first.
pub(crate) fn extract_infohash(magnet: &str) -> Option<String> {
    let xt_marker = "xt=urn:btih:";
    let idx = magnet.find(xt_marker)?;
    let rest = &magnet[idx + xt_marker.len()..];
    let end = rest.find('&').unwrap_or(rest.len());
    let raw = &rest[..end];
    if raw.len() == 40 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(raw.to_ascii_lowercase())
    } else {
        None
    }
}

/// Extract `&dn=` (display name) from a magnet URI, URL-decoded.
pub(crate) fn extract_name(magnet: &str) -> Option<String> {
    let idx = magnet.find("&dn=").or_else(|| magnet.find("?dn="))?;
    let rest = &magnet[idx + 4..];
    let end = rest.find('&').unwrap_or(rest.len());
    Some(urlencoding_decode(&rest[..end]))
}

/// Minimal `application/x-www-form-urlencoded` decoder. `+` → space,
/// `%XX` → byte. Anything malformed passes through verbatim.
pub(crate) fn urlencoding_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut iter = s.chars();
    while let Some(c) = iter.next() {
        if c == '+' {
            out.push(' ');
        } else if c == '%' {
            let h1 = iter.next();
            let h2 = iter.next();
            if let (Some(a), Some(b)) = (h1, h2) {
                if let Ok(byte) = u8::from_str_radix(&format!("{}{}", a, b), 16) {
                    out.push(byte as char);
                    continue;
                }
            }
            out.push(c);
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_basic_infohash() {
        let m = format!(
            "magnet:?xt=urn:btih:{}&dn=Sintel.mp4",
            "a".repeat(40)
        );
        assert_eq!(extract_infohash(&m).unwrap(), "a".repeat(40));
    }

    #[test]
    fn extract_uppercase_normalises() {
        let m = format!("magnet:?xt=urn:btih:{}", "A".repeat(40));
        assert_eq!(extract_infohash(&m).unwrap(), "a".repeat(40));
    }

    #[test]
    fn extract_invalid_returns_none() {
        assert!(extract_infohash("magnet:?xt=urn:btih:short").is_none());
        assert!(extract_infohash("not a magnet").is_none());
    }

    #[test]
    fn extract_dn_decodes() {
        let m = format!(
            "magnet:?xt=urn:btih:{}&dn=Big%20Buck%20Bunny+%282008%29",
            "f".repeat(40)
        );
        assert_eq!(extract_name(&m).as_deref(), Some("Big Buck Bunny (2008)"));
    }

    #[test]
    fn extract_dn_missing() {
        let m = format!("magnet:?xt=urn:btih:{}&tr=udp://t", "f".repeat(40));
        assert_eq!(extract_name(&m), None);
    }
}
