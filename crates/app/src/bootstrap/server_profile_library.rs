//! Server profile library menu service.
//!
//! Donor provenance: `src/app/bootstrap/startup.ts` (`serverProfileLibrary`)
//! over `src/settings/server/library.ts` (`listServerProfiles`).
//!
//! Sync port: refresh lists inline, so no generation guard is needed;
//! failures surface as status text. Profile selection and library dismissal
//! run through injected sinks so frontend wiring stays sync.

use qa_client::ui::library::menu::{LibraryEntry, LibraryMenuService};

/// One server profile record (donor `ServerProfileEntry`): `id` is the
/// profile path, `label` the display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerProfileEntry {
    /// Stable profile identity (profile path).
    pub id: String,
    /// Display label (profile name).
    pub label: String,
}

/// Server-profile source (donor `listServerProfiles`).
pub trait ServerProfileSource {
    /// Collection failure type.
    type Error: std::fmt::Display;

    /// Collect every saved server profile.
    fn collect(&self) -> Result<Vec<ServerProfileEntry>, Self::Error>;
}

/// Server profile library menu service: browse saved server profiles;
/// activating one runs its command through the injected shell sink. The
/// donor keeps the library open and reports the selection in the status.
pub struct ServerProfileMenuService<S, C> {
    profiles: S,
    entries: Vec<LibraryEntry>,
    status: String,
    command: C,
}

impl<S, C> ServerProfileMenuService<S, C>
where
    S: ServerProfileSource,
    C: FnMut(&str) -> Result<(), String>,
{
    /// Build the service over a profile source and a shell command sink.
    pub fn new(profiles: S, command: C) -> Self {
        Self {
            profiles,
            entries: Vec::new(),
            status: String::new(),
            command,
        }
    }
}

impl<S, C> LibraryMenuService for ServerProfileMenuService<S, C>
where
    S: ServerProfileSource,
    C: FnMut(&str) -> Result<(), String>,
{
    fn entries(&self) -> Vec<LibraryEntry> {
        self.entries.clone()
    }

    fn status(&self) -> String {
        self.status.clone()
    }

    fn refresh(&mut self) {
        match self.profiles.collect() {
            Ok(mut profiles) => {
                profiles.sort_by(|left, right| left.label.cmp(&right.label).then(left.id.cmp(&right.id)));
                self.entries = profiles
                    .into_iter()
                    .map(|profile| LibraryEntry {
                        id: profile.id,
                        label: profile.label,
                        detail: None,
                        unavailable: None,
                    })
                    .collect();
                self.status = format!("{} server profiles", self.entries.len());
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn activate(&mut self, id: &str) {
        let Some(label) = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.label.clone())
        else {
            self.status = "Server profile is no longer listed".to_string();
            return;
        };
        if let Err(error) = (self.command)(id) {
            self.status = error;
            return;
        }
        self.status = format!("Selected {label}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct FakeProfiles {
        profiles: Vec<ServerProfileEntry>,
        error: Option<String>,
    }

    impl ServerProfileSource for FakeProfiles {
        type Error = String;

        fn collect(&self) -> Result<Vec<ServerProfileEntry>, Self::Error> {
            match &self.error {
                Some(error) => Err(error.clone()),
                None => Ok(self.profiles.clone()),
            }
        }
    }

    fn profiles() -> FakeProfiles {
        FakeProfiles {
            profiles: vec![
                ServerProfileEntry {
                    id: "servers/vanilla.json".to_string(),
                    label: "vanilla".to_string(),
                },
                ServerProfileEntry {
                    id: "servers/ctf.json".to_string(),
                    label: "ctf".to_string(),
                },
            ],
            error: None,
        }
    }

    #[test]
    fn refresh_lists_profiles_sorted_by_name() {
        let mut service = ServerProfileMenuService::new(profiles(), |_: &str| Ok(()));
        service.refresh();
        let entries = service.entries();
        let rows: Vec<(&str, &str)> = entries
            .iter()
            .map(|entry| (entry.id.as_str(), entry.label.as_str()))
            .collect();
        assert_eq!(rows, [("servers/ctf.json", "ctf"), ("servers/vanilla.json", "vanilla")]);
        assert_eq!(service.status(), "2 server profiles");
    }

    #[test]
    fn activate_runs_command_and_reports_selection() {
        let commands: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink_commands = Rc::clone(&commands);
        let mut service = ServerProfileMenuService::new(profiles(), move |id: &str| {
            sink_commands.borrow_mut().push(id.to_string());
            Ok(())
        });
        service.refresh();
        service.activate("servers/ctf.json");
        assert_eq!(*commands.borrow(), ["servers/ctf.json"]);
        assert_eq!(service.status(), "Selected ctf");
    }

    #[test]
    fn activate_rejects_unlisted_profiles_without_touching_sink() {
        let commands: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink_commands = Rc::clone(&commands);
        let mut service = ServerProfileMenuService::new(profiles(), move |id: &str| {
            sink_commands.borrow_mut().push(id.to_string());
            Ok(())
        });
        service.refresh();
        service.activate("servers/absent.json");
        assert!(commands.borrow().is_empty());
        assert_eq!(service.status(), "Server profile is no longer listed");
    }

    #[test]
    fn failures_land_in_status() {
        let mut service = ServerProfileMenuService::new(
            FakeProfiles {
                profiles: Vec::new(),
                error: Some("store unreadable".to_string()),
            },
            |_: &str| Ok(()),
        );
        service.refresh();
        assert_eq!(service.status(), "store unreadable");

        let mut service = ServerProfileMenuService::new(profiles(), |_: &str| Err("no shell".to_string()));
        service.refresh();
        service.activate("servers/ctf.json");
        assert_eq!(service.status(), "no shell");
    }
}
