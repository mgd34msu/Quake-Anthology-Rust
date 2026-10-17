use qa_formats::{
    FormatError,
    inflate::{InflateFlush, Inflater},
};

// RFC1951 final stored block, LEN12 / NLEN65523, with no zlib header or trailer.
const RAW: &[u8] = b"\x01\x0c\x00\xf3\xffhello world!";

#[test]
fn bounded_steps_resume_small_input_and_output_and_preserve_exact_totals() {
    let mut inflater = Inflater::raw();
    let mut input_at = 0;
    let mut end = 0;
    let mut decoded = Vec::new();
    let mut finished = false;
    while !finished {
        end = (end + 2).min(RAW.len());
        let mut output = [0xcc; 5];
        let step = inflater
            .step(
                &RAW[input_at..end],
                &mut output[..3],
                InflateFlush::Continue,
            )
            .unwrap();
        assert!(step.consumed <= end - input_at);
        assert!(step.produced <= 3);
        assert_eq!(&output[3..], &[0xcc; 2]);
        assert!(step.consumed != 0 || step.produced != 0 || step.finished || end < RAW.len());
        input_at += step.consumed;
        decoded.extend_from_slice(&output[..step.produced]);
        finished = step.finished;
        assert_eq!(inflater.total_in(), input_at as u64);
        assert_eq!(inflater.total_out(), decoded.len() as u64);
    }
    assert_eq!(input_at, RAW.len());
    assert_eq!(decoded, b"hello world!");

    inflater.reset();
    assert_eq!((inflater.total_in(), inflater.total_out()), (0, 0));
    let mut with_tail = RAW.to_vec();
    with_tail.extend_from_slice(b"following native record");
    let mut output = [0xcc; 32];
    let step = inflater
        .step(&with_tail, &mut output, InflateFlush::Continue)
        .unwrap();
    assert!(step.finished);
    assert_eq!((step.consumed, step.produced), (RAW.len(), 12));
    assert_eq!(&with_tail[step.consumed..], b"following native record");
    assert_eq!(&output[..step.produced], b"hello world!");
    assert_eq!(&output[step.produced..], &[0xcc; 20]);
}

#[test]
fn sync_boundaries_keep_the_same_stream_until_its_final_block() {
    let mut inflater = Inflater::raw();
    // First stored block is nonfinal; the next block supplies BFINAL1.
    let first = b"\x00\x03\x00\xfc\xffone";
    let last = b"\x01\x03\x00\xfc\xfftwo";
    let mut output = [0xcc; 8];
    let step = inflater
        .step(first, &mut output, InflateFlush::Sync)
        .unwrap();
    assert_eq!(
        (step.consumed, step.produced, step.finished),
        (first.len(), 3, false)
    );
    assert_eq!(&output[..3], b"one");
    let step = inflater
        .step(last, &mut output, InflateFlush::Sync)
        .unwrap();
    assert_eq!(
        (step.consumed, step.produced, step.finished),
        (last.len(), 3, true)
    );
    assert_eq!(&output[..3], b"two");
    assert_eq!(
        (inflater.total_in(), inflater.total_out()),
        ((first.len() + last.len()) as u64, 6)
    );
}

#[test]
fn incomplete_input_is_resumable_and_corrupt_data_can_be_reset_without_a_new_owner() {
    let mut inflater = Inflater::raw();
    let mut output = [0; 32];
    let step = inflater
        .step(&RAW[..RAW.len() - 1], &mut output, InflateFlush::Continue)
        .unwrap();
    assert!(!step.finished);
    assert_eq!(step.produced, 11);
    assert_eq!(&output[..11], b"hello world");
    let stalled = inflater
        .step(&[], &mut output, InflateFlush::Continue)
        .unwrap();
    assert_eq!(
        (stalled.consumed, stalled.produced, stalled.finished),
        (0, 0, false)
    );
    let final_step = inflater
        .step(&RAW[RAW.len() - 1..], &mut output, InflateFlush::Continue)
        .unwrap();
    assert_eq!(
        (
            final_step.consumed,
            final_step.produced,
            final_step.finished
        ),
        (1, 1, true)
    );
    assert_eq!(output[0], b'!');

    inflater.reset();
    // BTYPE3 is reserved by RFC1951.
    assert_eq!(
        inflater.step(&[7], &mut output, InflateFlush::Continue),
        Err(FormatError::Compression)
    );
    inflater.reset();
    assert_eq!((inflater.total_in(), inflater.total_out()), (0, 0));
    let recovered = inflater
        .step(RAW, &mut output, InflateFlush::Continue)
        .unwrap();
    assert!(recovered.finished);
    assert_eq!(&output[..recovered.produced], b"hello world!");
}
