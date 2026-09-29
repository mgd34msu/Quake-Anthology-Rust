//! Legacy menu script parsing.

pub mod lexer;
pub mod memory;
pub mod precomp_memory;
pub mod preprocessor;
pub mod source_memory;
pub mod token_memory;

pub use preprocessor::{
    script_number_subtype_name, script_token_type_name, DebugEvalCallback, DiagnosticSeverity, GlobalDefine,
    IncludeKind, IncludeRequest, IncludeResolver, NowCallback, NullIncludeResolver, NumberFlag, Punctuation,
    ReportCallback, ScriptDateTime, ScriptDiagnostic, ScriptGlobalDefines, ScriptGlobalSnapshot, ScriptPreprocessor,
    ScriptPreprocessorOptions, ScriptPunctuation, ScriptSource, ScriptSourcePosition, ScriptSourceReader, ScriptToken,
    ScriptTokenRecord, ScriptTokenType, SourceLocation, DEFAULT_SCRIPT_TOKEN_LIMIT, MAX_DEFINE_PARAMETERS,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexported_script_limits_match_donor() {
        assert_eq!(DEFAULT_SCRIPT_TOKEN_LIMIT, 1024);
        const { assert!(MAX_DEFINE_PARAMETERS > 0) }
    }
}
