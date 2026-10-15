use qa_core::random::{Algorithm, Random};

#[test]
fn mt19937_reference_sequence_and_twist_boundary() {
    let mut random = Random::new(5489);
    for expected in [
        3499211612, 581869302, 3890346734, 3586334585, 545404204, 4161255391, 3922919429,
        949333985, 2715962298, 1323567403,
    ] {
        assert_eq!(random.next_u32(), expected);
    }
    // Independent state retains its seed through multiple twists.
    let mut a = Random::new(1);
    let mut b = Random::new(1);
    let mut c = Random::new(2);
    for _ in 0..2000 {
        let value = a.next_u32();
        assert_eq!(value, b.next_u32());
        assert_ne!(value, c.next_u32());
    }
}

#[test]
fn glibc_type3_matches_independent_libc_seed_and_ring_boundaries() {
    // glibc 2.44 srand/rand, captured through ctypes in a separate process.
    // Indices are zero-based and include both cursors wrapping repeatedly.
    const INDICES: [usize; 28] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 27, 28, 29, 30, 31, 32, 33, 309, 310, 311, 620, 621, 622,
        623, 624, 625, 999, 1999,
    ];
    for (seed, expected) in [
        (
            0x00000000,
            [
                1804289383, 846930886, 1681692777, 1714636915, 1957747793, 424238335, 719885386,
                1649760492, 596516649, 1189641421, 233665123, 2145174067, 468703135, 1101513929,
                1801979802, 1315634022, 635723058, 1057418418, 995706887, 1823089412, 1452888574,
                1280631491, 126401947, 1204462951, 1210359231, 521035021, 1143565421, 1455252833,
            ],
        ),
        (
            0x00000001,
            [
                1804289383, 846930886, 1681692777, 1714636915, 1957747793, 424238335, 719885386,
                1649760492, 596516649, 1189641421, 233665123, 2145174067, 468703135, 1101513929,
                1801979802, 1315634022, 635723058, 1057418418, 995706887, 1823089412, 1452888574,
                1280631491, 126401947, 1204462951, 1210359231, 521035021, 1143565421, 1455252833,
            ],
        ),
        (
            0x00000002,
            [
                1505335290, 1738766719, 190686788, 260874575, 747983061, 906156498, 1502820864,
                142559277, 1261608745, 1380759627, 682319630, 221122296, 96423063, 1385108013,
                1726457586, 1835189782, 1575794802, 2145219718, 399878157, 1021056178, 725569216,
                552499431, 829541303, 160953413, 11199140, 473478159, 2057566690, 117885264,
            ],
        ),
        (
            0x000003e9,
            [
                469353932, 2078239751, 916013951, 1943028792, 940422454, 1109929490, 759657113,
                622451069, 574969520, 483643406, 614302735, 300968346, 196778039, 1585202521,
                770322279, 127534142, 353732824, 1919993869, 1963399290, 1330182007, 1672169143,
                2066951529, 141530998, 1193006580, 1896111318, 39760794, 1356910565, 899946438,
            ],
        ),
        (
            0x7fffffff,
            [
                1065668062, 2142264300, 1066566375, 1064012770, 2141034222, 1065509725, 2135810236,
                2139491828, 2138030783, 2133535487, 1036193370, 2111890717, 1046299399, 2103680308,
                1030075131, 1041080051, 1022763035, 1792919474, 1546649487, 2121561633, 1223614185,
                14073445, 2018329223, 690271005, 294485555, 953457906, 1698607095, 958225834,
            ],
        ),
        (
            0x80000000,
            [
                1336741213, 1210407648, 1447044896, 337392383, 82502902, 538660432, 1313908778,
                370221063, 344413073, 1896089129, 446952434, 1723835522, 1213320778, 1649425186,
                913093087, 276244779, 948986434, 747840053, 581761865, 1694861624, 367809021,
                1523744448, 1687347870, 194050192, 993798258, 236023340, 193932953, 1851989278,
            ],
        ),
        (
            0xffffffff,
            [
                254925627, 1205188300, 366127624, 1401405153, 76053476, 1604170158, 1302235366,
                362229243, 334960208, 1882140968, 1483145805, 1688242591, 112136530, 1605621846,
                1943168218, 1317324830, 1971749470, 393275879, 2128411352, 1668939610, 1591423207,
                1537817893, 1558193445, 884321197, 1288283813, 1189481246, 1892540048, 662731464,
            ],
        ),
    ] {
        let mut random = Random::with_algorithm(seed, Algorithm::GlibcType3);
        let mut sample = 0;
        for index in 0..=1999 {
            let value = random.next_u32();
            assert!(value <= i32::MAX as u32);
            if index == INDICES[sample] {
                assert_eq!(value, expected[sample], "seed={seed:#x} index={index}");
                sample += 1;
            }
        }
        assert_eq!(sample, INDICES.len());
    }
}

#[test]
fn glibc_zero_seed_is_one_and_qc_low_bits_can_reach_the_upper_endpoint() {
    let mut zero = Random::with_algorithm(0, Algorithm::GlibcType3);
    let mut one = Random::with_algorithm(1, Algorithm::GlibcType3);
    for _ in 0..2048 {
        assert_eq!(zero.next_u32(), one.next_u32());
    }
    // Independent libc reference: the first word from seed 70403 is 2113667071.
    let word = Random::with_algorithm(70403, Algorithm::GlibcType3).next_u32();
    assert_eq!(word, 2113667071);
    assert_eq!(word & 0x7fff, 0x7fff);
}

#[test]
fn cloned_streams_resume_independently_across_algorithm_boundaries() {
    for algorithm in [Algorithm::Mt19937, Algorithm::GlibcType3] {
        let mut random = Random::with_algorithm(0x80000000, algorithm);
        for _ in 0..620 {
            random.next_u32();
        }
        let checkpoint = random.clone();
        let expected: [u32; 80] = std::array::from_fn(|_| random.next_u32());
        let mut restored = checkpoint.clone();
        for value in expected {
            assert_eq!(restored.next_u32(), value, "{algorithm:?}");
        }
        for _ in 0..128 {
            random.next_u32();
            restored.next_u32();
        }
        let mut untouched = checkpoint;
        for value in expected {
            assert_eq!(untouched.next_u32(), value, "{algorithm:?}");
        }
    }
}

#[test]
fn existing_constructor_keeps_the_explicit_mt19937_stream() {
    for seed in [0, 5489, u32::MAX] {
        let mut default = Random::new(seed);
        let mut explicit = Random::with_algorithm(seed, Algorithm::Mt19937);
        for _ in 0..2000 {
            assert_eq!(default.next_u32(), explicit.next_u32());
        }
    }
}
