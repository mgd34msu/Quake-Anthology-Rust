//! Q2 module ABI limits are available independently of the host backend.
use qa_core::primitives::RuleSetId;

pub const INFO_KEY_BYTES: usize = 64;
pub const INFO_VALUE_BYTES: usize = 256;
#[derive(Clone, Copy)]
pub struct InfoLayout {
    pub userinfo_bytes: usize,
    pub value_bytes: usize,
}
pub const CLASSIC: InfoLayout = InfoLayout {
    userinfo_bytes: 512,
    value_bytes: INFO_KEY_BYTES,
};
pub const RERELEASE: InfoLayout = InfoLayout {
    userinfo_bytes: 2048,
    value_bytes: INFO_VALUE_BYTES,
};
const INFO: [Option<InfoLayout>; 5] = [None, None, Some(CLASSIC), Some(RERELEASE), None];
pub fn info_layout(rules: RuleSetId) -> Option<InfoLayout> {
    INFO[rules as usize]
}
