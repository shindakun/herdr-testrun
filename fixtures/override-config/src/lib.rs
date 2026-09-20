//! Fixture: the config file replaces the run command. The override keeps
//! libtest's output shape so the cargo adapter still parses it.

pub fn answer() -> u32 {
    41
}

#[cfg(test)]
mod tests {
    #[test]
    fn answer_is_42() {
        assert_eq!(super::answer(), 42);
    }
}
