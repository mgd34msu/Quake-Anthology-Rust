//! Q2 module ABI limits are available independently of the host backend.
use qa_core::primitives::RuleSetId;

pub const INFO_FIELD_BYTES: usize = 64;
#[derive(Clone, Copy)]
pub struct InfoLayout {
    pub userinfo_bytes: usize,
}
pub const CLASSIC: InfoLayout = InfoLayout {
    userinfo_bytes: 512,
};
pub const RERELEASE: InfoLayout = InfoLayout {
    userinfo_bytes: 2048,
};
const INFO: [Option<InfoLayout>; 5] = [None, None, Some(CLASSIC), Some(RERELEASE), None];
pub fn userinfo_bytes(rules: RuleSetId) -> Option<usize> {
    INFO[rules as usize].map(|layout| layout.userinfo_bytes)
}
