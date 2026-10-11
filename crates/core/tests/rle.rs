use qa_core::rle::{PvsRleError, pvs};

#[test]
fn pvs_validation_and_decode_share_runs_and_preserve_error_boundaries() {
    for count in 0..=255_u8 {
        let encoded = [99, 7, 0, count, 11];
        let length = usize::from(count) + 2;
        let mut output = vec![55; length];
        let expected = if count == 0 {
            Err(PvsRleError::InvalidRun)
        } else {
            Ok(())
        };
        assert_eq!(pvs(&encoded, 1, length, None), expected);
        assert_eq!(pvs(&encoded, 1, length, Some(&mut output)), expected);
        assert_eq!(output[0], 7);
        if count != 0 {
            assert!(output[1..length - 1].iter().all(|&byte| byte == 0));
            assert_eq!(output[length - 1], 11);
        }
    }
    assert_eq!(pvs(&[], 0, 1, None), Err(PvsRleError::Truncated));
    assert_eq!(pvs(&[0], 0, 1, None), Err(PvsRleError::TruncatedRun));
    assert_eq!(pvs(&[0, 2], 0, 1, None), Err(PvsRleError::InvalidRun));
    assert_eq!(pvs(&[], usize::MAX, 0, None), Ok(()));
    let mut output = [55; 3];
    assert_eq!(
        pvs(&[7, 0], 0, 3, Some(&mut output)),
        Err(PvsRleError::TruncatedRun)
    );
    assert_eq!(output, [7, 55, 55]);
}
