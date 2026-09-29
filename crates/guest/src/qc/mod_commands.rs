//! QuakeC console-command lowering into source calls.
//!
//! Ported from `src/compat/qc/mod-commands.ts`. Declaration mirrors
//! (`ModConsoleCommand`, `ModConsoleValue`, `ModSourceCall`) live in
//! `super::mod_provider`, absorbed from `src/contracts/mod-callbacks.ts`.

use qa_core::numeric::native_atof;

use super::mod_provider::{
    ModCallbackValue, ModConsoleArgType, ModConsoleCommand, ModConsoleGlobal, ModConsoleValue, ModSourceCall,
    ModSourceGlobal,
};
use crate::error::GuestError;

/// Lower a console invocation into a source call.
pub fn qc_console_call(
    command: &ModConsoleCommand,
    argv: &[String],
    args_text: &str,
) -> Result<ModSourceCall, GuestError> {
    let resolve = |value: &ModConsoleValue| -> Result<ModCallbackValue, GuestError> {
        match value {
            ModConsoleValue::Float(value) => Ok(ModCallbackValue::Float(*value)),
            ModConsoleValue::String(value) => Ok(ModCallbackValue::String(value.clone())),
            ModConsoleValue::Vector(value) => Ok(ModCallbackValue::Vector(*value)),
            ModConsoleValue::Argument { index, arg_type } => {
                let text = argv.get(*index).map(String::as_str).unwrap_or("");
                match arg_type {
                    ModConsoleArgType::String => Ok(ModCallbackValue::String(text.to_string())),
                    ModConsoleArgType::Float => native_atof(text).map(ModCallbackValue::Float).map_err(|error| {
                        GuestError::invalid(format!("Mod console argument is not source text: {error}"))
                    }),
                }
            }
            ModConsoleValue::ArgumentsText => Ok(ModCallbackValue::String(args_text.to_string())),
            ModConsoleValue::ArgumentCount => Ok(ModCallbackValue::Float(argv.len() as f64)),
        }
    };
    let mut arguments = Vec::with_capacity(command.arguments.len());
    for value in &command.arguments {
        arguments.push(resolve(value)?);
    }
    let mut globals = Vec::with_capacity(command.globals.len());
    for global in &command.globals {
        globals.push(ModSourceGlobal {
            name: global.name.clone(),
            value: resolve(&global.value)?,
        });
    }
    Ok(ModSourceCall {
        function: command.function.clone(),
        arguments,
        globals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    fn command() -> ModConsoleCommand {
        ModConsoleCommand {
            name: "give".to_string(),
            function: "cmd_give".to_string(),
            arguments: vec![
                ModConsoleValue::Argument {
                    index: 1,
                    arg_type: ModConsoleArgType::String,
                },
                ModConsoleValue::Argument {
                    index: 2,
                    arg_type: ModConsoleArgType::Float,
                },
                ModConsoleValue::ArgumentsText,
                ModConsoleValue::ArgumentCount,
                ModConsoleValue::Float(1.5),
                ModConsoleValue::Vector(vec3(1.0, 2.0, 3.0)),
            ],
            globals: vec![ModConsoleGlobal {
                name: "self".to_string(),
                value: ModConsoleValue::String("fixed".to_string()),
            }],
        }
    }

    #[test]
    fn lowers_arguments_text_and_count() {
        let argv = vec!["give".to_string(), "shells".to_string(), "8".to_string()];
        let call = qc_console_call(&command(), &argv, "shells 8").unwrap();
        assert_eq!(call.function, "cmd_give");
        assert_eq!(call.arguments[0], ModCallbackValue::String("shells".to_string()));
        assert_eq!(call.arguments[1], ModCallbackValue::Float(8.0));
        assert_eq!(call.arguments[2], ModCallbackValue::String("shells 8".to_string()));
        assert_eq!(call.arguments[3], ModCallbackValue::Float(3.0));
        assert_eq!(call.arguments[4], ModCallbackValue::Float(1.5));
        assert_eq!(call.arguments[5], ModCallbackValue::Vector(vec3(1.0, 2.0, 3.0)));
        assert_eq!(call.globals[0].name, "self");
    }

    #[test]
    fn missing_argument_reads_empty() {
        let argv = vec!["give".to_string()];
        let call = qc_console_call(&command(), &argv, "").unwrap();
        assert_eq!(call.arguments[0], ModCallbackValue::String(String::new()));
        assert_eq!(call.arguments[1], ModCallbackValue::Float(0.0));
    }

    #[test]
    fn rejects_non_byte_argument_text() {
        let argv = vec!["give".to_string(), "x".to_string(), "héllo".to_string()];
        assert!(qc_console_call(&command(), &argv, "").is_err());
    }
}
