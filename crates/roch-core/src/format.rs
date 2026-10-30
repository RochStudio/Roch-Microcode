//! Small formatting helpers reproducing the .NET format strings used in the UI and reports.

/// `{value:N0}` with en-US grouping, e.g. `212,992`.
pub fn n0(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if value < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// Custom format `0.#`, `0.##` ...: at most `max_decimals`, trailing zeros removed.
pub fn decimals(value: f64, max_decimals: usize) -> String {
    let mut s = format!("{value:.max_decimals$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

/// `Guid.NewGuid().ToString("N")`: 32 lowercase hex digits.
pub fn guid_n() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// `DateTime.Now:yyyyMMdd-HHmmss`.
pub fn local_timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Lowercase SHA-256 hex (`Convert.ToHexStringLower(SHA256.HashData(...))`).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex_lower(&Sha256::digest(bytes))
}

pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 15) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grouping() {
        assert_eq!(n0(0), "0");
        assert_eq!(n0(999), "999");
        assert_eq!(n0(1000), "1,000");
        assert_eq!(n0(212992), "212,992");
        assert_eq!(n0(33558528), "33,558,528");
        assert_eq!(n0(-1234), "-1,234");
    }
    #[test]
    fn custom_decimals() {
        assert_eq!(decimals(206.0, 1), "206");
        assert_eq!(decimals(207.5, 1), "207.5");
        assert_eq!(decimals(32.003, 2), "32");
        assert_eq!(decimals(32.5, 2), "32.5");
        assert_eq!(decimals(14.91, 1), "14.9");
    }
    #[test]
    fn guid_shape() {
        let g = guid_n();
        assert_eq!(g.len(), 32);
        assert!(g.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }
    #[test]
    fn sha256_known_vector() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
