use super::*;

#[test]
fn physical_count_is_positive() {
    assert!(physical_core_count() > 0);
}

#[cfg(target_os = "linux")]
#[test]
fn allowed_list_preserves_sparse_affinity_ranges() {
    assert_eq!(cpu_list(" 0-2,8,11-12 \n"), Some(vec![0, 1, 2, 8, 11, 12]));
    for malformed in ["", "1-0", "0-", "-1", "1-2-3", "1,", "18446744073709551615"] {
        assert_eq!(cpu_list(malformed), None);
    }
}
