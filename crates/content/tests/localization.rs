use qa_content::localization::Catalog;
use qa_core::text::FixedText;

fn formatted(catalog: &Catalog, base: &[u8], arguments: &[Option<&[u8]>]) -> Vec<u8> {
    let mut text = FixedText::<1023>::default();
    catalog
        .format(
            base,
            arguments.len(),
            |index| Ok::<_, ()>(arguments[index]),
            &mut text,
        )
        .unwrap();
    text.as_bytes().to_vec()
}

#[test]
fn native_localization_keeps_exact_keys_argument_order_and_fallbacks() {
    let catalog = Catalog::load(
        br#"
        name = "Marine"
        NAME = "Upper"
        greeting = "Hello {$}, {}!"
        reordered = "{1} / {0:ignored} / {1}"
        name = "Ranger"
        mixed = "{} {0}"
        broken = "{"
    "#,
    )
    .unwrap();
    assert_eq!(catalog.len(), 4);
    let args = [Some(&b"$name"[..]), Some(&b"raw {0}"[..])];
    for (base, expected) in [
        (&b"$greeting"[..], &b"Hello Ranger, raw {0}!"[..]),
        (b"$reordered", b"raw {0} / Ranger / raw {0}"),
        (b"$NAME", b"Upper"),
        (b"$missing", b"missing"),
        (b"$mixed", b"mixed"),
        (b"$broken", b"broken"),
        (b"{} {}", b"Ranger raw {0}"),
        (b"{} {1}", b"{} {1}"),
        (b"{0} {}", b"{0} {}"),
        (b"unterminated {", b"unterminated {"),
        (b"literal {{braces}}", b"literal {{braces}}"),
    ] {
        assert_eq!(formatted(&catalog, base, &args), expected, "{base:?}");
    }
    assert_eq!(formatted(&catalog, b"$greeting", &args[..1]), b"greeting");
    assert_eq!(
        formatted(&catalog, b"$greeting", &[args[0], None]),
        b"greeting"
    );
    assert_eq!(
        formatted(&catalog, b"{}", &[Some(b"$greeting")]),
        b"greeting"
    );
    assert_eq!(formatted(&catalog, b"plain\0ignored", &[]), b"plain");
}

#[test]
fn native_argument_indices_narrow_to_u8_and_output_stays_bounded() {
    let catalog = Catalog::default();
    for base in [b"{256}".as_slice(), b"{4294967296}", b"{ +256:spec}"] {
        assert_eq!(formatted(&catalog, base, &[Some(b"zero")]), b"zero");
    }
    assert_eq!(formatted(&catalog, b"{-1}", &[Some(b"zero")]), b"{-1}");
    assert_eq!(
        formatted(&catalog, b"{}{}{}{}{}{}{}{}{}", &[Some(b"x".as_slice()); 9]),
        b"{}{}{}{}{}{}{}{}{}"
    );
    let long = vec![b'x'; 2048];
    assert_eq!(
        formatted(&catalog, b"{}suffix", &[Some(&long)]),
        long[..1023]
    );
    assert_eq!(formatted(&catalog, &long, &[]), long[..1023]);
    assert_eq!(
        formatted(&catalog, b"prefix {}", &[Some(b"\x80\xff")]),
        b"prefix \x80\xff"
    );
}
