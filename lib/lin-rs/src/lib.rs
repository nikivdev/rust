//! lin-rs: crate for linear algebra helpers and utilities.

/// Placeholder to keep the crate compiling; replace with real functionality.
pub fn identity(x: i32) -> i32 {
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_returns_input() {
        assert_eq!(identity(42), 42);
    }
}
