use qa_console::{numbers::number, views::RuleSetId};

#[test]
fn quake_hex_decoding_keeps_original_float_arithmetic() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for (text, expected) in [
            ("0x2bd03d5ffffffff", 0x5c2f40f5),
            ("0x2BD03D5FFFFFFFF", 0x5c2f40f5),
            ("-0x2bd03d5ffffffff", 0xdc2f40f5),
            ("0x0", 0),
            ("-0X0", 0x80000000),
            ("0xfFtail", 255.0f32.to_bits()),
        ] {
            assert_eq!(number(text, rules).to_bits(), expected, "{text}");
        }
    }
}
