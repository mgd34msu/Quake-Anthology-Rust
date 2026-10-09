use qa_content::products::{Edition, RootMetadata, client_rules_for_root, root_metadata};
use qa_core::primitives::RuleSetId;
use std::path::Path;

#[test]
fn stock_client_identity_comes_from_the_exact_product_root() {
    // These paths need no files or map witnesses: only existing stock product
    // directory and edition metadata identifies the startup client policy.
    for (root, expected) in [
        ("/unmounted/products/q1/id1", RuleSetId::Quake),
        ("/unmounted/products/q1/rerelease/id1", RuleSetId::Quake),
        ("/unmounted/products/q1/qw", RuleSetId::QuakeWorld),
        ("/unmounted/products/q2/baseq2", RuleSetId::Quake2),
        (
            "/unmounted/products/q2/rerelease/baseq2",
            RuleSetId::Quake2Rerelease,
        ),
        ("/unmounted/products/q3a/baseq3", RuleSetId::Quake3),
        ("/unmounted/products/q3a/missionpack", RuleSetId::Quake3),
        ("/unmounted/products/quakelive/baseq3", RuleSetId::Quake3),
        ("/unmounted/products/q1/rogue", RuleSetId::Quake),
        ("/unmounted/products/q2/rogue", RuleSetId::Quake2),
        ("/unmounted/products/id1", RuleSetId::Quake),
        ("/unmounted/products/baseq2", RuleSetId::Quake2),
        ("/unmounted/products/Q1/ID1", RuleSetId::Quake),
    ] {
        assert_eq!(client_rules_for_root(Path::new(root)), Some(expected));
    }
}

#[test]
fn ambiguous_or_nested_nonstock_roots_do_not_inherit_an_ancestor_client() {
    for root in [
        "/unmounted/products/rogue",
        "/unmounted/products/ctf",
        "/unmounted/products/q1",
        "/unmounted/products/q1/ad",
        "/unmounted/products/q1/id1/custom_mod",
        "/unmounted/products/q1/qw/custom_mod",
        "/unmounted/products/q2/baseq2/custom_mod",
        "/unmounted/products/q2/rerelease/baseq2/custom_mod",
        "/unmounted/products/q3a/baseq3/custom_mod",
        "/unmounted/products/q1/id1/maps",
        "/unmounted/products/q1/id1/pak0.pak",
        "/unmounted/products/q2/id1",
        "/unmounted/products/unknown",
    ] {
        assert_eq!(client_rules_for_root(Path::new(root)), None);
    }
}

#[test]
fn profile_metadata_retains_the_product_edition_independently_of_client_rules() {
    for (root, root_hint, edition, client_rules) in [
        (
            "/unmounted/products/q1/id1",
            b"q1".as_slice(),
            Edition::Classic,
            RuleSetId::Quake,
        ),
        (
            "/unmounted/products/q1/rerelease/id1",
            b"q1".as_slice(),
            Edition::Rerelease,
            RuleSetId::Quake,
        ),
        (
            "/unmounted/products/q1/qw",
            b"q1".as_slice(),
            Edition::QuakeWorld,
            RuleSetId::QuakeWorld,
        ),
        (
            "/unmounted/products/q2/baseq2",
            b"q2".as_slice(),
            Edition::Classic,
            RuleSetId::Quake2,
        ),
        (
            "/unmounted/products/q2/rerelease/baseq2",
            b"q2".as_slice(),
            Edition::Rerelease,
            RuleSetId::Quake2Rerelease,
        ),
    ] {
        assert_eq!(
            root_metadata(Path::new(root)),
            Some(RootMetadata {
                client_rules,
                root_hint,
                edition,
            })
        );
    }
}
