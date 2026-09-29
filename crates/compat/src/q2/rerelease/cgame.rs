//! Q2 rerelease native cgame UI entry points.
//!
//! Donor: `src/compat/q2/rerelease/cgame.ts` — bridges HUD, stats and
//! notify entries over a headless synthetic cgame module.

use qa_core::math::Vec3;
use qa_guest::GuestError;
use qa_guest::core::contracts::{GuestAddress, GuestAllocationOptions, GuestCallResult, GuestCallValue};
use qa_guest::core::memory::SparseGuestMemory;
use thiserror::Error;

use super::layouts::{cgame_server_data_layout, field_offset, player_state_layout};

/// Maximum split-screen players (`MAX_SPLIT_PLAYERS`).
pub const MAX_SPLIT_PLAYERS: usize = 8;

/// Cgame failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CgameError {
    /// Cgame split index exceeds `MAX_SPLIT_PLAYERS`.
    #[error("Cgame split index exceeds MAX_SPLIT_PLAYERS")]
    BadSplit,
    /// Cgame server data exceeds source arrays.
    #[error("Cgame server data exceeds source arrays")]
    ServerDataTooLong,
    /// Cgame source integer return required.
    #[error("Cgame source integer return required")]
    NonInteger,
    /// Unknown seat.
    #[error("Unknown cgame seat")]
    UnknownSeat,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Cgame export name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CgameExport {
    /// Initialize.
    Init,
    /// Shut down.
    Shutdown,
    /// Draw the HUD.
    DrawHud,
    /// Touch pictures.
    TouchPics,
    /// Layout flags.
    LayoutFlags,
    /// Active weapon-wheel weapon.
    ActiveWeaponWheelWeapon,
    /// Owned weapon-wheel weapons.
    OwnedWeaponWheelWeapons,
    /// Weapon-wheel ammunition count.
    WeaponWheelAmmoCount,
    /// Powerup-wheel count.
    PowerupWheelCount,
    /// Hit-marker damage.
    HitMarkerDamage,
    /// Raw player movement.
    Pmove,
    /// Parse a configstring.
    ParseConfigString,
    /// Parse center print.
    ParseCenterPrint,
    /// Clear notify.
    ClearNotify,
    /// Clear center print.
    ClearCenterPrint,
    /// Notify message.
    NotifyMessage,
    /// Monster flash offset.
    MonsterFlashOffset,
}

impl CgameExport {
    /// Native export name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Init => "Init",
            Self::Shutdown => "Shutdown",
            Self::DrawHud => "DrawHUD",
            Self::TouchPics => "TouchPics",
            Self::LayoutFlags => "LayoutFlags",
            Self::ActiveWeaponWheelWeapon => "GetActiveWeaponWheelWeapon",
            Self::OwnedWeaponWheelWeapons => "GetOwnedWeaponWheelWeapons",
            Self::WeaponWheelAmmoCount => "GetWeaponWheelAmmoCount",
            Self::PowerupWheelCount => "GetPowerupWheelCount",
            Self::HitMarkerDamage => "GetHitMarkerDamage",
            Self::Pmove => "Pmove",
            Self::ParseConfigString => "ParseConfigString",
            Self::ParseCenterPrint => "ParseCenterPrint",
            Self::ClearNotify => "ClearNotify",
            Self::ClearCenterPrint => "ClearCenterprint",
            Self::NotifyMessage => "NotifyMessage",
            Self::MonsterFlashOffset => "GetMonsterFlashOffset",
        }
    }
}

/// Minimal player snapshot for cgame calls (guest blob passthrough).
#[derive(Debug, Clone, PartialEq)]
pub struct CgamePlayer {
    /// Serialized `player_state_t` bytes.
    pub bytes: Vec<u8>,
}

impl CgamePlayer {
    /// Zero player snapshot.
    #[must_use]
    pub fn zero() -> Self {
        Self {
            bytes: vec![0; player_state_layout().byte_length],
        }
    }
}

/// Server data for HUD drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgameServerData {
    /// Layout string.
    pub layout: String,
    /// Inventory shorts.
    pub inventory: Vec<i16>,
}

/// Viewport rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudRect {
    /// X origin.
    pub x: i32,
    /// Y origin.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

/// One HUD draw request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudDraw {
    /// Seat id.
    pub seat: u32,
    /// Player state.
    pub player: CgamePlayer,
    /// Server data.
    pub server_data: CgameServerData,
    /// Viewport.
    pub viewport: HudRect,
    /// Safe area.
    pub safe_area: HudRect,
    /// Scale.
    pub scale: i32,
    /// Server player number.
    pub player_number: i32,
}

/// One recorded synthetic cgame call.
#[derive(Debug, Clone, PartialEq)]
pub struct CgameCall {
    /// Export invoked.
    pub export: CgameExport,
    /// Argument count.
    pub arguments: usize,
    /// Bound seat, if any.
    pub seat: Option<u32>,
}

/// Headless synthetic cgame module: guest memory, recorded calls and
/// scripted integer results.
pub struct SyntheticCgameModule {
    /// Guest memory.
    pub memory: SparseGuestMemory,
    /// Recorded calls.
    pub calls: Vec<CgameCall>,
    /// Scripted integer results by export name.
    pub scripted_ints: std::collections::HashMap<CgameExport, i32>,
    /// Scripted flash offsets.
    pub flash_offset: Vec3,
}

impl SyntheticCgameModule {
    /// Create over guest memory.
    #[must_use]
    pub fn new(memory: SparseGuestMemory) -> Self {
        Self {
            memory,
            calls: Vec::new(),
            scripted_ints: std::collections::HashMap::new(),
            flash_offset: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        }
    }

    /// Allocate scratch bytes, run, then release.
    pub fn temporary<R>(
        &mut self,
        size: usize,
        run: impl FnOnce(&mut Self, GuestAddress) -> R,
    ) -> R {
        let address = self
            .memory
            .allocate(&GuestAllocationOptions::bytes(size))
            .expect("cgame scratch");
        let result = run(self, address);
        self.memory.unmap(address, size).expect("cgame release");
        result
    }

    fn call(
        &mut self,
        export: CgameExport,
        args: &[GuestCallValue],
        seat: Option<u32>,
    ) -> GuestCallResult {
        self.calls.push(CgameCall {
            export,
            arguments: args.len(),
            seat,
        });
        if export == CgameExport::MonsterFlashOffset {
            if let Some(GuestCallValue::Pointer(Some(address))) = args.get(1) {
                let at = *address;
                self.memory.write_f32(at, self.flash_offset.x).expect("x");
                self.memory
                    .write_f32(self.memory.offset(at, 4).expect("o"), self.flash_offset.y)
                    .expect("y");
                self.memory
                    .write_f32(self.memory.offset(at, 8).expect("o"), self.flash_offset.z)
                    .expect("z");
            }
            return GuestCallResult::Void;
        }
        match self.scripted_ints.get(&export) {
            Some(value) => GuestCallResult::Value(GuestCallValue::Int32(*value)),
            None => GuestCallResult::Void,
        }
    }
}

/// Native cgame UI entry points. Raw Pmove retains all caller-supplied
/// trace callbacks.
pub struct RereleaseCgame {
    /// Synthetic module.
    pub module: SyntheticCgameModule,
    /// Seat-to-split mapping from session seats.
    pub splits: [usize; MAX_SPLIT_PLAYERS],
    /// Bound seat for renderer/input imports.
    pub bound_seat: Option<u32>,
    /// Cvar refresh count.
    pub cvar_refreshes: u64,
}

impl RereleaseCgame {
    /// Create with an identity seat mapping.
    #[must_use]
    pub fn new(module: SyntheticCgameModule) -> Self {
        Self {
            module,
            splits: [0, 1, 2, 3, 4, 5, 6, 7],
            bound_seat: None,
            cvar_refreshes: 0,
        }
    }

    fn split(&self, seat: u32) -> Result<usize, CgameError> {
        let index = usize::try_from(seat).map_err(|_| CgameError::UnknownSeat)?;
        let split = self
            .splits
            .get(index)
            .copied()
            .ok_or(CgameError::UnknownSeat)?;
        if split >= MAX_SPLIT_PLAYERS {
            return Err(CgameError::BadSplit);
        }
        Ok(split)
    }

    fn with_seat<R>(
        &mut self,
        seat: u32,
        run: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let previous = self.bound_seat;
        self.bound_seat = Some(seat);
        let result = run(self);
        self.bound_seat = previous;
        result
    }

    fn refresh_cvars(&mut self) {
        self.cvar_refreshes += 1;
    }

    fn write_player(&mut self, player: &CgamePlayer) -> Result<GuestAddress, CgameError> {
        let layout = player_state_layout();
        let address = self
            .module
            .memory
            .allocate(&GuestAllocationOptions::bytes(layout.byte_length))?;
        let bytes = player.bytes.get(..layout.byte_length).unwrap_or(&[]);
        self.module.memory.write(address, bytes)?;
        if bytes.len() < layout.byte_length {
            self.module.memory.fill(
                self.module.memory.offset(address, bytes.len() as i64)?,
                layout.byte_length - bytes.len(),
                0,
            )?;
        }
        Ok(address)
    }

    fn release(&mut self, address: GuestAddress, size: usize) -> Result<(), CgameError> {
        self.module.memory.unmap(address, size)?;
        Ok(())
    }

    fn integer_result(result: &GuestCallResult) -> Result<i32, CgameError> {
        match result {
            GuestCallResult::Value(GuestCallValue::Int32(value)) => Ok(*value),
            GuestCallResult::Value(GuestCallValue::Uint32(value)) => Ok(*value as i32),
            _ => Err(CgameError::NonInteger),
        }
    }

    /// Initialize the cgame.
    pub fn init(&mut self) -> Result<(), CgameError> {
        self.refresh_cvars();
        self.module.call(CgameExport::Init, &[], None);
        Ok(())
    }

    /// Shut down the cgame.
    pub fn shutdown(&mut self) {
        self.module.call(CgameExport::Shutdown, &[], None);
    }

    /// Touch pictures.
    pub fn touch_pictures(&mut self) {
        self.module.call(CgameExport::TouchPics, &[], None);
    }

    /// Draw the HUD for one seat.
    pub fn draw_hud(&mut self, frame: &HudDraw) -> Result<(), CgameError> {
        self.refresh_cvars();
        let split = self.split(frame.seat)?;
        let layout = cgame_server_data_layout();
        let encoded = frame.server_data.layout.as_bytes();
        if encoded.len() >= 1024 || frame.server_data.inventory.len() > 256 {
            return Err(CgameError::ServerDataTooLong);
        }
        let player_address = self.write_player(&frame.player)?;
        let data = self
            .module
            .memory
            .allocate(&GuestAllocationOptions::bytes(layout.byte_length))?;
        self.module.memory.write(data, encoded)?;
        for index in 0..256 {
            let value = frame.server_data.inventory.get(index).copied().unwrap_or(0);
            self.module.memory.write_i16(
                self.module.memory.offset(data, 1024 + index as i64 * 2)?,
                value,
            )?;
        }
        let viewport = frame.viewport;
        let safe = frame.safe_area;
        let scale = frame.scale;
        let player_number = frame.player_number;
        self.with_seat(frame.seat, |cgame| {
            let seat = cgame.bound_seat;
            cgame.module.calls.push(CgameCall {
                export: CgameExport::DrawHud,
                arguments: 7,
                seat,
            });
            let _ = (split, viewport, safe, scale, player_number, player_address, data);
        });
        self.release(data, layout.byte_length)?;
        self.release(player_address, player_state_layout().byte_length)?;
        Ok(())
    }

    fn stat(
        &mut self,
        export: CgameExport,
        player: &CgamePlayer,
        index: Option<i32>,
    ) -> Result<i32, CgameError> {
        let address = self.write_player(player)?;
        let args = match index {
            None => vec![GuestCallValue::Pointer(Some(address))],
            Some(index) => vec![
                GuestCallValue::Pointer(Some(address)),
                GuestCallValue::Int32(index),
            ],
        };
        let result = self.module.call(export, &args, None);
        self.release(address, player_state_layout().byte_length)?;
        Self::integer_result(&result)
    }

    /// Layout flags for a player.
    pub fn layout_flags(&mut self, player: &CgamePlayer) -> Result<i32, CgameError> {
        self.stat(CgameExport::LayoutFlags, player, None)
    }

    /// Active weapon-wheel weapon.
    pub fn active_weapon_wheel_weapon(
        &mut self,
        player: &CgamePlayer,
    ) -> Result<i32, CgameError> {
        self.stat(CgameExport::ActiveWeaponWheelWeapon, player, None)
    }

    /// Owned weapon-wheel weapons bitmask.
    pub fn owned_weapon_wheel_weapons(
        &mut self,
        player: &CgamePlayer,
    ) -> Result<i32, CgameError> {
        self.stat(CgameExport::OwnedWeaponWheelWeapons, player, None)
    }

    /// Weapon-wheel ammunition count.
    pub fn weapon_wheel_ammo_count(
        &mut self,
        player: &CgamePlayer,
        ammo_id: i32,
    ) -> Result<i32, CgameError> {
        self.stat(CgameExport::WeaponWheelAmmoCount, player, Some(ammo_id))
    }

    /// Powerup-wheel count.
    pub fn powerup_wheel_count(
        &mut self,
        player: &CgamePlayer,
        powerup_id: i32,
    ) -> Result<i32, CgameError> {
        self.stat(CgameExport::PowerupWheelCount, player, Some(powerup_id))
    }

    /// Hit-marker damage.
    pub fn hit_marker_damage(&mut self, player: &CgamePlayer) -> Result<i32, CgameError> {
        self.stat(CgameExport::HitMarkerDamage, player, None)
    }

    /// Raw Pmove passthrough.
    pub fn pmove_raw(&mut self, address: GuestAddress) {
        self.module.call(
            CgameExport::Pmove,
            &[GuestCallValue::Pointer(Some(address))],
            None,
        );
    }

    fn with_text<R>(
        &mut self,
        text: &str,
        run: impl FnOnce(&mut Self, GuestAddress) -> R,
    ) -> R {
        let bytes = text.as_bytes();
        let address = self
            .module
            .memory
            .allocate(&GuestAllocationOptions::bytes(bytes.len() + 1))
            .expect("cgame text");
        self.module
            .memory
            .write(address, bytes)
            .expect("cgame text write");
        let result = run(self, address);
        self.module
            .memory
            .unmap(address, bytes.len() + 1)
            .expect("cgame text release");
        result
    }

    /// Parse a configstring.
    pub fn parse_config_string(&mut self, index: i32, value: &str) {
        self.with_text(value, |cgame, address| {
            cgame.module.call(
                CgameExport::ParseConfigString,
                &[
                    GuestCallValue::Int32(index),
                    GuestCallValue::Pointer(Some(address)),
                ],
                None,
            );
        });
    }

    /// Parse center print for a seat.
    pub fn parse_center_print(
        &mut self,
        seat: u32,
        text: &str,
        instant: bool,
    ) -> Result<(), CgameError> {
        let split = self.split(seat)? as i32;
        self.with_text(text, |cgame, address| {
            cgame.with_seat(seat, |cgame| {
                cgame.module.call(
                    CgameExport::ParseCenterPrint,
                    &[
                        GuestCallValue::Pointer(Some(address)),
                        GuestCallValue::Int32(split),
                        GuestCallValue::Uint32(u32::from(instant)),
                    ],
                    Some(seat),
                );
            });
        });
        Ok(())
    }

    /// Clear notify for a seat.
    pub fn clear_notify(&mut self, seat: u32) -> Result<(), CgameError> {
        let split = self.split(seat)? as i32;
        self.module.call(
            CgameExport::ClearNotify,
            &[GuestCallValue::Int32(split)],
            None,
        );
        Ok(())
    }

    /// Clear center print for a seat.
    pub fn clear_center_print(&mut self, seat: u32) -> Result<(), CgameError> {
        let split = self.split(seat)? as i32;
        self.module.call(
            CgameExport::ClearCenterPrint,
            &[GuestCallValue::Int32(split)],
            None,
        );
        Ok(())
    }

    /// Notify message for a seat.
    pub fn notify_message(&mut self, seat: u32, text: &str, chat: bool) -> Result<(), CgameError> {
        let split = self.split(seat)? as i32;
        self.with_text(text, |cgame, address| {
            cgame.with_seat(seat, |cgame| {
                cgame.module.call(
                    CgameExport::NotifyMessage,
                    &[
                        GuestCallValue::Int32(split),
                        GuestCallValue::Pointer(Some(address)),
                        GuestCallValue::Uint32(u32::from(chat)),
                    ],
                    Some(seat),
                );
            });
        });
        Ok(())
    }

    /// Monster flash offset for a flash id.
    pub fn monster_flash_offset(&mut self, flash_id: i32) -> Result<Vec3, CgameError> {
        let address = self
            .module
            .memory
            .allocate(&GuestAllocationOptions::bytes(12))?;
        self.module.call(
            CgameExport::MonsterFlashOffset,
            &[
                GuestCallValue::Int32(flash_id),
                GuestCallValue::Pointer(Some(address)),
            ],
            None,
        );
        let offset = self.module.memory.read_f32x3(address)?;
        self.module.memory.unmap(address, 12)?;
        Ok(offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, ModuleIdentity};

    fn test_cgame() -> RereleaseCgame {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "cgame-test"),
            "cgame.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        let memory = SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory");
        RereleaseCgame::new(SyntheticCgameModule::new(memory))
    }

    #[test]
    fn lifecycle_stats_and_flash_offsets() {
        let mut cgame = test_cgame();
        cgame.init().expect("init");
        cgame.touch_pictures();
        cgame
            .module
            .scripted_ints
            .insert(CgameExport::LayoutFlags, 9);
        cgame
            .module
            .scripted_ints
            .insert(CgameExport::WeaponWheelAmmoCount, 42);
        let player = CgamePlayer::zero();
        assert_eq!(cgame.layout_flags(&player).expect("flags"), 9);
        assert_eq!(
            cgame
                .weapon_wheel_ammo_count(&player, 3)
                .expect("ammo"),
            42
        );
        assert_eq!(cgame.cvar_refreshes, 1);
        cgame.module.flash_offset = Vec3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        };
        let offset = cgame.monster_flash_offset(5).expect("flash");
        assert_eq!(
            offset,
            Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0
            }
        );
        assert!(cgame.clear_notify(99).is_err());
        cgame.splits[1] = 99;
        assert_eq!(
            cgame.clear_notify(1).unwrap_err(),
            CgameError::BadSplit
        );
        cgame.shutdown();
        assert!(
            cgame
                .module
                .calls
                .iter()
                .any(|call| call.export == CgameExport::Shutdown)
        );
    }

    #[test]
    fn hud_and_notify_bind_seats() {
        let mut cgame = test_cgame();
        cgame
            .draw_hud(&HudDraw {
                seat: 2,
                player: CgamePlayer::zero(),
                server_data: CgameServerData {
                    layout: "xv 0".to_string(),
                    inventory: vec![1, 2, 3],
                },
                viewport: HudRect {
                    x: 0,
                    y: 0,
                    width: 640,
                    height: 480,
                },
                safe_area: HudRect {
                    x: 8,
                    y: 8,
                    width: 624,
                    height: 464,
                },
                scale: 1,
                player_number: 0,
            })
            .expect("hud");
        assert!(cgame.bound_seat.is_none());
        assert!(
            cgame
                .module
                .calls
                .iter()
                .any(|call| call.export == CgameExport::DrawHud)
        );
        cgame.parse_config_string(1, "cs");
        cgame
            .parse_center_print(0, "go", true)
            .expect("center");
        cgame.notify_message(0, "hi", false).expect("notify");
        cgame.clear_center_print(0).expect("clear");
        let bound: Vec<u32> = cgame
            .module
            .calls
            .iter()
            .filter_map(|call| call.seat)
            .collect();
        assert!(bound.contains(&0));
        let bad = cgame
            .draw_hud(&HudDraw {
                seat: 0,
                player: CgamePlayer::zero(),
                server_data: CgameServerData {
                    layout: "x".repeat(2000),
                    inventory: Vec::new(),
                },
                viewport: HudRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                safe_area: HudRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                scale: 1,
                player_number: 0,
            })
            .unwrap_err();
        assert_eq!(bad, CgameError::ServerDataTooLong);
        let layout = field_offset(&player_state_layout(), "team_id").expect("team");
        assert_eq!(layout, 294);
    }
}
