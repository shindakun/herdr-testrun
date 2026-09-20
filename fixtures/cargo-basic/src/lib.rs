//! Fixture: one passing unit test, one failing assertion, one panic in an
//! integration test.

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Wrong on purpose: drops the first byte.
pub fn roundtrip(s: &str) -> String {
    s.chars().skip(1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds() {
        assert_eq!(add(1, 2), 3);
    }

    #[test]
    fn roundtrip_keeps_input() {
        assert_eq!(roundtrip("abc"), "abc");
    }
}
