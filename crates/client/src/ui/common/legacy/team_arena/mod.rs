//! Team arena menu memory.

pub mod memory;

#[cfg(test)]
mod tests {
    use super::memory::UiStringReference;

    #[test]
    fn string_reference_round_trip() {
        let reference = UiStringReference::literal("ta").expect("literal");
        assert_eq!(reference.read().expect("read"), "ta");
    }
}
