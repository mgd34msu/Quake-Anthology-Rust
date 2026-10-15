use qa_compat::abi::ImportStep;

pub fn word(step: ImportStep) -> u64 {
    match step {
        ImportStep::Return(value) => value,
        ImportStep::Call { .. } => panic!("scalar import unexpectedly requested a callback"),
    }
}
