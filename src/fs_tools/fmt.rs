//! Small formatting helpers shared by the filesystem tools.

use crate::params::Range;

/// `1234` -> `1.2K`.
pub fn human_size(n: u64) -> String {
    const UNITS: [&str; 4] = ["K", "M", "G", "T"];
    if n < 1024 {
        return format!("{n}B");
    }
    let mut value = n as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 10.0 {
        format!("{value:.0}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

/// `drwxr-xr-x`-style string; `kind` is `d`, `l` or `-`.
pub fn perms(mode: u32, kind: char) -> String {
    let mut out = String::with_capacity(10);
    out.push(kind);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        out.push(if bits & 4 != 0 { 'r' } else { '-' });
        out.push(if bits & 2 != 0 { 'w' } else { '-' });
        out.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    out
}

/// Cut `line` to `max` characters, noting how many were dropped.
pub fn clip(line: &str, max: usize) -> String {
    match line.char_indices().nth(max) {
        None => line.to_string(),
        Some((cut, _)) => format!("{}…[+{} chars]", &line[..cut], line[cut..].chars().count()),
    }
}

/// `value` or `default`, limited to `lo..=hi`.
pub fn bounded(value: Option<u64>, default: u64, lo: u64, hi: u64) -> usize {
    value.unwrap_or(default).clamp(lo, hi) as usize
}

/// Footer for a page that does not reach the end of the results. `len` is the page size,
/// used for the `entries` value of the next page.
pub fn more_footer(offset: usize, shown: usize, total: usize, len: usize) -> Option<String> {
    (offset + shown < total).then(|| {
        format!(
            "[{}..{} of {total}; next entries={}]",
            offset + 1,
            offset + shown,
            Range::next(offset + shown, len)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512B");
        assert_eq!(human_size(1536), "1.5K");
        assert_eq!(human_size(20 * 1024 * 1024), "20M");
    }

    #[test]
    fn clipping() {
        assert_eq!(clip("abc", 5), "abc");
        assert_eq!(clip("abcdef", 3), "abc…[+3 chars]");
        assert_eq!(clip("ééé", 2), "éé…[+1 chars]");
    }

    #[test]
    fn permissions() {
        assert_eq!(perms(0o755, 'd'), "drwxr-xr-x");
        assert_eq!(perms(0o640, '-'), "-rw-r-----");
    }
}
