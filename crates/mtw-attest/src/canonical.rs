//! Canonical JSON serialization for hash + signature stability.
//!
//! Cryptographic receipts are useless if signing the same logical payload
//! produces different bytes on different runs (or in different language
//! implementations). The MCP transport layer doesn't promise byte stability
//! — it just promises "JSON" — so we re-canonicalise before hashing.
//!
//! ## Rules
//!
//! Object keys: **sorted lexicographically by UTF-8 bytes**, recursively.
//! Numbers: serialized as JSON numbers, no exponential form for integers
//! that fit. Strings: UTF-8 with the standard JSON escaping. Arrays: order
//! preserved as-is (semantic order is the caller's problem).
//!
//! No whitespace. No trailing newline. No BOM.
//!
//! ## Why not pull in `serde_canonical_json` / `olpc-cjson` / etc.
//!
//! All the existing crates either drag a different number-formatting
//! convention (RFC 8785 vs. arbitrary) or have ABI baggage. The receipt
//! body is a tiny, fixed-shape struct — we'd rather own ~30 lines of
//! recursive sorting than add a dep with surprising edge cases.
//!
//! ## Cross-language interop
//!
//! TS and Rust implementations of `mtw-attest` MUST produce byte-identical
//! output for the same logical Value. The reference behaviour for tricky
//! cases:
//!   * Empty arrays / objects: `[]` / `{}` (no whitespace).
//!   * `null` is preserved (not stripped).
//!   * Numbers that round-trip through f64 use Rust's `{}` formatting
//!     (e.g. `1` not `1.0`); the TS side uses `JSON.stringify` on Number.
//!   * `NaN` / `Infinity` are not valid JSON; they will panic in `to_value`
//!     before reaching here.

use serde_json::Value;

/// Canonicalise a JSON value to bytes ready for hashing or signing.
pub fn canonicalize(value: &Value) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    write(&mut out, value);
    out
}

fn write(out: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write(out, item);
            }
            out.push(b']');
        }
        Value::Object(map) => {
            // Sort keys lexicographically — this is the whole point.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_string(out, k);
                out.push(b':');
                write(out, &map[*k]);
            }
            out.push(b'}');
        }
    }
}

/// JSON string escaping per RFC 8259. Matches `serde_json::to_string` for
/// the same input — we re-implement only because we need control over
/// where strings appear (object keys vs. values share the same escaping).
fn write_string(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    for c in s.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\u{08}' => out.extend_from_slice(b"\\b"),
            '\u{0C}' => out.extend_from_slice(b"\\f"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn primitives() {
        assert_eq!(canonicalize(&json!(null)), b"null");
        assert_eq!(canonicalize(&json!(true)), b"true");
        assert_eq!(canonicalize(&json!(false)), b"false");
        assert_eq!(canonicalize(&json!(42)), b"42");
        assert_eq!(canonicalize(&json!(3.14)), b"3.14");
        assert_eq!(canonicalize(&json!("hello")), b"\"hello\"");
    }

    #[test]
    fn keys_get_sorted() {
        let unsorted = json!({"z": 1, "b": 2, "a": 3});
        let sorted = json!({"a": 3, "b": 2, "z": 1});
        assert_eq!(canonicalize(&unsorted), canonicalize(&sorted));
        assert_eq!(canonicalize(&unsorted), br#"{"a":3,"b":2,"z":1}"#);
    }

    #[test]
    fn nested_objects_sort_recursively() {
        let v = json!({"outer": {"z": 1, "a": 2}});
        assert_eq!(canonicalize(&v), br#"{"outer":{"a":2,"z":1}}"#);
    }

    #[test]
    fn arrays_preserve_order() {
        let v = json!([3, 1, 2]);
        assert_eq!(canonicalize(&v), b"[3,1,2]");
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(canonicalize(&json!("a\"b")), b"\"a\\\"b\"");
        assert_eq!(canonicalize(&json!("line\nbreak")), b"\"line\\nbreak\"");
        assert_eq!(canonicalize(&json!("tab\there")), b"\"tab\\there\"");
    }

    #[test]
    fn empty_containers() {
        assert_eq!(canonicalize(&json!({})), b"{}");
        assert_eq!(canonicalize(&json!([])), b"[]");
    }

    #[test]
    fn unicode_passthrough() {
        // Non-ASCII printable characters are kept as UTF-8 bytes, not escaped.
        let v = json!("héllo 🌎");
        let bytes = canonicalize(&v);
        let expected = "\"héllo 🌎\"".as_bytes();
        assert_eq!(bytes, expected);
    }
}
