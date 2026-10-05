//! Movie library menu service over mounted videos.
//!
//! Donor provenance: `src/app/bootstrap/startup.ts` (`movieLibraryMenu`).
//!
//! Sync port: refresh lists inline, so no generation guard is needed; sink
//! failures surface as status text. Tier gating matches the donor exactly
//! (tier videos unlock through `g_spVideos`) and applies only when the
//! host reports a Quake III client, exactly the donor's dialect gate.

use qa_client::ui::library::menu::{LibraryEntry, LibraryMenuService};
use qa_compat::userinfo::info_value_for_key;
use qa_content::q3::base::game::items_core::game_atoi;

/// Mounted-video source (donor `scripts.listMounted`).
pub trait MovieLibraryMounts {
    /// Mount failure type.
    type Error: std::fmt::Display;

    /// List mounted files with an extension under a directory.
    fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, Self::Error>;
}

/// Movie library menu service: browse `video/` movies, play and stop
/// through injected command sinks.
pub struct MovieMenuService<M, G, V, P, S> {
    mounts: M,
    gated: G,
    videos: V,
    entries: Vec<LibraryEntry>,
    status: String,
    play: P,
    stop: S,
}

impl<M, G, V, P, S> MovieMenuService<M, G, V, P, S>
where
    M: MovieLibraryMounts,
    G: Fn() -> bool,
    V: Fn() -> String,
    P: FnMut(&str) -> Result<(), String>,
    S: FnMut() -> Result<(), String>,
{
    /// Build the service over mounts, the Quake III gate and videos
    /// readers, and cinematic command sinks.
    pub fn new(mounts: M, gated: G, videos: V, play: P, stop: S) -> Self {
        Self {
            mounts,
            gated,
            videos,
            entries: Vec::new(),
            status: String::new(),
            play,
            stop,
        }
    }

    /// Tier number gating a video name (donor `/^tier([1-7])\.roq$/`,
    /// `end.roq`/`demoend.roq` as tier 8).
    fn gated_tier(id: &str) -> Option<String> {
        let base = id.rsplit('/').next().unwrap_or(id).to_lowercase();
        if base.len() == 9 && base.starts_with("tier") && base.ends_with(".roq") {
            let tier = &base[4..5];
            if ('1'..='7').contains(&tier.chars().next().unwrap_or('0')) {
                return Some(tier.to_string());
            }
            return None;
        }
        if base == "end.roq" || base == "demoend.roq" {
            return Some("8".to_string());
        }
        None
    }

    /// Entries with tier gating applied (donor `visibleEntries`).
    fn visible(&self) -> Vec<LibraryEntry> {
        if !(self.gated)() {
            return self.entries.clone();
        }
        let videos = (self.videos)();
        self.entries
            .iter()
            .map(|entry| {
                let locked = Self::gated_tier(&entry.id).is_some_and(|tier| {
                    let value = info_value_for_key(&videos, &format!("tier{tier}"), 8192).unwrap_or_default();
                    game_atoi(&value) == 0
                });
                LibraryEntry {
                    unavailable: locked.then(|| "Complete the single-player tier to unlock".to_string()),
                    ..entry.clone()
                }
            })
            .collect()
    }
}

impl<M, G, V, P, S> LibraryMenuService for MovieMenuService<M, G, V, P, S>
where
    M: MovieLibraryMounts,
    G: Fn() -> bool,
    V: Fn() -> String,
    P: FnMut(&str) -> Result<(), String>,
    S: FnMut() -> Result<(), String>,
{
    fn entries(&self) -> Vec<LibraryEntry> {
        self.visible()
    }

    fn status(&self) -> String {
        self.status.clone()
    }

    fn refresh(&mut self) {
        let mut names = std::collections::BTreeSet::new();
        for extension in [".roq", ".cin", ".ogv"] {
            match self.mounts.list_files("video", extension) {
                Ok(files) => names.extend(files),
                Err(error) => {
                    self.status = error.to_string();
                    return;
                }
            }
        }
        self.entries = names
            .into_iter()
            .map(|name| LibraryEntry {
                id: format!("video/{name}"),
                label: name,
                detail: None,
                unavailable: None,
            })
            .collect();
        self.status = format!("{} movies", self.entries.len());
    }

    fn activate(&mut self, id: &str) {
        let available = self
            .visible()
            .iter()
            .any(|entry| entry.id == id && entry.unavailable.is_none());
        if !available {
            return;
        }
        if let Err(error) = (self.play)(id) {
            self.status = error;
        }
    }

    fn stop_label(&self) -> Option<String> {
        Some("Stop movie".to_string())
    }

    fn stop_activate(&mut self) {
        if let Err(error) = (self.stop)() {
            self.status = error;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    struct FakeMounts {
        files: HashMap<(String, String), Vec<String>>,
    }

    impl MovieLibraryMounts for FakeMounts {
        type Error = String;

        fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, Self::Error> {
            Ok(self
                .files
                .get(&(directory.to_string(), extension.to_string()))
                .cloned()
                .unwrap_or_default())
        }
    }

    fn mounts() -> FakeMounts {
        let mut files = HashMap::new();
        files.insert(
            ("video".to_string(), ".roq".to_string()),
            vec!["tier1.roq".to_string(), "intro.roq".to_string(), "end.roq".to_string()],
        );
        files.insert(("video".to_string(), ".cin".to_string()), vec!["quake.cin".to_string()]);
        FakeMounts { files }
    }

    #[test]
    fn lists_deduped_sorted_movies() {
        let mut service = MovieMenuService::new(mounts(), || false, String::new, |_: &str| Ok(()), || Ok(()));
        service.refresh();
        let entries = service.entries();
        let ids: Vec<&str> = entries.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(
            ids,
            ["video/end.roq", "video/intro.roq", "video/quake.cin", "video/tier1.roq"]
        );
        assert_eq!(service.status(), "4 movies");
        assert!(service.entries().iter().all(|entry| entry.unavailable.is_none()));
    }

    #[test]
    fn tier_videos_gate_on_sp_videos_with_q3_client() {
        let mut service = MovieMenuService::new(
            mounts(),
            || true,
            || "\\tier1\\1\\tier8\\0".to_string(),
            |_: &str| Ok(()),
            || Ok(()),
        );
        service.refresh();
        let entries = service.entries();
        let locked: Vec<(&str, bool)> = entries
            .iter()
            .map(|entry| (entry.id.as_str(), entry.unavailable.is_some()))
            .collect();
        assert_eq!(
            locked,
            [
                ("video/end.roq", true),
                ("video/intro.roq", false),
                ("video/quake.cin", false),
                ("video/tier1.roq", false),
            ]
        );
    }

    #[test]
    fn activate_skips_locked_entries_and_reports_sink_errors() {
        let played: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&played);
        let mut service = MovieMenuService::new(
            mounts(),
            || true,
            String::new,
            move |id: &str| {
                sink.borrow_mut().push(id.to_string());
                Ok(())
            },
            || Err("no stop".to_string()),
        );
        service.refresh();
        service.activate("video/end.roq");
        service.activate("video/intro.roq");
        assert_eq!(*played.borrow(), ["video/intro.roq"]);
        service.stop_activate();
        assert_eq!(service.status(), "no stop");
    }
}
