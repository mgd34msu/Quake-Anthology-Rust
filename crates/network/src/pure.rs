//! Native Q3 SV_VerifyPaks_f. The VFS supplies existing wire checksum words;
//! command tokenization remains the shared console's responsibility.
use qa_core::primitives::CommandArgs;

pub const PACKAGE_LIMIT: usize = 1024;
pub const REJECTION: &[u8] = b"Unpure client detected. Invalid .PK3 files referenced!";

#[derive(PartialEq, Eq)]
pub struct Policy {
    feed: u32,
    modules: [Option<u32>; 2],
    loaded: Box<[u32]>,
}
impl Policy {
    pub fn new(
        feed: u32,
        modules: [Option<u32>; 2],
        loaded: impl IntoIterator<Item = u32>,
    ) -> Self {
        Self {
            feed,
            modules,
            loaded: loaded.into_iter().take(PACKAGE_LIMIT).collect(),
        }
    }
    fn verify(&self, args: CommandArgs<'_>, server_id: i32) -> Decision {
        let argument = |index: usize| -> &[u8] { args.argv.get(index).copied().unwrap_or(&[]) };
        let word = |index| qa_core::text::atoi_prefix(argument(index)) as u32;
        // A delayed cp from an earlier checksum feed must leave current
        // authentication untouched, even when its remaining fields are bad.
        if (word(1) as i32) < server_id {
            return Decision::Ignored;
        }
        let count = args.argv.len();
        if count < 6
            || count - 5 > PACKAGE_LIMIT
            || self.modules.iter().enumerate().any(|(index, expected)| {
                argument(index + 2).starts_with(b"@") || *expected != Some(word(index + 2))
            })
            || !argument(4).starts_with(b"@")
        {
            return Decision::Rejected;
        }
        let references = count - 6;
        let mut words = [0; PACKAGE_LIMIT];
        let mut checksum = self.feed;
        for index in 0..references {
            let value = word(index + 5);
            if words[..index].contains(&value) || !self.loaded.contains(&value) {
                return Decision::Rejected;
            }
            words[index] = value;
            checksum ^= value;
        }
        if checksum ^ references as u32 == word(count - 1) {
            Decision::Authentic
        } else {
            Decision::Rejected
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Ignored,
    Authentic,
    Rejected,
}

/// One SERVER connection's native admission, reset by gamestate or vdr.
#[derive(Default)]
pub struct Admission {
    policy: Option<Policy>,
    enabled: bool,
    authentic: bool,
    received: bool,
    phase: Phase,
    feed_epoch: Option<(u32, i32)>,
}
#[derive(Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Unmanaged,
    Connected,
    Primed,
    Active,
}
impl Admission {
    pub fn connected(&mut self) {
        self.reset();
        self.phase = Phase::Connected;
    }
    pub fn configure(&mut self, policy: Option<Policy>, feed: u32, server_id: i32) {
        self.enabled = policy.is_some();
        self.policy = policy;
        self.gamestate(feed, server_id);
    }
    /// Rechecking mounted metadata must not reset an unchanged connection.
    pub fn refresh_policy(&mut self, policy: Policy, feed: u32, server_id: i32) -> bool {
        if self.policy.as_ref() == Some(&policy) {
            return false;
        }
        self.configure(Some(policy), feed, server_id);
        true
    }
    pub fn gamestate(&mut self, feed: u32, server_id: i32) {
        if self.feed_epoch.is_none_or(|(old, _)| old != feed) {
            self.feed_epoch = Some((feed, server_id));
        }
        self.reset();
        self.phase = Phase::Primed;
    }
    pub fn reset(&mut self) {
        self.authentic = false;
        self.received = false;
    }
    /// A live sv_pure change preserves admission and its existing native cp.
    /// Keep the load-prepared policy available while enforcement is disabled.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    pub fn verify(&mut self, args: CommandArgs<'_>) -> Decision {
        if !self.enabled {
            return Decision::Ignored;
        }
        let decision = self.policy.as_ref().map_or(Decision::Ignored, |policy| {
            self.feed_epoch
                .filter(|(feed, _)| *feed == policy.feed)
                .map_or(Decision::Rejected, |(_, server_id)| {
                    policy.verify(args, server_id)
                })
        });
        if decision != Decision::Ignored {
            self.received = true;
            self.authentic = decision == Decision::Authentic;
        }
        decision
    }
    pub fn allows_movement(&self) -> bool {
        !self.enabled || self.authentic
    }
    pub fn movement(&mut self) -> Movement {
        if self.phase == Phase::Connected {
            Movement::Wait
        } else if self.allows_movement() {
            if self.phase == Phase::Primed {
                Movement::EnterWorld
            } else {
                Movement::Allowed
            }
        } else if self.phase == Phase::Active && !self.received {
            Movement::ResendGamestate
        } else {
            Movement::Wait
        }
    }
    pub fn entered_world(&mut self) {
        self.phase = Phase::Active;
    }
    pub fn snapshot_flags(&self) -> u8 {
        if matches!(self.phase, Phase::Connected | Phase::Primed) {
            crate::snapshots::Q3_NOT_ACTIVE
        } else {
            0
        }
    }
    pub fn received(&self) -> bool {
        self.received
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Movement {
    Allowed,
    EnterWorld,
    Wait,
    ResendGamestate,
}
