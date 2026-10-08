pub mod host;

use qa_console::commands::Host;
use qa_content::vfs::Vfs;
use qa_core::events::EventRing;
use qa_network::ingress::PacketReceiver;
use qa_session::clients::Server;

pub struct Runtime {
    pub vfs: Vfs,
    pub quit: bool,
    pub network: PacketReceiver,
    pub server: Server,
    pub events: EventRing,
}

impl Runtime {
    pub fn load() -> Result<Self, String> {
        Ok(Self {
            vfs: Vfs::default(),
            quit: false,
            network: PacketReceiver::default(),
            server: Server::load(64, 8192, 116, 16).map_err(|e| format!("server: {e:?}"))?,
            events: EventRing::load(4096).map_err(|e| format!("output events: {e:?}"))?,
        })
    }
}

impl Host for Runtime {
    fn print(&mut self, text: std::fmt::Arguments<'_>) {
        qa_console::logger::console(text);
    }
    fn read_script(&mut self, path: &str) -> Result<String, String> {
        let file = self
            .vfs
            .open(path.as_bytes())
            .ok_or_else(|| format!("script unavailable: {path}"))?;
        let length = self.vfs.length(file).map_err(|e| format!("{e:?}"))?;
        if length > 65535 {
            return Err("script exceeds command buffer capacity".into());
        }
        let mut bytes = vec![0; length as usize];
        self.vfs
            .read_at(file, 0, &mut bytes)
            .map_err(|e| format!("{e:?}"))?;
        String::from_utf8(bytes).map_err(|e| format!("script is not UTF-8: {e}"))
    }
    fn quit(&mut self) {
        self.quit = true;
    }
}
