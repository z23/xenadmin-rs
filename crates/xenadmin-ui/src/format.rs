//! Labels and orderings every area shares.

use std::cmp::Ordering;

/// A byte count as the Rune front end wrote it (`bytes` in `main.rs`):
/// zero is a dash, a gibibyte or more has one decimal, less is whole mebibytes.
pub fn memory_label(value: u64) -> String {
    if value == 0 {
        "—".into()
    } else if value >= 1 << 30 {
        format!("{:.1} GiB", value as f64 / (1u64 << 30) as f64)
    } else {
        format!("{:.0} MiB", value as f64 / (1u64 << 20) as f64)
    }
}

/// The Rune helper name for [`memory_label`].
pub fn bytes(value: u64) -> String {
    memory_label(value)
}

/// A storage size that may be unknown.
pub fn storage_bytes(value: Option<u64>) -> String {
    match value {
        None => "Unknown".into(),
        Some(0) => "0 B".into(),
        Some(value) => memory_label(value),
    }
}

/// Name order for lists and grids: runs of digits compare as numbers, other
/// characters compare without case, and names that are equal that way fall
/// back to plain order so the result is total. "vm 2" sorts before "vm 10".
///
/// The Rune table used `rune_kit::natural_cmp`; this is the model's own
/// implementation of the same order.
pub fn natural_cmp(left: &str, right: &str) -> Ordering {
    let mut a = left.chars().peekable();
    let mut b = right.chars().peekable();
    loop {
        let (Some(&x), Some(&y)) = (a.peek(), b.peek()) else {
            return match (a.peek(), b.peek()) {
                (None, None) => left.cmp(right),
                (None, Some(_)) => Ordering::Less,
                _ => Ordering::Greater,
            };
        };
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let m = digits(&mut a);
            let n = digits(&mut b);
            let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
            let order = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
            if order != Ordering::Equal {
                return order;
            }
        } else {
            let order = x.to_lowercase().cmp(y.to_lowercase());
            if order != Ordering::Equal {
                return order;
            }
            a.next();
            b.next();
        }
    }
}

fn digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut run = String::new();
    while let Some(&c) = chars.peek() {
        if !c.is_ascii_digit() {
            break;
        }
        run.push(c);
        chars.next();
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_labels_keep_the_desktop_wording() {
        assert_eq!(storage_bytes(None), "Unknown");
        assert_eq!(storage_bytes(Some(0)), "0 B");
        assert_eq!(memory_label(0), "—");
        assert_eq!(bytes(3 << 30), "3.0 GiB");
        assert_eq!(bytes(512 << 20), "512 MiB");
    }

    #[test]
    fn names_order_by_number_then_without_case() {
        let mut names = vec!["vm 10", "VM 2", "vm 1", "Alpha", "alpha", "vm 02", "vm"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            ["Alpha", "alpha", "vm", "vm 1", "VM 2", "vm 02", "vm 10"]
        );
        assert_eq!(natural_cmp("a", "a"), Ordering::Equal);
        assert_eq!(natural_cmp("a9", "a10"), Ordering::Less);
        assert_eq!(natural_cmp("b", "A"), Ordering::Greater);
    }
}
