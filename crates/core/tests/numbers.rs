use qa_core::text::atof_prefix;

#[test]
fn decimal_integer_scan_preserves_end_pointer_and_native_widths() {
    use qa_core::text::{atoi_prefix, integer_prefix};
    for (bytes, value, end) in [
        (b" \t-12tail".as_slice(), -12, 5),
        (b"+", 0, 0),
        (b"  x", 0, 0),
        (b"0x10", 0, 1),
        (b"4294967296}", 4294967296, 10),
        (b"9223372036854775808", i64::MAX, 19),
        (b"-9223372036854775809", i64::MIN, 20),
    ] {
        assert_eq!(integer_prefix(bytes), (value, end));
        assert_eq!(
            atoi_prefix(bytes),
            value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
        );
    }
}

#[test]
fn decimal_scan_retains_prefix_and_incomplete_exponent_boundaries() {
    use qa_core::text::decimal_prefix_end;
    for (bytes, start, expected) in [
        (b"".as_slice(), 0, None),
        (b".".as_slice(), 0, None),
        (b".5e+tail".as_slice(), 0, Some(2)),
        (b"12.e-3tail".as_slice(), 0, Some(6)),
        (b"-12E+3tail".as_slice(), 1, Some(6)),
        (b"1e".as_slice(), 0, Some(1)),
        (b"0x1".as_slice(), 0, Some(1)),
        (b"Infinity".as_slice(), 0, None),
        (b"1".as_slice(), 2, None),
    ] {
        assert_eq!(decimal_prefix_end(bytes, start), expected);
    }
    for byte in 0..=u8::MAX {
        assert_eq!(
            qa_core::text::hex_digit(byte).map(u32::from),
            (byte as char).to_digit(16)
        );
    }
}

// C-locale glibc strtod results, including its end pointer. These fixtures
// cover prefix fallback and rounding at normal/subnormal/overflow boundaries.
#[test]
fn prefix_and_binary64_rounding_match_glibc() {
    let cases: &[(&str, u64, usize)] = &[
        ("", 0x0000000000000000, 0),
        (" ", 0x0000000000000000, 0),
        (".", 0x0000000000000000, 0),
        ("-", 0x0000000000000000, 0),
        ("\x0b-.5tail", 0xbfe0000000000000, 4),
        (" +3.25 x", 0x400a000000000000, 6),
        ("nan", 0x7ff8000000000000, 3),
        ("-NaN", 0xfff8000000000000, 4),
        ("NaN(7)", 0x7ff8000000000007, 6),
        ("nan(0x1234)", 0x7ff8000000001234, 11),
        ("nan(077)", 0x7ff800000000003f, 8),
        ("nan(thing)", 0x7ff8000000000000, 10),
        ("nan(7", 0x7ff8000000000000, 3),
        ("nan(+7)", 0x7ff8000000000000, 3),
        ("nan(9999999999999999999999999999)", 0x7fffffffffffffff, 33),
        ("infinity", 0x7ff0000000000000, 8),
        ("-INFinitytail", 0xfff0000000000000, 9),
        ("infinit", 0x7ff0000000000000, 3),
        ("0x", 0x0000000000000000, 1),
        ("-0x.", 0x8000000000000000, 2),
        ("0x.p0", 0x0000000000000000, 1),
        ("0x.1", 0x3fb0000000000000, 4),
        ("0x1p", 0x3ff0000000000000, 3),
        ("0x1p+tail", 0x3ff0000000000000, 3),
        ("0x1p-4tail", 0x3fb0000000000000, 6),
        ("-0.0", 0x8000000000000000, 4),
        ("1e+suffix", 0x3ff0000000000000, 1),
        ("1e999", 0x7ff0000000000000, 5),
        ("-1e-999", 0x8000000000000000, 7),
        (
            "1.00000000000000011102230246251565404236316680908203125",
            0x3ff0000000000000,
            55,
        ),
        (
            "1.00000000000000011102230246251565404236316680908203126",
            0x3ff0000000000001,
            55,
        ),
        ("0x1p-1075", 0x0000000000000000, 9),
        ("0x1.0000000000000001p-1075", 0x0000000000000001, 26),
        ("0x1.0000000000000000p-1075", 0x0000000000000000, 26),
        ("0x1p-1074", 0x0000000000000001, 9),
        ("0x1.fffffffffffffp-1023", 0x0010000000000000, 23),
        ("0x1.fffffffffffff8p1023", 0x7ff0000000000000, 23),
        ("0x1.fffffffffffff7p1023", 0x7fefffffffffffff, 23),
        ("0x0.fffffffffffff8p-1022", 0x0010000000000000, 24),
        ("0x1.fffffffffffff8p-1023", 0x0010000000000000, 24),
        (
            "0x0.00000000000008000000000000000001p-1022",
            0x0000000000000001,
            42,
        ),
        (
            "0x1.ffffffffffffffffffffffffffffffffp-1075",
            0x0000000000000001,
            42,
        ),
        ("0Xabc.defP+2", 0x40c579bde0000000, 12),
        ("12é", 0x4028000000000000, 2),
        ("é12", 0x0000000000000000, 0),
        (
            " -0x8a7c575e0cbb82cbdb93e8f48195ef6bc5e0c397acc53cd93de7e56616.d5f71fe8ed8a9c19e3186f8cb2p-457tail",
            0xb1d14f8aebc19770,
            94,
        ),
        ("0xa.3a8b1e823p362tail", 0x56c475163d046000, 17),
        (
            "0x7dcd929f32ee63788537.36bff0fp596tail",
            0x6a1f7364a7ccbb99,
            34,
        ),
        (
            " -0xf853e1879c.4dbde21e53299a26facf875a01b266f7fc13af86dd9285b4b0cp-427tail",
            0xa7bf0a7c30f389b8,
            71,
        ),
        ("0x.b9cp-1109tail", 0x0000000000000000, 12),
        ("0x6f.c1p450tail", 0x5c7bf04000000000, 11),
        (" -0x087f47212a64093987.p-1153tail", 0x8000000000000000, 29),
        (
            "0xbf6f11482973d25de0e.80fed83ae786f426af71a8b46950d1372398e413fbdd1973d03a6b8ff3a625dc1b2376p-1072tail",
            0x01a7ede229052e7a,
            98,
        ),
        (
            "0x57f0c3815669ccab31df08d72e21e4a8e6e9d1944fbc5878d80ded6ef4093a4ca60154591.5cp-1003tail",
            0x1365fc30e0559a73,
            84,
        ),
        (" -0x81c8f45413e5eec.4f50p925tail", 0xfd70391e8a827cbe, 28),
        ("0x.ea6dp-1086tail", 0x0000000000000000, 13),
        (
            "0x30013e6b4764a3699f158863f54112334c19b83bfe6f4.f277f30a554a82260fdap-745tail",
            0x1c78009f35a3b252,
            73,
        ),
        (
            " -0xe3ebd9d89e91e4b5c3bae1.2d391p-951tail",
            0x89fc7d7b3b13d23d,
            37,
        ),
        (
            "0x9f0a7b8694964679cb43.a1adf87c3e0dc8eb1ec30ep-337tail",
            0x2fd3e14f70d292c9,
            50,
        ),
        (
            "0x94f2df19d2944781b22db97be2.a4cbp1041tail",
            0x7ff0000000000000,
            38,
        ),
        (
            " -0x325760.1ee3c15d28053342c67990a49063bb7d26313bf390774d7696a5f25d394811c4a197a4c0c14db326a34p-1122tail",
            0x8000000000000000,
            100,
        ),
        (
            "0x1d6740e7081f61e82a0e2b512af91.b9e02f481ce32d1c58bfc487cbc9ddap1015tail",
            0x7ff0000000000000,
            68,
        ),
        ("0xf5.4f2be49abp914tail", 0x798ea9e57c935600, 18),
        (
            " -0x7852dbdf.48c3835771e9cac116d7b80d06c8319efbb6f6fd368bbe9e6cb56bee36fd23p-662tail",
            0x987e14b6f7d230e1,
            80,
        ),
        (
            "0x1691c2ba0cee3fb69824a212ff40454a58766d10.d874f1f488579c52323af90d68456b7e86b50p543tail",
            0x6ba691c2ba0cee40,
            84,
        ),
        (
            "0x677b6fc3957e34328f4a7fa476259.7ff8f0d6db898c91c69051954f28a768f1acp-32tail",
            0x4519dedbf0e55f8d,
            72,
        ),
        (
            " -0x867f74de.7a14914025e362d433e51e83b5f95d8d0143a38d4d4p631tail",
            0xe950cfee9bcf4292,
            60,
        ),
        ("0xa07117.eca3ed565p-121tail", 0x39d40e22fd947dab, 23),
        (
            "0x110e2b7c02f974a9c6d.1fc01a2p-505tail",
            0x24e10e2b7c02f975,
            34,
        ),
        (
            " -0xb42860afa9f24363956f0e1c76b611a5e6b05bc5.c2bcafe37e97bc036a0be0da8p-265tail",
            0xb956850c15f53e48,
            75,
        ),
        ("0xa22e3.53f3582ca794p1034tail", 0x7ff0000000000000, 25),
        (
            "0x492553fda5.48bd3d3c002c063cfd6a3873057b360ac204d6984p493tail",
            0x61224954ff69522f,
            58,
        ),
        (
            " -0x5.2ef617df711e6c7903bce5450d545fap-1017tail",
            0x8084bbd85f7dc47a,
            43,
        ),
        (
            "0x7f.3f107e21b636e68ec42972d1e10b6a08ap678tail",
            0x6abfcfc41f886d8e,
            42,
        ),
        (
            "0x3904435082c.183524a8c8e2e34ddcf8d3fe47ff246a9c1c9a19237d6dd98e2f3f9p-969tail",
            0x05fc8221a84160c2,
            74,
        ),
        (
            " -0x54a1f64e891395d3329a15.f7c1a714179496e46f11b38p-481tail",
            0xa745287d93a244e5,
            55,
        ),
        ("0x158.99bac4e7e11dd7e2b86p941tail", 0x7b45899bac4e7e12, 29),
        (
            "0x13d37a26edf3.6bee1f5ded1p1089tail",
            0x7ff0000000000000,
            31,
        ),
        (
            " -0x21c5dba1a55aeba1392ba1f2324b845.15a48563a6008961b5e7044d28a10ecebd1f5acbbca4f1e7efd002084p940tail",
            0xfff0000000000000,
            97,
        ),
        ("0xa2.2399ap1108tail", 0x7ff0000000000000, 15),
        (
            "0x20c109f71a9e1f975a836eb22.2f04bp391tail",
            0x5e706084fb8d4f10,
            37,
        ),
        (
            " -0x9b614ae18ff4fb3e44bb17c6dd51.935c8e9ee6d54eb3b9f35137b44af7fap-1010tail",
            0x87c36c295c31fe9f,
            71,
        ),
        (
            "0x0566299.3574906a710af89624b918430722f75e33e029febfda77p-1095tail",
            0x0000000000000003,
            62,
        ),
        ("0xf.p319tail", 0x541e000000000000, 8),
        (
            " -0x0c1542c35cdca7558.bc48778db52f422b7d5bf4b297c5c42fp-989tail",
            0x86182a8586b9b94f,
            59,
        ),
        (
            "0xc800.f23a1c23f677574bb43ac3f85ap1111tail",
            0x7ff0000000000000,
            38,
        ),
        (
            "0x551592dc360de0d5442fb0a5bb7c94ad752ce.3d9d31a0c7e83aafb0fee31efc244c06bab5555358cp130tail",
            0x51354564b70d8378,
            87,
        ),
        (
            " -0xa1af232da5453ed77c73bf5.bfae46ec1eb02p-1027tail",
            0x857435e465b4a8a8,
            47,
        ),
        (
            "0x6affb308c2998e52aa406d54b94.a7651bbb643cc31749be2b41p-886tail",
            0x0f3abfecc230a664,
            59,
        ),
        ("0x00ed0301.48d05p180tail", 0x4cada060291a0a00, 20),
        (
            " -0xf4af3de05484aa41134.e439c695d4e3ac0eeb644p-179tail",
            0xb97e95e7bc0a9095,
            50,
        ),
        (
            "0xf2df4dda6795a932614fb83da3d6f1.2cf1bae034772da16ca25e81p-571tail",
            0x23be5be9bb4cf2b5,
            62,
        ),
        ("0xba64634.528c5606fc42p138tail", 0x4a474c8c68a518ac, 26),
        (
            " -0xe2fdd1d3705654af6624b825b9.f8d306460710ce54d15c85fa44402efd98f722d408d2ap-409tail",
            0xacdc5fba3a6e0acb,
            81,
        ),
        (
            "0x5477703c5b5853b6beb282f89535303d02a.p-865tail",
            0x12851ddc0f16d615,
            43,
        ),
        (
            "0x2757e274ce484c729fd9884b27a5df68f19.242c7f840371fc1af2575ec8p777tail",
            0x7913abf13a672426,
            66,
        ),
        (
            " -0xacaf0d8afdd1386ff1a1f9ff8aae2ac67e5658ac.25e47457fd31d7e029b08e7f61829df06p-1009tail",
            0x8ad595e1b15fba27,
            84,
        ),
        (
            "0xc355d1dfe4dd7aac8468ac93bdab18c9fea19.85570336c4da58p-429tail",
            0x2e586aba3bfc9baf,
            59,
        ),
        (
            "0x6a9374d890d.30df8213e1746fap-145tail",
            0x398aa4dd362434c3,
            34,
        ),
        (
            " -0x4c1ab9d1a9306b3ac4cb01b60cc1b2e89b0647693f7adf472e4476528851beb5ca58d675188c56ec1e6ce24d.3p-928tail",
            0x9bd306ae746a4c1b,
            99,
        ),
        (
            "0xa12bfcb4bba3c19b356.90d51048eea2bp-57tail",
            0x4114257f96977478,
            39,
        ),
        ("0x.3a1a388d8102847e70p-943tail", 0x04dd0d1c46c08142, 26),
        (
            " -0xcf3716681d14360aca89d40f0df5b7a19a65e40c3d3c882e1fae9703ebdc2.5a3a414f9p-28tail",
            0xcd69e6e2cd03a287,
            79,
        ),
        (
            "0xf4b816d4d8c76211a735.4baf3ce60ad131c2487f6875b3316dc672d739p219tail",
            0x529e9702da9b18ec,
            65,
        ),
        (
            "0xa274567c0fbeb26a8c61f373d7.ddad99ce64032eb65p-7tail",
            0x45f44e8acf81f7d6,
            49,
        ),
        (
            " -0x57a9848e1fccbb77d13a12fc13f0c025d8231fcf5f8.8f7013ba5b0b7f8p-237tail",
            0xbbc5ea612387f32f,
            68,
        ),
        (
            "0x342c6f0a5b2ed149dcd12a73080f8388c83.08638308f93689a5ee01570e1fe53b2f0e51fap-310tail",
            0x352a1637852d9769,
            81,
        ),
        ("0xca94.86a7be459b0323fc8p-424tail", 0x26695290d4f7c8b3, 29),
        (" -0x0f5b8cc.0491d1d4d4p1039tail", 0xfff0000000000000, 27),
        (
            "0x389233933fd921aa94.0d6ff44874272160p362tail",
            0x5aec4919c99fec91,
            41,
        ),
        (
            "0x3ad14eab95b9292.0c99f7b1e5ffb725723d359f518887443b110f2143cd0fp-393tail",
            0x2afd68a755cadc95,
            69,
        ),
        (
            " -0xaea16e55aba8b7528c979efc5af2cb618ef937b2766931ad47bfd12892a0.eb1b1p317tail",
            0xe2b5d42dcab57517,
            74,
        ),
        (
            "0xd035a7b4dcfef4d25ca1d6bbda70877edcfb9bcd.5be346bp566tail",
            0x6d4a06b4f69b9fdf,
            54,
        ),
        (
            "0xf51932e0c984c832a5a0b9e6c953a.208a8c18p87tail",
            0x4c9ea3265c193099,
            43,
        ),
        (
            " -0x.200f4b2a2fb5ea819cebba917949cc4d87c992a09fc65733f34a55ead41fc7746dd719p-275tail",
            0xae9007a59517daf5,
            80,
        ),
        (
            "0x6a2b30c436b903eef635ffbca2145806b1b6cc870a49237bfc.2e0c349bbp142tail",
            0x553a8acc310dae41,
            66,
        ),
        ("0xf.b5p64tail", 0x442f6a0000000000, 9),
        (" -0x78d.p850tail", 0xf5be340000000000, 12),
        ("0x8c6.c5af972557765p-753tail", 0x11918d8b5f2e4aaf, 24),
        (
            "0x09e2f13620eeb5a4f419c8191e.3b1eb6c850b953b58dp1012tail",
            0x7ff0000000000000,
            52,
        ),
        (
            " -0x270f4d00c0c5df587dbf3a3dd29b1242275a467dda.714e8379f36d5f14e4d55ca818f8b8b3ba48500p393tail",
            0xe2d387a6806062f0,
            90,
        ),
        (
            "0x10ac7a676a50a14d33f645c681b54cce2b25525ad15e3e152fc.1d3f190a2p-637tail",
            0x24a0ac7a676a50a1,
            68,
        ),
        ("0x5b2025d6f.dea81p170tail", 0x4cb6c80975bf7aa0, 21),
        (" -0x3875461.4c74276p-405tail", 0xa83c3aa30a63a13b, 24),
        (
            "0x069.aa6a1b067176e221547e9b79132603ddb4c6d1a1ccb413d8d04fp-846tail",
            0x0b7a6a9a86c19c5e,
            63,
        ),
    ];
    for &(text, bits, consumed) in cases {
        let result = atof_prefix(text);
        assert_eq!(result.0.to_bits(), bits, "{text}");
        assert_eq!(result.1, consumed, "{text}");
    }
}
