//! Config library menu service over saved configuration files.
//!
//! Donor provenance: `src/app/bootstrap/startup.ts`
//! (`configurationLibrary`).
//!
//! Sync port: refresh lists inline, so no generation guard is needed; the
//! donor's async exec completion becomes the exec sink's return (the sink
//! reports the executed script name, or the finished `Not found`/`Could not
//! read` text). Failures surface as status text, and the library stays open
//! exactly like the donor.

use qa_client::ui::library::menu::{LibraryEntry, LibraryMenuService};

/// One saved configuration file (donor `scripts.list` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFileEntry {
    /// File name (entry id and label).
    pub name: String,
    /// Seat config (`true`) or selected-game config (`false`).
    pub seat: bool,
}

/// Saved-configuration source (donor `scripts.list`).
pub trait ConfigLibrarySource {
    /// Listing failure type.
    type Error: std::fmt::Display;

    /// List every saved configuration file.
    fn list(&self) -> Result<Vec<ConfigFileEntry>, Self::Error>;
}

/// Config library menu service: browse saved configuration files, `exec`
/// one through an injected sink, or save a new one through `writeconfig`.
pub struct ConfigMenuService<S, E, W> {
    source: S,
    entries: Vec<LibraryEntry>,
    status: String,
    exec: E,
    writeconfig: W,
}

impl<S, E, W> ConfigMenuService<S, E, W>
where
    S: ConfigLibrarySource,
    E: FnMut(&str) -> Result<String, String>,
    W: FnMut(&str) -> Result<(), String>,
{
    /// Build the service over a config source and the exec/writeconfig
    /// command sinks. The exec sink returns the executed script name.
    pub fn new(source: S, exec: E, writeconfig: W) -> Self {
        Self {
            source,
            entries: Vec::new(),
            status: String::new(),
            exec,
            writeconfig,
        }
    }
}

impl<S, E, W> LibraryMenuService for ConfigMenuService<S, E, W>
where
    S: ConfigLibrarySource,
    E: FnMut(&str) -> Result<String, String>,
    W: FnMut(&str) -> Result<(), String>,
{
    fn entries(&self) -> Vec<LibraryEntry> {
        self.entries.clone()
    }

    fn status(&self) -> String {
        self.status.clone()
    }

    fn refresh(&mut self) {
        match self.source.list() {
            Ok(files) => {
                self.entries = files
                    .into_iter()
                    .map(|file| LibraryEntry {
                        id: file.name.clone(),
                        label: file.name,
                        detail: Some(if file.seat { "Current player" } else { "Selected game" }.to_string()),
                        unavailable: None,
                    })
                    .collect();
                self.status = format!("{} configuration files", self.entries.len());
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn activate(&mut self, id: &str) {
        if !self.entries.iter().any(|entry| entry.id == id) {
            return;
        }
        match (self.exec)(id) {
            Ok(name) => self.status = format!("Finished {name}"),
            Err(error) => self.status = error,
        }
    }

    fn create_label(&self) -> Option<String> {
        Some("Save config".to_string())
    }

    fn create_submit(&mut self, name: &str) {
        match (self.writeconfig)(name) {
            Ok(()) => self.status = format!("Saved {name}"),
            Err(error) => self.status = format!("Could not save {name}: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct FakeSource {
        files: Vec<ConfigFileEntry>,
        error: Option<String>,
    }

    impl ConfigLibrarySource for FakeSource {
        type Error = String;

        fn list(&self) -> Result<Vec<ConfigFileEntry>, Self::Error> {
            match &self.error {
                Some(error) => Err(error.clone()),
                None => Ok(self.files.clone()),
            }
        }
    }

    fn source() -> FakeSource {
        FakeSource {
            files: vec![
                ConfigFileEntry {
                    name: "quake.cfg".to_string(),
                    seat: true,
                },
                ConfigFileEntry {
                    name: "server.cfg".to_string(),
                    seat: false,
                },
            ],
            error: None,
        }
    }

    #[allow(clippy::type_complexity)]
    fn service(
        source: FakeSource,
    ) -> ConfigMenuService<FakeSource, impl FnMut(&str) -> Result<String, String>, impl FnMut(&str) -> Result<(), String>>
    {
        ConfigMenuService::new(source, |id: &str| Ok(id.to_string()), |_: &str| Ok(()))
    }

    #[test]
    fn lists_configs_with_owner_detail() {
        let mut service = service(source());
        service.refresh();
        let entries = service.entries();
        let rows: Vec<(&str, &str, Option<&str>)> = entries
            .iter()
            .map(|entry| (entry.id.as_str(), entry.label.as_str(), entry.detail.as_deref()))
            .collect();
        assert_eq!(
            rows,
            [
                ("quake.cfg", "quake.cfg", Some("Current player")),
                ("server.cfg", "server.cfg", Some("Selected game")),
            ]
        );
        assert_eq!(service.status(), "2 configuration files");
    }

    #[test]
    fn refresh_failure_lands_in_status() {
        let mut service = service(FakeSource {
            files: Vec::new(),
            error: Some("store down".to_string()),
        });
        service.refresh();
        assert!(service.entries().is_empty());
        assert_eq!(service.status(), "store down");
    }

    #[test]
    fn activate_execs_listed_entry_and_reports_completion() {
        let executed: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&executed);
        let mut service = ConfigMenuService::new(
            source(),
            move |id: &str| {
                sink.borrow_mut().push(id.to_string());
                Ok(format!("{id}.cfg"))
            },
            |_: &str| Ok(()),
        );
        service.refresh();
        service.activate("server.cfg");
        service.activate("absent.cfg");
        assert_eq!(*executed.borrow(), ["server.cfg"]);
        assert_eq!(service.status(), "Finished server.cfg.cfg");
    }

    #[test]
    fn exec_failure_lands_in_status() {
        let mut service = ConfigMenuService::new(
            source(),
            |_: &str| Err("Not found: server.cfg".to_string()),
            |_: &str| Ok(()),
        );
        service.refresh();
        service.activate("server.cfg");
        assert_eq!(service.status(), "Not found: server.cfg");
    }

    #[test]
    fn create_saves_config_through_writeconfig() {
        let saved: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&saved);
        let mut service = ConfigMenuService::new(
            source(),
            |id: &str| Ok(id.to_string()),
            move |name: &str| {
                sink.borrow_mut().push(name.to_string());
                Ok(())
            },
        );
        assert_eq!(service.create_label().as_deref(), Some("Save config"));
        service.create_submit("fresh.cfg");
        assert_eq!(*saved.borrow(), ["fresh.cfg"]);
        assert_eq!(service.status(), "Saved fresh.cfg");

        let mut service = ConfigMenuService::new(
            source(),
            |id: &str| Ok(id.to_string()),
            |_: &str| Err("disk full".to_string()),
        );
        service.create_submit("fresh.cfg");
        assert_eq!(service.status(), "Could not save fresh.cfg: disk full");
    }
}
