//! One host-owned command table, aliases and command buffer for every source.
use crate::{
    command_buffer::CommandBuffer,
    command_text::{self, Arguments, TextError},
    cvars::{Cvars, WriteError},
    views::{Context, Source},
};
use std::fmt::Arguments as Output;

pub trait Host {
    fn print(&mut self, text: Output<'_>);
    fn read_script(&mut self, path: &str) -> Result<String, String>;
    fn quit(&mut self);
}
#[derive(Debug)]
pub enum CommandError {
    Text(TextError),
    Cvar(WriteError),
    Usage,
    UnknownCvar,
    AliasName,
    Script(String),
}
impl From<TextError> for CommandError {
    fn from(e: TextError) -> Self {
        Self::Text(e)
    }
}
impl From<WriteError> for CommandError {
    fn from(e: WriteError) -> Self {
        Self::Cvar(e)
    }
}
pub type CommandFn<H> =
    fn(&mut Console<H>, &mut H, &Arguments<'_>, Context) -> Result<(), CommandError>;
struct Command<H> {
    name: &'static str,
    function: CommandFn<H>,
}
struct Alias {
    name: String,
    text: String,
}
pub struct Console<H> {
    pub cvars: Cvars,
    buffer: CommandBuffer,
    commands: Vec<Command<H>>,
    aliases: Vec<Alias>,
}
impl<H: Host> Console<H> {
    pub fn new(context: Context) -> Self {
        let mut console = Self {
            cvars: Cvars::with_context(context),
            buffer: CommandBuffer::new(),
            commands: Vec::with_capacity(64),
            aliases: Vec::new(),
        };
        for (name, function) in [
            ("echo", Self::echo as CommandFn<H>),
            ("wait", Self::wait),
            ("alias", Self::alias),
            ("unalias", Self::unalias),
            ("exec", Self::exec),
            ("vstr", Self::vstr),
            ("set", Self::set),
            ("cmdlist", Self::cmdlist),
            ("cvarlist", Self::cvarlist),
            ("quit", Self::quit),
        ] {
            console.commands.push(Command { name, function });
        }
        console
    }
    pub fn register(&mut self, name: &'static str, function: CommandFn<H>) -> bool {
        if self
            .commands
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case(name))
        {
            return false;
        }
        self.commands.push(Command { name, function });
        true
    }
    pub fn append(&mut self, text: &str, context: Context) -> Result<(), TextError> {
        self.buffer.append(text, context)
    }
    pub fn idle(&self) -> bool {
        self.buffer.is_empty()
    }
    pub fn execute_frame(&mut self, host: &mut H) {
        let mut executed = 0;
        let mut aliases = 0;
        while !self.buffer.is_empty() && self.buffer.ready() {
            if executed >= 4096 {
                host.print(format_args!("Command frame limit\n"));
                break;
            }
            let Some(line) = self.buffer.next_line() else {
                break;
            };
            executed += 1;
            let expanded;
            let text = if matches!(
                line.context.source,
                Source::Quake2 | Source::Quake2Rerelease
            ) {
                expanded = command_text::expand(&line.text, |name| {
                    self.cvars
                        .bind(name, line.context)
                        .and_then(|view| self.cvars.read(view).ok())
                        .map(|text| text.as_str().to_owned())
                });
                match &expanded {
                    Ok(text) => text.as_str(),
                    Err(error) => {
                        host.print(format_args!("Command rejected: {error:?}\n"));
                        continue;
                    }
                }
            } else {
                line.text.as_str()
            };
            let args = match command_text::tokenize(text, line.context.source) {
                Ok(args) => args,
                Err(error) => {
                    host.print(format_args!("Command rejected: {error:?}\n"));
                    continue;
                }
            };
            let name = args.get(0);
            if name.is_empty() {
                continue;
            }
            let result = if let Some(function) = self
                .commands
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(name))
                .map(|c| c.function)
            {
                function(self, host, &args, line.context)
            } else if let Some(view) = self.cvars.bind(name, line.context) {
                if args.values.len() == 1 {
                    match self.cvars.read(view) {
                        Ok(value) => {
                            if self.cvars.private(view) {
                                host.print(format_args!("{name} is private\n"));
                            } else {
                                host.print(format_args!("{name} = \"{}\"\n", value.as_str()));
                            }
                            Ok(())
                        }
                        Err(error) => Err(CommandError::Cvar(WriteError::Conversion(error))),
                    }
                } else {
                    self.cvars.write(view, &args.tail(1)).map_err(Into::into)
                }
            } else if let Some(alias) = self
                .aliases
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case(name))
            {
                aliases += 1;
                if aliases > 16 {
                    host.print(format_args!("Alias expansion limit\n"));
                    continue;
                }
                let text = alias.text.clone();
                self.buffer.insert(&text, line.context).map_err(Into::into)
            } else {
                host.print(format_args!("Unknown command \"{name}\"\n"));
                Ok(())
            };
            if let Err(error) = result {
                host.print(format_args!("Command {name} rejected: {error:?}\n"));
            }
        }
    }
    fn echo(&mut self, host: &mut H, args: &Arguments<'_>, _: Context) -> Result<(), CommandError> {
        for text in args.values.iter().skip(1) {
            host.print(format_args!("{text} "));
        }
        host.print(format_args!("\n"));
        Ok(())
    }
    fn wait(&mut self, _: &mut H, args: &Arguments<'_>, _: Context) -> Result<(), CommandError> {
        let frames = if args.values.len() == 1 {
            1
        } else {
            crate::numbers::integer(args.get(1)).max(0) as u32
        };
        self.buffer.wait(frames);
        Ok(())
    }
    fn alias(
        &mut self,
        host: &mut H,
        args: &Arguments<'_>,
        _: Context,
    ) -> Result<(), CommandError> {
        if args.values.len() == 1 {
            for alias in &self.aliases {
                host.print(format_args!("{} : {}", alias.name, alias.text));
            }
            return Ok(());
        }
        let name = args.get(1);
        if name.is_empty()
            || name.len() >= 32
            || name
                .bytes()
                .any(|b| b <= 32 || matches!(b, b'"' | b';' | b'\\' | b'/'))
        {
            return Err(CommandError::AliasName);
        }
        let text = args.tail(2) + "\n";
        if let Some(alias) = self
            .aliases
            .iter_mut()
            .find(|a| a.name.eq_ignore_ascii_case(name))
        {
            alias.text = text;
        } else {
            self.aliases.push(Alias {
                name: name.to_owned(),
                text,
            });
        }
        Ok(())
    }
    fn unalias(&mut self, _: &mut H, args: &Arguments<'_>, _: Context) -> Result<(), CommandError> {
        if args.values.len() != 2 {
            return Err(CommandError::Usage);
        }
        self.aliases
            .retain(|a| !a.name.eq_ignore_ascii_case(args.get(1)));
        Ok(())
    }
    fn exec(
        &mut self,
        host: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        if args.values.len() != 2 {
            return Err(CommandError::Usage);
        }
        let path = args.get(1);
        let path = if std::path::Path::new(path).extension().is_some() {
            path.to_owned()
        } else {
            format!("{path}.cfg")
        };
        let text = host.read_script(&path).map_err(CommandError::Script)?;
        self.buffer.insert(&text, context)?;
        Ok(())
    }
    fn vstr(
        &mut self,
        _: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        if args.values.len() != 2 {
            return Err(CommandError::Usage);
        }
        let view = self
            .cvars
            .bind(args.get(1), context)
            .ok_or(CommandError::UnknownCvar)?;
        let text = self
            .cvars
            .read(view)
            .map_err(|e| CommandError::Cvar(WriteError::Conversion(e)))?
            .into_owned();
        self.buffer.insert(&(text + "\n"), context)?;
        Ok(())
    }
    fn set(
        &mut self,
        _: &mut H,
        args: &Arguments<'_>,
        context: Context,
    ) -> Result<(), CommandError> {
        if args.values.len() < 3 {
            return Err(CommandError::Usage);
        }
        let view = self
            .cvars
            .bind(args.get(1), context)
            .ok_or(CommandError::UnknownCvar)?;
        self.cvars.write(view, &args.tail(2))?;
        Ok(())
    }
    fn cmdlist(&mut self, host: &mut H, _: &Arguments<'_>, _: Context) -> Result<(), CommandError> {
        for c in &self.commands {
            host.print(format_args!("{}\n", c.name));
        }
        Ok(())
    }
    fn cvarlist(
        &mut self,
        host: &mut H,
        _: &Arguments<'_>,
        _: Context,
    ) -> Result<(), CommandError> {
        for definition in crate::cvars_generated::DEFINITIONS {
            if definition.family_count > 1 {
                host.print(format_args!("{} [seats]\n", definition.name));
            }
        }
        for (_, name, value, definition, seat) in self.cvars.entries() {
            let indent = if seat == 0 { "" } else { "  " };
            if definition.stored {
                host.print(format_args!(
                    "{indent}{name} = \"{}\"\n",
                    if definition.policies & 2 != 0 {
                        "<private>"
                    } else {
                        value
                    }
                ));
            } else {
                host.print(format_args!("{indent}{name} (derived)\n"));
            }
        }
        Ok(())
    }
    fn quit(&mut self, host: &mut H, _: &Arguments<'_>, _: Context) -> Result<(), CommandError> {
        host.quit();
        self.buffer.clear();
        Ok(())
    }
}
