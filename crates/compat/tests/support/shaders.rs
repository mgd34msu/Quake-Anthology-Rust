use qa_compat::services::CallError;
use qa_core::primitives::RuleSetId;

#[derive(Default)]
pub struct ShaderLoader {
    pub calls: Vec<(Vec<u8>, RuleSetId, bool)>,
}
impl qa_compat::services::ModelLookup for ShaderLoader {
    fn bounds(&self, _: &[u8], _: RuleSetId) -> Option<qa_core::primitives::Bounds> {
        None
    }
    fn shader_handle(&self, name: &[u8]) -> Option<i32> {
        match name {
            b"ui/pic" => Some(42),
            b"ui/missing" => Some(0),
            _ => None,
        }
    }
}
impl qa_compat::services::ModelLoader for ShaderLoader {
    fn precache(
        &mut self,
        _: &qa_content::vfs::Vfs,
        _: &[u8],
        _: RuleSetId,
    ) -> Result<qa_core::primitives::Bounds, CallError> {
        Err(CallError::Unsupported)
    }
    fn register_shader(
        &mut self,
        _: &qa_content::vfs::Vfs,
        name: &[u8],
        source: RuleSetId,
        mipmaps: bool,
    ) -> Result<i32, CallError> {
        self.calls.push((name.to_vec(), source, mipmaps));
        if name == b"ui/pic" {
            Ok(42)
        } else {
            Err(CallError::File)
        }
    }
}
