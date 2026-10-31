//! SHA-256 helpers, canonical JSON and the time source.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Lowercase hex of arbitrary bytes.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 15) as usize] as char);
    }
    out
}

/// SHA-256 of bytes, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Canonical JSON: object keys sorted, no insignificant whitespace. Used for every hash that must
/// be stable across machines and serde_json feature sets.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String((*key).clone()).to_string());
                out.push(':');
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// SHA-256 of the canonical JSON form.
pub fn canonical_hash(value: &Value) -> String {
    sha256_hex(canonical_json(value).as_bytes())
}

/// Time source, replaceable in tests.
pub trait Clock: Send + Sync {
    /// Seconds since the Unix epoch.
    fn now_secs(&self) -> i64;

    /// RFC 3339 UTC timestamp with second precision.
    fn now_rfc3339(&self) -> String {
        rfc3339_utc(self.now_secs())
    }
}

/// The wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_secs(&self) -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
    }
}

/// A clock that always returns the same instant.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now_secs(&self) -> i64 {
        self.0
    }
}

/// Format Unix seconds as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// Days since 1970-01-01 to a civil date (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (y + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn rfc3339_known_instants() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_793_404_800), "2026-10-31T00:00:00Z");
        assert_eq!(rfc3339_utc(1_793_404_800 + 3_661), "2026-10-31T01:01:01Z");
    }

    #[test]
    fn canonical_json_sorts_keys_and_is_stable() {
        let a = json!({"b": 1, "a": {"y": [1, 2], "x": null}});
        let b = json!({"a": {"x": null, "y": [1, 2]}, "b": 1});
        assert_eq!(canonical_json(&a), r#"{"a":{"x":null,"y":[1,2]},"b":1}"#);
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
    }
}
