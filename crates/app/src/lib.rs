pub mod host;

use qa_console::commands::{Host, ScriptError};
use qa_content::vfs::Vfs;
use qa_core::events::EventRing;
use qa_core::loopback::Loopback;
use qa_network::ingress::PacketReceiver;
use qa_session::clients::Server;

pub struct Runtime {
    pub vfs: Vfs,
    pub quit: bool,
    pub network: PacketReceiver,
    pub server: Server,
    pub events: EventRing,
    pub loopback: Loopback,
    pub input: qa_input::Input,
    pub input_time: qa_core::sys_events::EventTime,
    script_reader: qa_formats::archive::ArchiveReader,
}

impl Runtime {
    pub fn load() -> Result<Self, String> {
        Ok(Self {
            vfs: Vfs::default(),
            quit: false,
            network: PacketReceiver::default(),
            server: Server::load(64, 8192, 116, 16).map_err(|e| format!("server: {e:?}"))?,
            events: EventRing::load(4096).map_err(|e| format!("output events: {e:?}"))?,
            loopback: Loopback::load(),
            input: qa_input::Input::load(),
            input_time: qa_core::sys_events::EventTime::default(),
            script_reader: qa_formats::archive::ArchiveReader::default(),
        })
    }
}

impl Host for Runtime {
    fn input(&mut self) -> &mut qa_input::Input {
        &mut self.input
    }
    fn input_time(&self) -> qa_core::sys_events::EventTime {
        self.input_time
    }
    fn print(&mut self, text: std::fmt::Arguments<'_>) {
        qa_console::logger::console(text);
    }
    fn read_script(&mut self, path: &str, destination: &mut [u8]) -> Result<usize, ScriptError> {
        let file = self.vfs.open(path.as_bytes()).ok_or(ScriptError::Missing)?;
        let length = self.vfs.length(file).map_err(|_| ScriptError::Read)?;
        let length = usize::try_from(length).map_err(|_| ScriptError::TooLong)?;
        if length > destination.len() {
            return Err(ScriptError::TooLong);
        }
        let read = self
            .vfs
            .read_into_reusing(file, &mut destination[..length], &mut self.script_reader)
            .map_err(|_| ScriptError::Read)?;
        if read != length {
            return Err(ScriptError::Read);
        }
        Ok(length)
    }
    fn quit(&mut self) {
        self.quit = true;
    }
}
