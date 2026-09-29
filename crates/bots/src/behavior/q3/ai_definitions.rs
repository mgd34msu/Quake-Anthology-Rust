//! Q3 bot AI definitions from `src/bots/behavior/q3/ai-definitions.ts`
//! (`game/ai_main.h`, `ai_dmq3.h`, `ai_dmnet.h`, `inv.h`, `chars.h`,
//! `match.h`, `syn.h`).
//!
//! Shared enums and timing constants: team task durations, patrol
//! flags, inventory slots, characteristics, match contexts, message
//! ids, subtypes, variables, and synonym contexts.

/// Bot settings path length.
pub const BOT_SETTINGS_PATH_LENGTH: usize = 144;
/// Maximum proximity mines tracked.
pub const MAX_PROXMINES: usize = 64;
/// Maximum activate stack depth.
pub const MAX_ACTIVATESTACK: usize = 8;
/// Maximum activate areas.
pub const MAX_ACTIVATEAREAS: usize = 32;
/// Maximum waypoints.
pub const MAX_WAYPOINTS: usize = 128;
/// Maximum node switches per frame.
pub const MAX_NODESWITCHES: usize = 50;

/// Bot flag bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotFlag;

impl BotFlag {
    /// Strafe right.
    pub const STRAFERIGHT: i32 = 1;
    /// Was attacked.
    pub const ATTACKED: i32 = 2;
    /// Attack jumped.
    pub const ATTACKJUMPED: i32 = 4;
    /// Aiming at enemy.
    pub const AIMATENEMY: i32 = 8;
    /// Avoid right.
    pub const AVOIDRIGHT: i32 = 16;
    /// Ideal view set.
    pub const IDEALVIEWSET: i32 = 32;
    /// Fight suicidal.
    pub const FIGHTSUICIDAL: i32 = 64;
}

/// Long-term goal types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BotLongTermGoal {
    /// No goal.
    #[default]
    None = 0,
    /// Help a teammate.
    TeamHelp = 1,
    /// Accompany a teammate.
    TeamAccompany = 2,
    /// Defend a key area.
    DefendKeyArea = 3,
    /// Get the flag.
    GetFlag = 4,
    /// Rush the base.
    RushBase = 5,
    /// Return the flag.
    ReturnFlag = 6,
    /// Camp.
    Camp = 7,
    /// Ordered camp.
    CampOrder = 8,
    /// Patrol.
    Patrol = 9,
    /// Get an item.
    GetItem = 10,
    /// Kill someone.
    Kill = 11,
    /// Harvest skulls.
    Harvest = 12,
    /// Attack the enemy base.
    AttackEnemyBase = 13,
    /// Celebrate under.
    MakeLoveUnder = 14,
    /// Celebrate on top.
    MakeLoveOnTop = 15,
}

impl BotLongTermGoal {
    /// Convert from the donor integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Self {
        match value {
            1 => Self::TeamHelp,
            2 => Self::TeamAccompany,
            3 => Self::DefendKeyArea,
            4 => Self::GetFlag,
            5 => Self::RushBase,
            6 => Self::ReturnFlag,
            7 => Self::Camp,
            8 => Self::CampOrder,
            9 => Self::Patrol,
            10 => Self::GetItem,
            11 => Self::Kill,
            12 => Self::Harvest,
            13 => Self::AttackEnemyBase,
            14 => Self::MakeLoveUnder,
            15 => Self::MakeLoveOnTop,
            _ => Self::None,
        }
    }
}

/// Team help duration seconds.
pub const TEAM_HELP_TIME: f32 = 60.0;
/// Team accompany duration seconds.
pub const TEAM_ACCOMPANY_TIME: f32 = 600.0;
/// Defend key area duration seconds.
pub const TEAM_DEFENDKEYAREA_TIME: f32 = 600.0;
/// Camp duration seconds.
pub const TEAM_CAMP_TIME: f32 = 600.0;
/// Patrol duration seconds.
pub const TEAM_PATROL_TIME: f32 = 600.0;
/// Lead duration seconds.
pub const TEAM_LEAD_TIME: f32 = 600.0;
/// Get item duration seconds.
pub const TEAM_GETITEM_TIME: f32 = 60.0;
/// Kill duration seconds.
pub const TEAM_KILL_SOMEONE: f32 = 180.0;
/// Attack enemy base duration seconds.
pub const TEAM_ATTACKENEMYBASE_TIME: f32 = 600.0;
/// Harvest duration seconds.
pub const TEAM_HARVEST_TIME: f32 = 120.0;
/// CTF get flag duration seconds.
pub const CTF_GETFLAG_TIME: f32 = 600.0;
/// CTF rush base duration seconds.
pub const CTF_RUSHBASE_TIME: f32 = 120.0;
/// CTF return flag duration seconds.
pub const CTF_RETURNFLAG_TIME: f32 = 180.0;
/// CTF roam duration seconds.
pub const CTF_ROAM_TIME: f32 = 60.0;

/// Patrol flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotPatrolFlag;

impl BotPatrolFlag {
    /// Loop patrol.
    pub const LOOP: i32 = 1;
    /// Reverse patrol.
    pub const REVERSE: i32 = 2;
    /// Patrol back.
    pub const BACK: i32 = 4;
}

/// Team task preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotTeamTaskPreference {
    /// Defender.
    Defender = 1,
    /// Attacker.
    Attacker = 2,
}

/// CTF strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotCtfStrategy {
    /// Aggressive.
    Aggressive = 1,
}

/// Bot presence type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotPresenceType {
    /// None.
    None = 1,
    /// Normal.
    Normal = 2,
    /// Crouch.
    Crouch = 4,
}

/// CTF flag carrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotCtfFlag {
    /// None.
    None = 0,
    /// Red.
    Red = 1,
    /// Blue.
    Blue = 2,
}

/// Red team skin.
pub const CTF_SKIN_REDTEAM: &str = "red";
/// Blue team skin.
pub const CTF_SKIN_BLUETEAM: &str = "blue";

/// Inventory slots (`inv.h`), including unused holes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotInventory;

impl BotInventory {
    /// None.
    pub const NONE: usize = 0;
    /// Armor.
    pub const ARMOR: usize = 1;
    /// Gauntlet.
    pub const GAUNTLET: usize = 4;
    /// Shotgun.
    pub const SHOTGUN: usize = 5;
    /// Machinegun.
    pub const MACHINEGUN: usize = 6;
    /// Grenade launcher.
    pub const GRENADELAUNCHER: usize = 7;
    /// Rocket launcher.
    pub const ROCKETLAUNCHER: usize = 8;
    /// Lightning gun.
    pub const LIGHTNING: usize = 9;
    /// Railgun.
    pub const RAILGUN: usize = 10;
    /// Plasmagun.
    pub const PLASMAGUN: usize = 11;
    /// BFG10K.
    pub const BFG10K: usize = 13;
    /// Grappling hook.
    pub const GRAPPLINGHOOK: usize = 14;
    /// Nailgun.
    pub const NAILGUN: usize = 15;
    /// Prox launcher.
    pub const PROXLAUNCHER: usize = 16;
    /// Chaingun.
    pub const CHAINGUN: usize = 17;
    /// Shells.
    pub const SHELLS: usize = 18;
    /// Bullets.
    pub const BULLETS: usize = 19;
    /// Grenades.
    pub const GRENADES: usize = 20;
    /// Cells.
    pub const CELLS: usize = 21;
    /// Lightning ammo.
    pub const LIGHTNINGAMMO: usize = 22;
    /// Rockets.
    pub const ROCKETS: usize = 23;
    /// Slugs.
    pub const SLUGS: usize = 24;
    /// BFG ammo.
    pub const BFGAMMO: usize = 25;
    /// Nails.
    pub const NAILS: usize = 26;
    /// Mines.
    pub const MINES: usize = 27;
    /// Belt.
    pub const BELT: usize = 28;
    /// Health.
    pub const HEALTH: usize = 29;
    /// Teleporter.
    pub const TELEPORTER: usize = 30;
    /// Medkit.
    pub const MEDKIT: usize = 31;
    /// Kamikaze.
    pub const KAMIKAZE: usize = 32;
    /// Portal.
    pub const PORTAL: usize = 33;
    /// Invulnerability.
    pub const INVULNERABILITY: usize = 34;
    /// Quad.
    pub const QUAD: usize = 35;
    /// Environment suit.
    pub const ENVIRONMENTSUIT: usize = 36;
    /// Haste.
    pub const HASTE: usize = 37;
    /// Invisibility.
    pub const INVISIBILITY: usize = 38;
    /// Regen.
    pub const REGEN: usize = 39;
    /// Flight.
    pub const FLIGHT: usize = 40;
    /// Scout.
    pub const SCOUT: usize = 41;
    /// Guard.
    pub const GUARD: usize = 42;
    /// Doubler.
    pub const DOUBLER: usize = 43;
    /// Ammo regen.
    pub const AMMOREGEN: usize = 44;
    /// Red flag.
    pub const REDFLAG: usize = 45;
    /// Blue flag.
    pub const BLUEFLAG: usize = 46;
    /// Neutral flag.
    pub const NEUTRALFLAG: usize = 47;
    /// Red cube.
    pub const REDCUBE: usize = 48;
    /// Blue cube.
    pub const BLUECUBE: usize = 49;
    /// Enemy horizontal distance.
    pub const ENEMY_HORIZONTAL_DIST: usize = 200;
    /// Enemy height.
    pub const ENEMY_HEIGHT: usize = 201;
    /// Visible enemies.
    pub const NUM_VISIBLE_ENEMIES: usize = 202;
    /// Visible teammates.
    pub const NUM_VISIBLE_TEAMMATES: usize = 203;
    /// Inventory size.
    pub const SIZE: usize = 204;
}

/// Model indexes (`inv.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotModelIndex;

impl BotModelIndex {
    /// Armor shard.
    pub const ARMORSHARD: i32 = 1;
    /// Armor combat.
    pub const ARMORCOMBAT: i32 = 2;
    /// Armor body.
    pub const ARMORBODY: i32 = 3;
    /// Small health.
    pub const HEALTHSMALL: i32 = 4;
    /// Health.
    pub const HEALTH: i32 = 5;
    /// Large health.
    pub const HEALTHLARGE: i32 = 6;
    /// Mega health.
    pub const HEALTHMEGA: i32 = 7;
    /// Gauntlet.
    pub const GAUNTLET: i32 = 8;
    /// Shotgun.
    pub const SHOTGUN: i32 = 9;
    /// Machinegun.
    pub const MACHINEGUN: i32 = 10;
    /// Grenade launcher.
    pub const GRENADELAUNCHER: i32 = 11;
    /// Rocket launcher.
    pub const ROCKETLAUNCHER: i32 = 12;
    /// Lightning.
    pub const LIGHTNING: i32 = 13;
    /// Railgun.
    pub const RAILGUN: i32 = 14;
    /// Plasmagun.
    pub const PLASMAGUN: i32 = 15;
    /// BFG10K.
    pub const BFG10K: i32 = 16;
    /// Grappling hook.
    pub const GRAPPLINGHOOK: i32 = 17;
    /// Shells.
    pub const SHELLS: i32 = 18;
    /// Bullets.
    pub const BULLETS: i32 = 19;
    /// Grenades.
    pub const GRENADES: i32 = 20;
    /// Cells.
    pub const CELLS: i32 = 21;
    /// Lightning ammo.
    pub const LIGHTNINGAMMO: i32 = 22;
    /// Rockets.
    pub const ROCKETS: i32 = 23;
    /// Slugs.
    pub const SLUGS: i32 = 24;
    /// BFG ammo.
    pub const BFGAMMO: i32 = 25;
    /// Teleporter.
    pub const TELEPORTER: i32 = 26;
    /// Medkit.
    pub const MEDKIT: i32 = 27;
    /// Quad.
    pub const QUAD: i32 = 28;
    /// Environment suit.
    pub const ENVIRONMENTSUIT: i32 = 29;
    /// Haste.
    pub const HASTE: i32 = 30;
    /// Invisibility.
    pub const INVISIBILITY: i32 = 31;
    /// Regen.
    pub const REGEN: i32 = 32;
    /// Flight.
    pub const FLIGHT: i32 = 33;
    /// Red flag.
    pub const REDFLAG: i32 = 34;
    /// Blue flag.
    pub const BLUEFLAG: i32 = 35;
    /// Kamikaze.
    pub const KAMIKAZE: i32 = 36;
    /// Portal.
    pub const PORTAL: i32 = 37;
    /// Invulnerability.
    pub const INVULNERABILITY: i32 = 38;
    /// Nails.
    pub const NAILS: i32 = 39;
    /// Mines.
    pub const MINES: i32 = 40;
    /// Belt.
    pub const BELT: i32 = 41;
    /// Scout.
    pub const SCOUT: i32 = 42;
    /// Guard.
    pub const GUARD: i32 = 43;
    /// Doubler.
    pub const DOUBLER: i32 = 44;
    /// Ammo regen.
    pub const AMMOREGEN: i32 = 45;
    /// Neutral flag.
    pub const NEUTRALFLAG: i32 = 46;
    /// Red cube.
    pub const REDCUBE: i32 = 47;
    /// Blue cube.
    pub const BLUECUBE: i32 = 48;
    /// Nailgun.
    pub const NAILGUN: i32 = 49;
    /// Prox launcher.
    pub const PROXLAUNCHER: i32 = 50;
    /// Chaingun.
    pub const CHAINGUN: i32 = 51;
}

/// Match contexts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotMatchContext;

impl BotMatchContext {
    /// Misc.
    pub const MISC: u32 = 2;
    /// Initial team chat.
    pub const INITIALTEAMCHAT: u32 = 4;
    /// Time.
    pub const TIME: u32 = 8;
    /// Teammate.
    pub const TEAMMATE: u32 = 16;
    /// Addressee.
    pub const ADDRESSEE: u32 = 32;
    /// Patrol key area.
    pub const PATROLKEYAREA: u32 = 64;
    /// Reply chat.
    pub const REPLYCHAT: u32 = 128;
    /// CTF.
    pub const CTF: u32 = 256;
}

/// Team message ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotMessage;

impl BotMessage {
    /// New leader.
    pub const NEWLEADER: i32 = 1;
    /// Enter game.
    pub const ENTERGAME: i32 = 2;
    /// Help.
    pub const HELP: i32 = 3;
    /// Accompany.
    pub const ACCOMPANY: i32 = 4;
    /// Defend key area.
    pub const DEFENDKEYAREA: i32 = 5;
    /// Rush base.
    pub const RUSHBASE: i32 = 6;
    /// Get flag.
    pub const GETFLAG: i32 = 7;
    /// Start team leadership.
    pub const STARTTEAMLEADERSHIP: i32 = 8;
    /// Stop team leadership.
    pub const STOPTEAMLEADERSHIP: i32 = 9;
    /// Who is team leader.
    pub const WHOISTEAMLEADER: i32 = 10;
    /// Wait.
    pub const WAIT: i32 = 11;
    /// What are you doing.
    pub const WHATAREYOUDOING: i32 = 12;
    /// Join subteam.
    pub const JOINSUBTEAM: i32 = 13;
    /// Leave subteam.
    pub const LEAVESUBTEAM: i32 = 14;
    /// Create new formation.
    pub const CREATENEWFORMATION: i32 = 15;
    /// Formation position.
    pub const FORMATIONPOSITION: i32 = 16;
    /// Formation space.
    pub const FORMATIONSPACE: i32 = 17;
    /// Do formation.
    pub const DOFORMATION: i32 = 18;
    /// Dismiss.
    pub const DISMISS: i32 = 19;
    /// Camp.
    pub const CAMP: i32 = 20;
    /// Checkpoint.
    pub const CHECKPOINT: i32 = 21;
    /// Patrol.
    pub const PATROL: i32 = 22;
    /// Lead the way.
    pub const LEADTHEWAY: i32 = 23;
    /// Get item.
    pub const GETITEM: i32 = 24;
    /// Kill.
    pub const KILL: i32 = 25;
    /// Where are you.
    pub const WHEREAREYOU: i32 = 26;
    /// Return flag.
    pub const RETURNFLAG: i32 = 27;
    /// What is my command.
    pub const WHATISMYCOMMAND: i32 = 28;
    /// Which team.
    pub const WHICHTEAM: i32 = 29;
    /// Task preference.
    pub const TASKPREFERENCE: i32 = 30;
    /// Attack enemy base.
    pub const ATTACKENEMYBASE: i32 = 31;
    /// Harvest.
    pub const HARVEST: i32 = 32;
    /// Suicide.
    pub const SUICIDE: i32 = 33;
    /// Me.
    pub const ME: i32 = 100;
    /// Everyone.
    pub const EVERYONE: i32 = 101;
    /// Multiple names.
    pub const MULTIPLENAMES: i32 = 102;
    /// Name.
    pub const NAME: i32 = 103;
    /// Patrol key area.
    pub const PATROLKEYAREA: i32 = 104;
    /// Minutes.
    pub const MINUTES: i32 = 105;
    /// Seconds.
    pub const SECONDS: i32 = 106;
    /// Forever.
    pub const FOREVER: i32 = 107;
    /// For a long time.
    pub const FORALONGTIME: i32 = 108;
    /// For a while.
    pub const FORAWHILE: i32 = 109;
    /// Chat all.
    pub const CHATALL: i32 = 200;
    /// Chat team.
    pub const CHATTEAM: i32 = 201;
    /// Chat tell.
    pub const CHATTELL: i32 = 202;
    /// CTF.
    pub const CTF: i32 = 300;
}

/// Match subtypes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotMatchSubtype;

impl BotMatchSubtype {
    /// Somewhere.
    pub const SOMEWHERE: i32 = 0;
    /// Near item.
    pub const NEARITEM: i32 = 1;
    /// Addressed.
    pub const ADDRESSED: i32 = 2;
    /// Meter.
    pub const METER: i32 = 4;
    /// Feet.
    pub const FEET: i32 = 8;
    /// Time.
    pub const TIME: i32 = 16;
    /// Here.
    pub const HERE: i32 = 32;
    /// There.
    pub const THERE: i32 = 64;
    /// I.
    pub const I: i32 = 128;
    /// More.
    pub const MORE: i32 = 256;
    /// Back.
    pub const BACK: i32 = 512;
    /// Reverse.
    pub const REVERSE: i32 = 1024;
    /// Someone.
    pub const SOMEONE: i32 = 2048;
    /// Got flag.
    pub const GOTFLAG: i32 = 4096;
    /// Captured flag.
    pub const CAPTUREDFLAG: i32 = 8192;
    /// Returned flag.
    pub const RETURNEDFLAG: i32 = 16384;
    /// Team.
    pub const TEAM: i32 = 32768;
    /// Defender.
    pub const DEFENDER: i32 = 1;
    /// Attacker.
    pub const ATTACKER: i32 = 2;
    /// Roamer.
    pub const ROAMER: i32 = 4;
}

/// Match variable slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotMatchVariable;

impl BotMatchVariable {
    /// The enemy.
    pub const THE_ENEMY: usize = 7;
    /// The team.
    pub const THE_TEAM: usize = 7;
    /// Net name.
    pub const NETNAME: usize = 0;
    /// Place.
    pub const PLACE: usize = 1;
    /// Flag.
    pub const FLAG: usize = 1;
    /// Message.
    pub const MESSAGE: usize = 2;
    /// Addressee.
    pub const ADDRESSEE: usize = 2;
    /// Item.
    pub const ITEM: usize = 3;
    /// Teammate.
    pub const TEAMMATE: usize = 4;
    /// Team name.
    pub const TEAMNAME: usize = 4;
    /// Enemy.
    pub const ENEMY: usize = 4;
    /// Key area.
    pub const KEYAREA: usize = 5;
    /// Formation.
    pub const FORMATION: usize = 5;
    /// Position.
    pub const POSITION: usize = 5;
    /// Number.
    pub const NUMBER: usize = 5;
    /// Time.
    pub const TIME: usize = 6;
    /// Name.
    pub const NAME: usize = 6;
    /// More.
    pub const MORE: usize = 6;
}

/// Synonym contexts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotSynonymContext;

impl BotSynonymContext {
    /// All.
    pub const ALL: u32 = 0xffff_ffff;
    /// Normal.
    pub const NORMAL: u32 = 1;
    /// Nearby item.
    pub const NEARBYITEM: u32 = 2;
    /// CTF red team.
    pub const CTFREDTEAM: u32 = 4;
    /// CTF blue team.
    pub const CTFBLUETEAM: u32 = 8;
    /// Reply.
    pub const REPLY: u32 = 16;
    /// Obelisk red team.
    pub const OBELISKREDTEAM: u32 = 32;
    /// Obelisk blue team.
    pub const OBELISKBLUETEAM: u32 = 64;
    /// Harvester red team.
    pub const HARVESTERREDTEAM: u32 = 128;
    /// Harvester blue team.
    pub const HARVESTERBLUETEAM: u32 = 256;
    /// Names.
    pub const NAMES: u32 = 1024;
}

/// Chat escape byte.
pub const BOT_CHAT_ESCAPE: char = '\u{19}';
