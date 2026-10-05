//! What the command line shares with its tests.

/// A size on the command line: bytes, or with a unit like `64M` or `1G`.
pub fn parse_size(text: &str) -> Result<u64, String> {
    let t = text.trim();
    let (number, unit) = match t.find(|c: char| c.is_ascii_alphabetic()) {
        Some(i) => (&t[..i], t[i..].to_ascii_lowercase()),
        None => (t, String::new()),
    };
    let scale: u64 = match unit.trim_end_matches(['b', 'i']) {
        "" => 1,
        "k" => 1 << 10,
        "m" => 1 << 20,
        "g" => 1 << 30,
        _ => return Err(format!("can't read the size {text:?}: write it like 64M or 1G")),
    };
    let n: u64 = number
        .trim()
        .parse()
        .map_err(|_| format!("can't read the size {text:?}: write it like 64M or 1G"))?;
    n.checked_mul(scale)
        .ok_or_else(|| format!("the size {text:?} is too large"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_have_units() {
        assert_eq!(parse_size("1024"), Ok(1024));
        assert_eq!(parse_size("64M"), Ok(64 << 20));
        assert_eq!(parse_size("64MiB"), Ok(64 << 20));
        assert_eq!(parse_size("2g"), Ok(2 << 30));
        assert_eq!(parse_size("500k"), Ok(500 << 10));
        assert!(parse_size("lots").is_err());
        assert!(parse_size("5T").is_err());
    }
}
