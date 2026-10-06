//! Native Q1 status bar layout (WinQuake `sbar.c`).
//!
//! Stock `Sbar_Draw`/`Sbar_DrawInventory`/`Sbar_DrawFrags`/`Sbar_DrawFace`,
//! the solo scoreboard, the deathmatch overlays, and
//! `Sbar_IntermissionOverlay`, over a caller-supplied frame of live
//! client state. Coordinates are absolute viewport pixels from the
//! top-left: the bar anchors to the viewport bottom (`y + height - 24`
//! for bar rows) and to the horizontal center in single player
//! (`x + (width - 320) / 2`, truncated like the `>> 1`), while
//! deathmatch draws the bar uncentered. Hipnotic and rogue branches
//! follow the same `sbar.c` code, selected by [`Q1SbarProduct`].
//!
//! All arithmetic mirrors the C: `Sbar_itoa` truncation, `Sbar_DrawNum`
//! digit packing (24 px per digit, right-aligned in the field),
//! weapon-flash timing (`(time - gettime) * 10`), the 2 s item flash
//! window, and the insertion-ish frag sort (bubble by frags, stable
//! for ties).

/// Stock bar width in pixels.
pub const Q1_SBAR_WIDTH: i32 = 320;
/// Stock main-bar height in pixels (`SBAR_HEIGHT`).
pub const Q1_SBAR_HEIGHT: i32 = 24;
/// Stock full bar (inventory plus main) in pixels.
pub const Q1_SBAR_FULL: i32 = 48;
/// `STAT_MINUS`: num frame for `-` digits.
pub const Q1_STAT_MINUS: usize = 10;
/// Stock `MAX_CL_STATS`.
pub const Q1_MAX_STATS: usize = 32;
/// Stock `MAX_SCOREBOARD`.
pub const Q1_MAX_SCOREBOARD: usize = 16;

/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_HEALTH: usize = 0;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_FRAGS: usize = 1;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_WEAPON: usize = 2;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_AMMO: usize = 3;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_ARMOR: usize = 4;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_WEAPONFRAME: usize = 5;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_SHELLS: usize = 6;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_NAILS: usize = 7;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_ROCKETS: usize = 8;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_CELLS: usize = 9;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_ACTIVEWEAPON: usize = 10;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_TOTALSECRETS: usize = 11;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_TOTALMONSTERS: usize = 12;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_SECRETS: usize = 13;
/// Stock stat indices (`quakedef.h:120-134`).
pub const Q1_STAT_MONSTERS: usize = 14;

/// Stock item bits (`quakedef.h:138-166`, `defs.qc:285-306`).
pub const Q1_IT_SHOTGUN: u32 = 1;
/// Stock item bits.
pub const Q1_IT_SUPER_SHOTGUN: u32 = 2;
/// Stock item bits.
pub const Q1_IT_NAILGUN: u32 = 4;
/// Stock item bits.
pub const Q1_IT_SUPER_NAILGUN: u32 = 8;
/// Stock item bits.
pub const Q1_IT_GRENADE_LAUNCHER: u32 = 16;
/// Stock item bits.
pub const Q1_IT_ROCKET_LAUNCHER: u32 = 32;
/// Stock item bits.
pub const Q1_IT_LIGHTNING: u32 = 64;
/// Stock item bits.
pub const Q1_IT_SUPER_LIGHTNING: u32 = 128;
/// Stock item bits.
pub const Q1_IT_SHELLS: u32 = 256;
/// Stock item bits.
pub const Q1_IT_NAILS: u32 = 512;
/// Stock item bits.
pub const Q1_IT_ROCKETS: u32 = 1024;
/// Stock item bits.
pub const Q1_IT_CELLS: u32 = 2048;
/// Stock item bits.
pub const Q1_IT_AXE: u32 = 4096;
/// Stock item bits.
pub const Q1_IT_ARMOR1: u32 = 8192;
/// Stock item bits.
pub const Q1_IT_ARMOR2: u32 = 16384;
/// Stock item bits.
pub const Q1_IT_ARMOR3: u32 = 32768;
/// Stock item bits.
pub const Q1_IT_SUPERHEALTH: u32 = 65536;
/// Stock item bits.
pub const Q1_IT_KEY1: u32 = 131072;
/// Stock item bits.
pub const Q1_IT_KEY2: u32 = 262144;
/// Stock item bits.
pub const Q1_IT_INVISIBILITY: u32 = 524288;
/// Stock item bits.
pub const Q1_IT_INVULNERABILITY: u32 = 1048576;
/// Stock item bits.
pub const Q1_IT_SUIT: u32 = 2097152;
/// Stock item bits.
pub const Q1_IT_QUAD: u32 = 4194304;
/// Stock item bits.
pub const Q1_IT_SIGIL1: u32 = 1 << 28;
/// Stock item bits.
pub const Q1_IT_SIGIL2: u32 = 1 << 29;
/// Stock item bits.
pub const Q1_IT_SIGIL3: u32 = 1 << 30;
/// Stock item bits.
pub const Q1_IT_SIGIL4: u32 = 1 << 31;

/// Rogue item bits (`quakedef.h:173-187`, product-selected).
pub const Q1_RIT_SHELLS: u32 = 128;
/// Rogue item bits.
pub const Q1_RIT_NAILS: u32 = 256;
/// Rogue item bits.
pub const Q1_RIT_ROCKETS: u32 = 512;
/// Rogue item bits.
pub const Q1_RIT_CELLS: u32 = 1024;
/// Rogue item bits.
pub const Q1_RIT_AXE: u32 = 2048;
/// Rogue item bits.
pub const Q1_RIT_LAVA_NAILGUN: u32 = 4096;
/// Rogue item bits.
pub const Q1_RIT_LAVA_SUPER_NAILGUN: u32 = 8192;
/// Rogue item bits.
pub const Q1_RIT_MULTI_GRENADE: u32 = 16384;
/// Rogue item bits.
pub const Q1_RIT_MULTI_ROCKET: u32 = 32768;
/// Rogue item bits.
pub const Q1_RIT_PLASMA_GUN: u32 = 65536;
/// Rogue item bits.
pub const Q1_RIT_ARMOR1: u32 = 8388608;
/// Rogue item bits.
pub const Q1_RIT_ARMOR2: u32 = 16777216;
/// Rogue item bits.
pub const Q1_RIT_ARMOR3: u32 = 33554432;
/// Rogue item bits.
pub const Q1_RIT_LAVA_NAILS: u32 = 67108864;
/// Rogue item bits.
pub const Q1_RIT_PLASMA_AMMO: u32 = 134217728;
/// Rogue item bits.
pub const Q1_RIT_MULTI_ROCKETS: u32 = 268435456;

/// Hipnotic weapon bit positions (`quakedef.h:193-196`,
/// `sbar.c:60`): laser, mjolnir, grenade/prox slot, proximity gun.
pub const Q1_HIPWEAPONS: [u32; 4] = [23, 7, 4, 16];

/// Weapon inventory lump stems (`sbar.c:127-152`).
pub const Q1_WEAPON_STEMS: [&str; 7] = [
    "shotgun", "sshotgun", "nailgun", "snailgun", "rlaunch", "srlaunch", "lightng",
];

/// Hipnotic weapon lump stems (`sbar.c:201-220`).
pub const Q1_HIPNOTIC_STEMS: [&str; 5] = ["laser", "mjolnir", "gren_prox", "prox_gren", "prox"];

/// Rogue powered-weapon lumps (`sbar.c:231-235`).
pub const Q1_ROGUE_WEAPONS: [&str; 5] = ["r_lava", "r_superlava", "r_gren", "r_multirock", "r_plasma"];

/// Ammo icon lumps (`sbar.c:154-157`).
pub const Q1_AMMO_LUMPS: [&str; 4] = ["sb_shells", "sb_nails", "sb_rocket", "sb_cells"];

/// Rogue ammo icon lumps (`sbar.c:244-246`).
pub const Q1_ROGUE_AMMO_LUMPS: [&str; 3] = ["r_ammolava", "r_ammomulti", "r_ammoplasma"];

/// Armor icon lumps (`sbar.c:159-161`).
pub const Q1_ARMOR_LUMPS: [&str; 3] = ["sb_armor1", "sb_armor2", "sb_armor3"];

/// Item icon lumps (`sbar.c:163-168`): keys, ring, pent, suit, quad.
pub const Q1_ITEM_LUMPS: [&str; 6] = ["sb_key1", "sb_key2", "sb_invis", "sb_invuln", "sb_suit", "sb_quad"];

/// Sigil lumps (`sbar.c:170-173`).
pub const Q1_SIGIL_LUMPS: [&str; 4] = ["sb_sigil1", "sb_sigil2", "sb_sigil3", "sb_sigil4"];

/// Hipnotic item lumps (`sbar.c:222-223`): wetsuit, empathy shield.
pub const Q1_HIPNOTIC_ITEM_LUMPS: [&str; 2] = ["sb_wsuit", "sb_eshld"];

/// Rogue item lumps (`sbar.c:237-238`): shield, antigrav.
pub const Q1_ROGUE_ITEM_LUMPS: [&str; 2] = ["r_shield1", "r_agrav1"];

/// Content product selecting the sbar branches (`hipnotic`/`rogue`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1SbarProduct {
    /// id1.
    Id1,
    /// Hipnotic (Scourge of Armagon).
    Hipnotic,
    /// Rogue (Dissolution of Eternity).
    Rogue,
}

/// One deathmatch scoreboard entry (`scoreboard_t` subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1ScoreEntry {
    /// Player name (empty slots are skipped).
    pub name: String,
    /// Frags.
    pub frags: i32,
    /// Shirt/pants colors (`colors`).
    pub colors: u8,
}

/// Live client state for one status-bar frame (`cl` subset).
#[derive(Debug, Clone, PartialEq)]
pub struct NativeQ1HudFrame {
    /// Client stats (`cl.stats`, `STAT_*` indices).
    pub stats: [i32; Q1_MAX_STATS],
    /// Carried item bits (`cl.items`).
    pub items: u32,
    /// Pickup times by item bit for bar flashes (`cl.item_gettime`).
    pub item_gettime: [f32; 32],
    /// Client time in seconds (`cl.time`).
    pub time: f32,
    /// Face pain-frame expiry (`cl.faceanimtime`).
    pub face_anim_until: f32,
    /// Level name (`cl.levelname`).
    pub levelname: String,
    /// Deathmatch rules (`cl.gametype == GAME_DEATHMATCH`).
    pub deathmatch: bool,
    /// Client slots (`cl.maxclients`).
    pub maxclients: i32,
    /// View entity, 1-based (`cl.viewentity`).
    pub viewentity: i32,
    /// Scores key held (`sb_showscores`).
    pub showscores: bool,
    /// Scoreboard slots by client index (`cl.scores`).
    pub scores: Vec<Q1ScoreEntry>,
    /// Content product (`hipnotic`/`rogue`).
    pub product: Q1SbarProduct,
    /// `teamplay` cvar (rogue CTF face branch).
    pub teamplay: f32,
    /// Completed level time in seconds (`cl.completed_time`).
    pub completed_time: f32,
}

impl Default for NativeQ1HudFrame {
    fn default() -> Self {
        Self {
            stats: [0; Q1_MAX_STATS],
            items: 0,
            item_gettime: [0.0; 32],
            time: 0.0,
            face_anim_until: 0.0,
            levelname: String::new(),
            deathmatch: false,
            maxclients: 1,
            viewentity: 1,
            showscores: false,
            scores: Vec::new(),
            product: Q1SbarProduct::Id1,
            teamplay: 0.0,
            completed_time: 0.0,
        }
    }
}

/// One stock status-bar draw in absolute viewport pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeQ1HudOperation {
    /// Opaque picture (`Draw_Pic`).
    Picture {
        /// Left edge.
        x: i32,
        /// Top edge.
        y: i32,
        /// WAD lump name.
        lump: String,
    },
    /// Transparent picture (`Draw_TransPic`).
    TransPicture {
        /// Left edge.
        x: i32,
        /// Top edge.
        y: i32,
        /// WAD lump name.
        lump: String,
    },
    /// Picture centered in the 320-wide playfield (`M_DrawPic`).
    CenteredPicture {
        /// Playfield left edge (the bar x offset).
        x_base: i32,
        /// Top edge.
        y: i32,
        /// WAD lump name.
        lump: String,
    },
    /// Picture centered in the viewport (finale overlay).
    ViewportCenteredPicture {
        /// Top edge.
        y: i32,
        /// WAD lump name.
        lump: String,
    },
    /// One console character (`Draw_Character`, conchars cell).
    Char {
        /// Left edge.
        x: i32,
        /// Top edge.
        y: i32,
        /// Character code.
        code: i32,
    },
    /// Solid fill (`Draw_Fill`, palette index).
    Fill {
        /// Left edge.
        x: i32,
        /// Top edge.
        y: i32,
        /// Width.
        width: i32,
        /// Height.
        height: i32,
        /// Palette color.
        color: u8,
    },
}

/// Bar line count from the view size (`screen.c:254-259`): 120 hides
/// the bar, 110 drops the inventory, anything smaller draws all 48.
#[must_use]
pub const fn q1_sb_lines(viewsize: i32) -> i32 {
    if viewsize >= 120 {
        0
    } else if viewsize >= 110 {
        Q1_SBAR_HEIGHT
    } else {
        Q1_SBAR_FULL
    }
}

/// Stock `Sbar_itoa` (`sbar.c:314-342`): signed decimal, no padding.
fn sbar_itoa(num: i32) -> String {
    // The C negates `i32::MIN` into itself, then prints through
    // wrapping division; `unsigned_abs` matches that expansion.
    let mut text = String::new();
    if num < 0 {
        text.push('-');
    }
    text.push_str(&num.unsigned_abs().to_string());
    text
}

/// Stock `Sbar_DrawNum` field (`sbar.c:350-374`): right-aligned
/// `digits` field of 24 px numerals, truncating leading (most
/// significant) digits past the field width. `color` selects `num_`
/// (0) or `anum_` (1); `xofs`/`base_y` anchor the bar.
struct Q1NumField {
    /// Bar x offset.
    xofs: i32,
    /// Bar top edge.
    base_y: i32,
    /// Field left edge.
    x: i32,
    /// Field top edge.
    y: i32,
    /// Value.
    num: i32,
    /// Field width in digits.
    digits: usize,
    /// Numeral set.
    color: usize,
}

fn draw_num(out: &mut Vec<NativeQ1HudOperation>, field: Q1NumField) {
    let text = sbar_itoa(field.num);
    let chars: Vec<char> = text.chars().collect();
    let start = chars.len().saturating_sub(field.digits);
    let shown = &chars[start..];
    let mut x = field.x + (field.digits.saturating_sub(shown.len()) as i32) * 24;
    for ch in shown {
        let lump = if *ch == '-' {
            format!("{}_minus", if field.color == 0 { "num" } else { "anum" })
        } else {
            format!("{}_{}", if field.color == 0 { "num" } else { "anum" }, ch)
        };
        out.push(NativeQ1HudOperation::TransPicture {
            x: field.xofs + x,
            y: field.base_y + field.y,
            lump,
        });
        x += 24;
    }
}

/// Short `Sbar_DrawNum` call over an explicit field.
#[allow(clippy::too_many_arguments)]
fn draw_num_at(
    out: &mut Vec<NativeQ1HudOperation>,
    xofs: i32,
    base_y: i32,
    x: i32,
    y: i32,
    num: i32,
    digits: usize,
    color: usize,
) {
    draw_num(
        out,
        Q1NumField {
            xofs,
            base_y,
            x,
            y,
            num,
            digits,
            color,
        },
    )
}

/// Stock `Sbar_DrawString` body (`sbar.c:301-307`): 8 px characters.
fn draw_string(out: &mut Vec<NativeQ1HudOperation>, xofs: i32, base_y: i32, x: i32, y: i32, text: &str) {
    for (index, byte) in text.bytes().enumerate() {
        out.push(NativeQ1HudOperation::Char {
            x: xofs + x + index as i32 * 8,
            y: base_y + y,
            code: i32::from(byte),
        });
    }
}

/// Stock `Sbar_ColorForMap` (`sbar.c:416-419`): identity plus 8.
#[must_use]
pub const fn q1_color_for_map(m: u8) -> u8 {
    m.wrapping_add(8)
}

/// Stock `Sbar_SortFrags` (`sbar.c:391-414`): scoreboard slots with
/// names, bubble-sorted by descending frags (stable for ties).
#[must_use]
pub fn q1_sort_frags(frame: &NativeQ1HudFrame) -> Vec<usize> {
    let mut order: Vec<usize> = (0..frame.maxclients.max(0) as usize)
        .filter(|slot| frame.scores.get(*slot).is_some_and(|entry| !entry.name.is_empty()))
        .collect();
    for i in 0..order.len() {
        for j in 0..order.len().saturating_sub(1 + i) {
            if frame.scores[order[j]].frags < frame.scores[order[j + 1]].frags {
                order.swap(j, j + 1);
            }
        }
    }
    order
}

/// Face lump for the bar (`Sbar_DrawFace`, `sbar.c:828-919`): powerup
/// faces first, then health frames (`face1` at 100+, `health / 20`
/// below), with the pain frame while `time <= face_anim_until`.
#[must_use]
pub fn q1_face_lump(frame: &NativeQ1HudFrame) -> &'static str {
    if frame.items & (Q1_IT_INVISIBILITY | Q1_IT_INVULNERABILITY) == (Q1_IT_INVISIBILITY | Q1_IT_INVULNERABILITY) {
        return "face_inv2";
    }
    if frame.items & Q1_IT_QUAD != 0 {
        return "face_quad";
    }
    if frame.items & Q1_IT_INVISIBILITY != 0 {
        return "face_invis";
    }
    if frame.items & Q1_IT_INVULNERABILITY != 0 {
        return "face_invul2";
    }
    let health = frame.stats[Q1_STAT_HEALTH];
    let face = if health >= 100 { 4 } else { (health / 20).clamp(0, 4) };
    let pain = frame.time <= frame.face_anim_until;
    match (face, pain) {
        (4, false) => "face1",
        (4, true) => "face_p1",
        (3, false) => "face2",
        (3, true) => "face_p2",
        (2, false) => "face3",
        (2, true) => "face_p3",
        (1, false) => "face4",
        (1, true) => "face_p4",
        (_, false) => "face5",
        (_, true) => "face_p5",
    }
}

/// Weapon-flash row (`sbar.c:570-585`): fresh pickups cycle rows
/// 2-6, settled weapons show row 1 when active, else row 0.
fn weapon_flash_row(frame: &NativeQ1HudFrame, bit: u32, active: bool) -> usize {
    let gettime = frame.item_gettime.get(bit as usize).copied().unwrap_or(0.0);
    let flashon = ((frame.time - gettime) * 10.0) as i32;
    if flashon >= 10 {
        usize::from(active)
    } else {
        (flashon % 5 + 2) as usize
    }
}

/// Weapon inventory lump for a flash row and stem.
fn weapon_lump(row: usize, stem: &str) -> String {
    match row {
        0 => format!("inv_{stem}"),
        1 => format!("inv2_{stem}"),
        _ => format!("inva{}_{stem}", row - 1),
    }
}

/// Inventory strip (`Sbar_DrawInventory`, `sbar.c:546-757`).
fn draw_inventory(out: &mut Vec<NativeQ1HudOperation>, frame: &NativeQ1HudFrame, xofs: i32, base_y: i32) {
    let rogue = frame.product == Q1SbarProduct::Rogue;
    let hipnotic = frame.product == Q1SbarProduct::Hipnotic;
    if rogue {
        let lump = if frame.stats[Q1_STAT_ACTIVEWEAPON] >= Q1_RIT_LAVA_NAILGUN as i32 {
            "r_invbar1"
        } else {
            "r_invbar2"
        };
        out.push(NativeQ1HudOperation::Picture {
            x: xofs,
            y: base_y - 24,
            lump: lump.to_string(),
        });
    } else {
        out.push(NativeQ1HudOperation::Picture {
            x: xofs,
            y: base_y - 24,
            lump: "ibar".to_string(),
        });
    }

    for (slot, stem) in Q1_WEAPON_STEMS.iter().enumerate() {
        if frame.items & (Q1_IT_SHOTGUN << slot) == 0 {
            continue;
        }
        let active = frame.stats[Q1_STAT_ACTIVEWEAPON] == (Q1_IT_SHOTGUN << slot) as i32;
        let row = weapon_flash_row(frame, slot as u32, active);
        out.push(NativeQ1HudOperation::Picture {
            x: xofs + slot as i32 * 24,
            y: base_y - 16,
            lump: weapon_lump(row, stem),
        });
    }

    if hipnotic {
        draw_hipnotic_weapons(out, frame, xofs, base_y);
    }
    if rogue {
        draw_rogue_weapons(out, frame, xofs, base_y);
    }

    for (slot, stat) in [Q1_STAT_SHELLS, Q1_STAT_NAILS, Q1_STAT_ROCKETS, Q1_STAT_CELLS]
        .iter()
        .enumerate()
    {
        let text = format!("{:3}", frame.stats[*stat]);
        let bytes = text.as_bytes();
        for (digit, byte) in bytes.iter().enumerate() {
            if *byte == b' ' {
                continue;
            }
            out.push(NativeQ1HudOperation::Char {
                x: xofs + (6 * slot as i32 + digit as i32 + 1) * 8 - 2,
                y: base_y - 24,
                code: 18 + i32::from(*byte - b'0'),
            });
        }
    }

    // Stock keeps a `flashon` that is always 0 here, so carried items
    // always draw (`sbar.c:673-693`); hipnotic moves the keys to the
    // main bar (`sbar.c:686`).
    for (slot, lump) in Q1_ITEM_LUMPS.iter().enumerate() {
        if frame.items & (1 << (17 + slot)) == 0 {
            continue;
        }
        if hipnotic && slot <= 1 {
            continue;
        }
        out.push(NativeQ1HudOperation::Picture {
            x: xofs + 192 + slot as i32 * 16,
            y: base_y - 16,
            lump: (*lump).to_string(),
        });
    }
    if hipnotic {
        for (slot, lump) in Q1_HIPNOTIC_ITEM_LUMPS.iter().enumerate() {
            if frame.items & (1 << (24 + slot)) == 0 {
                continue;
            }
            out.push(NativeQ1HudOperation::Picture {
                x: xofs + 288 + slot as i32 * 16,
                y: base_y - 16,
                lump: (*lump).to_string(),
            });
        }
    }
    if rogue {
        for (slot, lump) in Q1_ROGUE_ITEM_LUMPS.iter().enumerate() {
            if frame.items & (1 << (29 + slot)) == 0 {
                continue;
            }
            out.push(NativeQ1HudOperation::Picture {
                x: xofs + 288 + slot as i32 * 16,
                y: base_y - 16,
                lump: (*lump).to_string(),
            });
        }
    } else {
        for (slot, lump) in Q1_SIGIL_LUMPS.iter().enumerate() {
            if frame.items & (1 << (28 + slot)) == 0 {
                continue;
            }
            out.push(NativeQ1HudOperation::Picture {
                x: xofs + 320 - 32 + slot as i32 * 8,
                y: base_y - 16,
                lump: (*lump).to_string(),
            });
        }
    }
}

/// Hipnotic weapons (`sbar.c:591-644`): laser and mjolnir ride the
/// right end, grenade/proximity share slot 96 with the stock
/// grenade-launcher presence deciding which lump shows.
fn draw_hipnotic_weapons(out: &mut Vec<NativeQ1HudOperation>, frame: &NativeQ1HudFrame, xofs: i32, base_y: i32) {
    let mut grenade_flashing = false;
    for (slot, bit) in Q1_HIPWEAPONS.iter().enumerate() {
        if frame.items & (1 << bit) == 0 {
            continue;
        }
        let active = frame.stats[Q1_STAT_ACTIVEWEAPON] == 1 << bit;
        let row = weapon_flash_row(frame, *bit, active);
        if slot == 2 {
            if frame.items & Q1_IT_GRENADE_LAUNCHER != 0 && row != 0 {
                grenade_flashing = true;
                out.push(NativeQ1HudOperation::Picture {
                    x: xofs + 96,
                    y: base_y - 16,
                    lump: weapon_lump(row, Q1_HIPNOTIC_STEMS[2]),
                });
            }
        } else if slot == 3 {
            if frame.items & (Q1_IT_SHOTGUN << 4) != 0 {
                if row != 0 && !grenade_flashing {
                    out.push(NativeQ1HudOperation::Picture {
                        x: xofs + 96,
                        y: base_y - 16,
                        lump: weapon_lump(row, Q1_HIPNOTIC_STEMS[3]),
                    });
                } else if !grenade_flashing {
                    out.push(NativeQ1HudOperation::Picture {
                        x: xofs + 96,
                        y: base_y - 16,
                        lump: weapon_lump(0, Q1_HIPNOTIC_STEMS[3]),
                    });
                }
            } else {
                out.push(NativeQ1HudOperation::Picture {
                    x: xofs + 96,
                    y: base_y - 16,
                    lump: weapon_lump(row, Q1_HIPNOTIC_STEMS[4]),
                });
            }
        } else {
            out.push(NativeQ1HudOperation::Picture {
                x: xofs + 176 + slot as i32 * 24,
                y: base_y - 16,
                lump: weapon_lump(row, Q1_HIPNOTIC_STEMS[slot]),
            });
        }
    }
}

/// Rogue powered weapons (`sbar.c:646-659`).
fn draw_rogue_weapons(out: &mut Vec<NativeQ1HudOperation>, frame: &NativeQ1HudFrame, xofs: i32, base_y: i32) {
    if frame.stats[Q1_STAT_ACTIVEWEAPON] < Q1_RIT_LAVA_NAILGUN as i32 {
        return;
    }
    for (slot, lump) in Q1_ROGUE_WEAPONS.iter().enumerate() {
        if frame.stats[Q1_STAT_ACTIVEWEAPON] == (Q1_RIT_LAVA_NAILGUN << slot) as i32 {
            out.push(NativeQ1HudOperation::Picture {
                x: xofs + (slot as i32 + 2) * 24,
                y: base_y - 16,
                lump: (*lump).to_string(),
            });
        }
    }
}

/// Frag strip (`Sbar_DrawFrags`, `sbar.c:766-818`): the top four
/// scores above the inventory, with brackets around the viewer.
fn draw_frags(out: &mut Vec<NativeQ1HudOperation>, frame: &NativeQ1HudFrame, xofs: i32, base_y: i32, height: i32) {
    let order = q1_sort_frags(frame);
    let y = height - Q1_SBAR_HEIGHT - 23;
    for (row, slot) in order.iter().take(4).enumerate() {
        let entry = &frame.scores[*slot];
        let x = 23 + row as i32 * 4;
        let top = q1_color_for_map(entry.colors & 0xf0);
        let bottom = q1_color_for_map((entry.colors & 15) << 4);
        out.push(NativeQ1HudOperation::Fill {
            x: xofs + x * 8 + 10,
            y,
            width: 28,
            height: 4,
            color: top,
        });
        out.push(NativeQ1HudOperation::Fill {
            x: xofs + x * 8 + 10,
            y: y + 4,
            width: 28,
            height: 3,
            color: bottom,
        });
        let text = format!("{:3}", entry.frags);
        for (digit, byte) in text.bytes().enumerate() {
            out.push(NativeQ1HudOperation::Char {
                x: xofs + (x + 1 + digit as i32) * 8,
                y: base_y - 24,
                code: i32::from(byte),
            });
        }
        if *slot as i32 == frame.viewentity - 1 {
            out.push(NativeQ1HudOperation::Char {
                x: xofs + x * 8 + 2,
                y: base_y - 24,
                code: 16,
            });
            out.push(NativeQ1HudOperation::Char {
                x: xofs + (x + 4) * 8 - 4,
                y: base_y - 24,
                code: 17,
            });
        }
    }
}

/// Face row (`Sbar_DrawFace`, `sbar.c:828-919`), including the rogue
/// team-color branch (`teamplay` 4-6 in multiplayer).
fn draw_face(out: &mut Vec<NativeQ1HudOperation>, frame: &NativeQ1HudFrame, xofs: i32, base_y: i32) {
    if frame.product == Q1SbarProduct::Rogue && frame.maxclients != 1 && frame.teamplay > 3.0 && frame.teamplay < 7.0 {
        let slot = (frame.viewentity - 1).max(0) as usize;
        if let Some(entry) = frame.scores.get(slot) {
            let top = q1_color_for_map(entry.colors & 0xf0);
            let bottom = q1_color_for_map((entry.colors & 15) << 4);
            out.push(NativeQ1HudOperation::Picture {
                x: xofs + 112,
                y: base_y,
                lump: "r_teambord".to_string(),
            });
            out.push(NativeQ1HudOperation::Fill {
                x: xofs + 113,
                y: base_y + 3,
                width: 22,
                height: 9,
                color: top,
            });
            out.push(NativeQ1HudOperation::Fill {
                x: xofs + 113,
                y: base_y + 12,
                width: 22,
                height: 9,
                color: bottom,
            });
            let text = format!("{:3}", entry.frags);
            for (digit, byte) in text.bytes().enumerate() {
                let code = if top == 8 && byte != b' ' {
                    18 + i32::from(byte - b'0')
                } else {
                    i32::from(byte)
                };
                out.push(NativeQ1HudOperation::Char {
                    x: xofs + 109 + digit as i32 * 7,
                    y: base_y + 3,
                    code,
                });
            }
        }
        return;
    }
    out.push(NativeQ1HudOperation::Picture {
        x: xofs + 112,
        y: base_y,
        lump: q1_face_lump(frame).to_string(),
    });
}

/// Armor row (`Sbar_Draw`, `sbar.c:967-997`): invulnerability shows
/// 666 plus the disc, otherwise the value plus the strongest icon.
fn draw_armor(out: &mut Vec<NativeQ1HudOperation>, frame: &NativeQ1HudFrame, xofs: i32, base_y: i32) {
    if frame.items & Q1_IT_INVULNERABILITY != 0 {
        draw_num_at(out, xofs, base_y, 24, 0, 666, 3, 1);
        out.push(NativeQ1HudOperation::Picture {
            x: xofs,
            y: base_y,
            lump: "disc".to_string(),
        });
        return;
    }
    let armor = frame.stats[Q1_STAT_ARMOR];
    draw_num_at(out, xofs, base_y, 24, 0, armor, 3, usize::from(armor <= 25));
    let (low, mid, high) = if frame.product == Q1SbarProduct::Rogue {
        (Q1_RIT_ARMOR1, Q1_RIT_ARMOR2, Q1_RIT_ARMOR3)
    } else {
        (Q1_IT_ARMOR1, Q1_IT_ARMOR2, Q1_IT_ARMOR3)
    };
    let lump = if frame.items & high != 0 {
        Some(Q1_ARMOR_LUMPS[2])
    } else if frame.items & mid != 0 {
        Some(Q1_ARMOR_LUMPS[1])
    } else if frame.items & low != 0 {
        Some(Q1_ARMOR_LUMPS[0])
    } else {
        None
    };
    if let Some(lump) = lump {
        out.push(NativeQ1HudOperation::Picture {
            x: xofs,
            y: base_y,
            lump: lump.to_string(),
        });
    }
}

/// Ammo row (`Sbar_Draw`, `sbar.c:1006-1037`): the current-ammo icon
/// plus its value.
fn draw_ammo(out: &mut Vec<NativeQ1HudOperation>, frame: &NativeQ1HudFrame, xofs: i32, base_y: i32) {
    let lump = if frame.product == Q1SbarProduct::Rogue {
        if frame.items & Q1_RIT_SHELLS != 0 {
            Some(Q1_AMMO_LUMPS[0])
        } else if frame.items & Q1_RIT_NAILS != 0 {
            Some(Q1_AMMO_LUMPS[1])
        } else if frame.items & Q1_RIT_ROCKETS != 0 {
            Some(Q1_AMMO_LUMPS[2])
        } else if frame.items & Q1_RIT_CELLS != 0 {
            Some(Q1_AMMO_LUMPS[3])
        } else if frame.items & Q1_RIT_LAVA_NAILS != 0 {
            Some(Q1_ROGUE_AMMO_LUMPS[0])
        } else if frame.items & Q1_RIT_PLASMA_AMMO != 0 {
            Some(Q1_ROGUE_AMMO_LUMPS[1])
        } else if frame.items & Q1_RIT_MULTI_ROCKETS != 0 {
            Some(Q1_ROGUE_AMMO_LUMPS[2])
        } else {
            None
        }
    } else if frame.items & Q1_IT_SHELLS != 0 {
        Some(Q1_AMMO_LUMPS[0])
    } else if frame.items & Q1_IT_NAILS != 0 {
        Some(Q1_AMMO_LUMPS[1])
    } else if frame.items & Q1_IT_ROCKETS != 0 {
        Some(Q1_AMMO_LUMPS[2])
    } else if frame.items & Q1_IT_CELLS != 0 {
        Some(Q1_AMMO_LUMPS[3])
    } else {
        None
    };
    if let Some(lump) = lump {
        out.push(NativeQ1HudOperation::Picture {
            x: xofs + 224,
            y: base_y,
            lump: lump.to_string(),
        });
    }
    let ammo = frame.stats[Q1_STAT_AMMO];
    draw_num_at(out, xofs, base_y, 248, 0, ammo, 3, usize::from(ammo <= 10));
}

/// Solo scoreboard (`Sbar_SoloScoreboard`, `sbar.c:457-480`): monster
/// and secret tallies, level time, and the level name.
fn draw_solo_scoreboard(out: &mut Vec<NativeQ1HudOperation>, frame: &NativeQ1HudFrame, xofs: i32, base_y: i32) {
    draw_string(
        out,
        xofs,
        base_y,
        8,
        4,
        &format!(
            "Monsters:{:3} /{:3}",
            frame.stats[Q1_STAT_MONSTERS], frame.stats[Q1_STAT_TOTALMONSTERS]
        ),
    );
    draw_string(
        out,
        xofs,
        base_y,
        8,
        12,
        &format!(
            "Secrets :{:3} /{:3}",
            frame.stats[Q1_STAT_SECRETS], frame.stats[Q1_STAT_TOTALSECRETS]
        ),
    );
    let minutes = (frame.time / 60.0) as i32;
    let seconds = (frame.time - 60.0 * minutes as f32) as i32;
    draw_string(
        out,
        xofs,
        base_y,
        184,
        4,
        &format!("Time :{:3}:{}{}", minutes, seconds / 10, seconds - 10 * (seconds / 10)),
    );
    let width = frame.levelname.len() as i32;
    draw_string(out, xofs, base_y, 232 - width * 4, 12, &frame.levelname);
}

/// Deathmatch scoreboard overlay (`Sbar_DeathmatchOverlay`,
/// `sbar.c:1086-1159`): ranking header plus every score with its
/// team-color swatch, centered in the 320-wide playfield.
fn draw_deathmatch_overlay(out: &mut Vec<NativeQ1HudOperation>, frame: &NativeQ1HudFrame, xofs: i32) {
    out.push(NativeQ1HudOperation::CenteredPicture {
        x_base: xofs,
        y: 8,
        lump: "gfx/ranking.lmp".to_string(),
    });
    let order = q1_sort_frags(frame);
    let mut y = 40;
    for slot in order {
        let entry = &frame.scores[slot];
        let x = xofs + 80;
        let top = q1_color_for_map(entry.colors & 0xf0);
        let bottom = q1_color_for_map((entry.colors & 15) << 4);
        out.push(NativeQ1HudOperation::Fill {
            x,
            y,
            width: 40,
            height: 4,
            color: top,
        });
        out.push(NativeQ1HudOperation::Fill {
            x,
            y: y + 4,
            width: 40,
            height: 4,
            color: bottom,
        });
        let text = format!("{:3}", entry.frags);
        for (digit, byte) in text.bytes().enumerate() {
            out.push(NativeQ1HudOperation::Char {
                x: x + 8 + digit as i32 * 8,
                y,
                code: i32::from(byte),
            });
        }
        if slot as i32 == frame.viewentity - 1 {
            out.push(NativeQ1HudOperation::Char { x: x - 8, y, code: 12 });
        }
        for (index, byte) in entry.name.bytes().enumerate() {
            out.push(NativeQ1HudOperation::Char {
                x: x + 64 + index as i32 * 8,
                y,
                code: i32::from(byte),
            });
        }
        y += 10;
    }
}

/// Mini deathmatch overlay (`Sbar_MiniDeathmatchOverlay`,
/// `sbar.c:1167-1261`): the score window beside the bar on wide
/// screens, scrolled to keep the viewer centered.
fn draw_mini_deathmatch_overlay(
    out: &mut Vec<NativeQ1HudOperation>,
    frame: &NativeQ1HudFrame,
    width: i32,
    height: i32,
    sb_lines: i32,
) {
    if width < 512 || sb_lines == 0 {
        return;
    }
    let order = q1_sort_frags(frame);
    let numlines = sb_lines / 8;
    if numlines < 3 {
        return;
    }
    let viewer = order
        .iter()
        .position(|slot| *slot as i32 == frame.viewentity - 1)
        .unwrap_or(0);
    let mut start = if order.iter().any(|slot| *slot as i32 == frame.viewentity - 1) {
        viewer as i32 - numlines / 2
    } else {
        0
    };
    start = start.min(order.len() as i32 - numlines).max(0);
    let mut y = height - sb_lines;
    for slot in order.iter().skip(start as usize) {
        if y >= height - 8 {
            break;
        }
        let entry = &frame.scores[*slot];
        let x = 324;
        let top = q1_color_for_map(entry.colors & 0xf0);
        let bottom = q1_color_for_map((entry.colors & 15) << 4);
        out.push(NativeQ1HudOperation::Fill {
            x,
            y: y + 1,
            width: 40,
            height: 3,
            color: top,
        });
        out.push(NativeQ1HudOperation::Fill {
            x,
            y: y + 4,
            width: 40,
            height: 4,
            color: bottom,
        });
        let text = format!("{:3}", entry.frags);
        for (digit, byte) in text.bytes().enumerate() {
            out.push(NativeQ1HudOperation::Char {
                x: x + 8 + digit as i32 * 8,
                y,
                code: i32::from(byte),
            });
        }
        if *slot as i32 == frame.viewentity - 1 {
            out.push(NativeQ1HudOperation::Char { x, y, code: 16 });
            out.push(NativeQ1HudOperation::Char { x: x + 32, y, code: 17 });
        }
        for (index, byte) in entry.name.bytes().enumerate() {
            out.push(NativeQ1HudOperation::Char {
                x: x + 48 + index as i32 * 8,
                y,
                code: i32::from(byte),
            });
        }
        y += 8;
    }
}

/// Stock `Sbar_Draw` (`sbar.c:926-1044`) without the console-full and
/// page-skip gates (the caller owns those): inventory plus frags
/// above 24 lines, then the scoreboard or the sbar with face and
/// vitals, plus the mini overlay on wide deathmatch screens.
pub fn q1_sbar_operations(
    frame: &NativeQ1HudFrame,
    width: i32,
    height: i32,
    sb_lines: i32,
) -> Vec<NativeQ1HudOperation> {
    let mut out = Vec::new();
    let xofs = if frame.deathmatch {
        0
    } else {
        (width - Q1_SBAR_WIDTH) >> 1
    };
    let base_y = height - Q1_SBAR_HEIGHT;
    if sb_lines > 24 {
        draw_inventory(&mut out, frame, xofs, base_y);
        if frame.maxclients != 1 {
            draw_frags(&mut out, frame, xofs, base_y, height);
        }
    }
    if frame.showscores || frame.stats[Q1_STAT_HEALTH] <= 0 {
        out.push(NativeQ1HudOperation::Picture {
            x: xofs,
            y: base_y,
            lump: "scorebar".to_string(),
        });
        draw_solo_scoreboard(&mut out, frame, xofs, base_y);
        if frame.deathmatch {
            draw_deathmatch_overlay(&mut out, frame, xofs);
        }
    } else if sb_lines != 0 {
        out.push(NativeQ1HudOperation::Picture {
            x: xofs,
            y: base_y,
            lump: "sbar".to_string(),
        });
        if frame.product == Q1SbarProduct::Hipnotic {
            if frame.items & Q1_IT_KEY1 != 0 {
                out.push(NativeQ1HudOperation::Picture {
                    x: xofs + 209,
                    y: base_y + 3,
                    lump: Q1_ITEM_LUMPS[0].to_string(),
                });
            }
            if frame.items & Q1_IT_KEY2 != 0 {
                out.push(NativeQ1HudOperation::Picture {
                    x: xofs + 209,
                    y: base_y + 12,
                    lump: Q1_ITEM_LUMPS[1].to_string(),
                });
            }
        }
        draw_armor(&mut out, frame, xofs, base_y);
        draw_face(&mut out, frame, xofs, base_y);
        let health = frame.stats[Q1_STAT_HEALTH];
        draw_num_at(&mut out, xofs, base_y, 136, 0, health, 3, usize::from(health <= 25));
        draw_ammo(&mut out, frame, xofs, base_y);
    }
    if width > Q1_SBAR_WIDTH && frame.deathmatch {
        draw_mini_deathmatch_overlay(&mut out, frame, width, height, sb_lines);
    }
    out
}

/// Intermission tallies (`Sbar_IntermissionOverlay`, `sbar.c:1269-1306`):
/// deathmatch reuses the ranking overlay, otherwise the complete/inter
/// plates with time, secrets, and monster counts.
pub fn q1_intermission_operations(frame: &NativeQ1HudFrame, width: i32) -> Vec<NativeQ1HudOperation> {
    let mut out = Vec::new();
    let xofs = if frame.deathmatch {
        0
    } else {
        (width - Q1_SBAR_WIDTH) >> 1
    };
    if frame.deathmatch {
        draw_deathmatch_overlay(&mut out, frame, xofs);
        return out;
    }
    out.push(NativeQ1HudOperation::Picture {
        x: xofs + 64,
        y: 24,
        lump: "gfx/complete.lmp".to_string(),
    });
    out.push(NativeQ1HudOperation::TransPicture {
        x: xofs,
        y: 56,
        lump: "gfx/inter.lmp".to_string(),
    });
    let minutes = (frame.completed_time / 60.0) as i32;
    let seconds = (frame.completed_time - 60.0 * minutes as f32) as i32;
    draw_num_at(&mut out, xofs, 0, 160, 64, minutes, 3, 0);
    out.push(NativeQ1HudOperation::TransPicture {
        x: xofs + 234,
        y: 64,
        lump: "num_colon".to_string(),
    });
    out.push(NativeQ1HudOperation::TransPicture {
        x: xofs + 246,
        y: 64,
        lump: format!("num_{}", seconds / 10),
    });
    out.push(NativeQ1HudOperation::TransPicture {
        x: xofs + 266,
        y: 64,
        lump: format!("num_{}", seconds % 10),
    });
    draw_num_at(&mut out, xofs, 0, 160, 104, frame.stats[Q1_STAT_SECRETS], 3, 0);
    out.push(NativeQ1HudOperation::TransPicture {
        x: xofs + 232,
        y: 104,
        lump: "num_slash".to_string(),
    });
    draw_num_at(&mut out, xofs, 0, 240, 104, frame.stats[Q1_STAT_TOTALSECRETS], 3, 0);
    draw_num_at(&mut out, xofs, 0, 160, 144, frame.stats[Q1_STAT_MONSTERS], 3, 0);
    out.push(NativeQ1HudOperation::TransPicture {
        x: xofs + 232,
        y: 144,
        lump: "num_slash".to_string(),
    });
    draw_num_at(&mut out, xofs, 0, 240, 144, frame.stats[Q1_STAT_TOTALMONSTERS], 3, 0);
    out
}

/// Finale plate (`Sbar_FinaleOverlay`, `sbar.c:1315-1323`).
#[must_use]
pub fn q1_finale_operations() -> Vec<NativeQ1HudOperation> {
    vec![NativeQ1HudOperation::ViewportCenteredPicture {
        y: 16,
        lump: "gfx/finale.lmp".to_string(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> NativeQ1HudFrame {
        let mut frame = NativeQ1HudFrame::default();
        frame.stats[Q1_STAT_HEALTH] = 100;
        // A settled client: stock shows the pain frame at t=0
        // (`0 <= faceanimtime`), so expire it explicitly.
        frame.face_anim_until = -1.0;
        frame
    }

    fn pictures(ops: &[NativeQ1HudOperation]) -> Vec<(i32, i32, &str)> {
        ops.iter()
            .filter_map(|op| match op {
                NativeQ1HudOperation::Picture { x, y, lump } | NativeQ1HudOperation::TransPicture { x, y, lump } => {
                    Some((*x, *y, lump.as_str()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn sb_lines_follow_viewsize() {
        assert_eq!(q1_sb_lines(120), 0);
        assert_eq!(q1_sb_lines(200), 0);
        assert_eq!(q1_sb_lines(119), 24);
        assert_eq!(q1_sb_lines(110), 24);
        assert_eq!(q1_sb_lines(109), 48);
        assert_eq!(q1_sb_lines(100), 48);
    }

    #[test]
    fn face_follows_health_and_pain() {
        let mut frame = frame();
        assert_eq!(q1_face_lump(&frame), "face1");
        frame.stats[Q1_STAT_HEALTH] = 99;
        assert_eq!(q1_face_lump(&frame), "face1");
        frame.stats[Q1_STAT_HEALTH] = 79;
        assert_eq!(q1_face_lump(&frame), "face2");
        frame.stats[Q1_STAT_HEALTH] = 40;
        assert_eq!(q1_face_lump(&frame), "face3");
        frame.stats[Q1_STAT_HEALTH] = 39;
        assert_eq!(q1_face_lump(&frame), "face4");
        frame.stats[Q1_STAT_HEALTH] = 0;
        assert_eq!(q1_face_lump(&frame), "face5");
        frame.stats[Q1_STAT_HEALTH] = -50;
        assert_eq!(q1_face_lump(&frame), "face5");
        frame.stats[Q1_STAT_HEALTH] = 100;
        frame.time = 10.0;
        frame.face_anim_until = 10.2;
        assert_eq!(q1_face_lump(&frame), "face_p1");
        frame.time = 10.3;
        assert_eq!(q1_face_lump(&frame), "face1");
    }

    #[test]
    fn face_powerups_take_precedence() {
        let mut frame = frame();
        frame.items = Q1_IT_QUAD;
        assert_eq!(q1_face_lump(&frame), "face_quad");
        frame.items = Q1_IT_INVISIBILITY;
        assert_eq!(q1_face_lump(&frame), "face_invis");
        frame.items = Q1_IT_INVULNERABILITY;
        assert_eq!(q1_face_lump(&frame), "face_invul2");
        frame.items = Q1_IT_INVISIBILITY | Q1_IT_INVULNERABILITY;
        assert_eq!(q1_face_lump(&frame), "face_inv2");
        frame.items = Q1_IT_QUAD | Q1_IT_INVISIBILITY | Q1_IT_INVULNERABILITY;
        assert_eq!(q1_face_lump(&frame), "face_inv2");
    }

    #[test]
    fn main_bar_places_sbar_face_and_vitals() {
        let mut frame = frame();
        frame.stats[Q1_STAT_ARMOR] = 50;
        frame.stats[Q1_STAT_AMMO] = 25;
        frame.items = Q1_IT_ARMOR1 | Q1_IT_SHELLS;
        let ops = q1_sbar_operations(&frame, 320, 200, 48);
        let pics = pictures(&ops);
        assert!(pics.contains(&(0, 176, "sbar")), "{pics:?}");
        assert!(pics.contains(&(0, 176, "sb_armor1")), "{pics:?}");
        assert!(pics.contains(&(112, 176, "face1")), "{pics:?}");
        assert!(pics.contains(&(224, 176, "sb_shells")), "{pics:?}");
        // Health 100 right-aligned at x=136: num_1/num_0/num_0.
        assert!(pics.contains(&(136, 176, "num_1")), "{pics:?}");
        assert!(pics.contains(&(160, 176, "num_0")), "{pics:?}");
        assert!(pics.contains(&(184, 176, "num_0")), "{pics:?}");
        // Armor 50 and ammo 25 use the normal set above their floors.
        assert!(pics.contains(&(48, 176, "num_5")), "{pics:?}");
        assert!(pics.contains(&(72, 176, "num_0")), "{pics:?}");
        assert!(pics.contains(&(272, 176, "num_2")), "{pics:?}");
        assert!(pics.contains(&(296, 176, "num_5")), "{pics:?}");
    }

    #[test]
    fn low_vitals_use_the_red_set() {
        let mut frame = frame();
        frame.stats[Q1_STAT_HEALTH] = 25;
        frame.stats[Q1_STAT_ARMOR] = 25;
        frame.stats[Q1_STAT_AMMO] = 10;
        let ops = q1_sbar_operations(&frame, 320, 200, 48);
        let pics = pictures(&ops);
        assert!(pics.contains(&(184, 176, "anum_5")), "{pics:?}");
        assert!(pics.contains(&(72, 176, "anum_5")), "{pics:?}");
        assert!(pics.contains(&(296, 176, "anum_0")), "{pics:?}");
    }

    #[test]
    fn invulnerability_shows_666_and_disc() {
        let mut frame = frame();
        frame.items = Q1_IT_INVULNERABILITY;
        let ops = q1_sbar_operations(&frame, 320, 200, 48);
        let pics = pictures(&ops);
        assert!(pics.contains(&(0, 176, "disc")), "{pics:?}");
        assert!(pics.contains(&(24, 176, "anum_6")), "{pics:?}");
        assert!(pics.contains(&(48, 176, "anum_6")), "{pics:?}");
        assert!(pics.contains(&(72, 176, "anum_6")), "{pics:?}");
    }

    #[test]
    fn numbers_truncate_and_pad_like_sbar_drawnum() {
        let mut frame = frame();
        frame.stats[Q1_STAT_HEALTH] = 1234;
        let ops = q1_sbar_operations(&frame, 320, 200, 48);
        let pics = pictures(&ops);
        // Four digits truncate the leading one: 234 at x=136.
        assert!(pics.contains(&(136, 176, "num_2")), "{pics:?}");
        assert!(pics.contains(&(160, 176, "num_3")), "{pics:?}");
        assert!(pics.contains(&(184, 176, "num_4")), "{pics:?}");
        // Negative health shows the scoreboard instead of the bar,
        // so exercise the minus frame through armor.
        frame.stats[Q1_STAT_HEALTH] = 100;
        frame.stats[Q1_STAT_ARMOR] = -5;
        let ops = q1_sbar_operations(&frame, 320, 200, 48);
        let pics = pictures(&ops);
        // "-5" right-aligns in red: minus at 48, digit at 72.
        assert!(pics.contains(&(48, 176, "anum_minus")), "{pics:?}");
        assert!(pics.contains(&(72, 176, "anum_5")), "{pics:?}");
    }

    #[test]
    fn weapons_flash_then_settle() {
        let mut frame = frame();
        frame.items = Q1_IT_SHOTGUN | Q1_IT_NAILGUN;
        frame.stats[Q1_STAT_ACTIVEWEAPON] = Q1_IT_SHOTGUN as i32;
        frame.time = 100.0;
        // Fresh shotgun pickup cycles the inva rows; settled nailgun is owned-but-inactive.
        frame.item_gettime[0] = 99.75;
        frame.item_gettime[2] = 0.0;
        let ops = q1_sbar_operations(&frame, 320, 200, 48);
        let pics = pictures(&ops);
        assert!(pics.contains(&(0, 152, "ibar")), "{pics:?}");
        // (100 - 99.75) * 10 = 2 -> row (2 % 5) + 2 = 4 -> inva3_shotgun.
        assert!(pics.contains(&(0, 160, "inva3_shotgun")), "{pics:?}");
        assert!(pics.contains(&(48, 160, "inv_nailgun")), "{pics:?}");
        frame.item_gettime[0] = 0.0;
        let ops = q1_sbar_operations(&frame, 320, 200, 48);
        let pics = pictures(&ops);
        assert!(pics.contains(&(0, 160, "inv2_shotgun")), "{pics:?}");
    }

    #[test]
    fn inventory_counts_keys_and_sigils() {
        let mut frame = frame();
        frame.stats[Q1_STAT_SHELLS] = 25;
        frame.items = Q1_IT_KEY1 | Q1_IT_QUAD | Q1_IT_SIGIL2;
        let ops = q1_sbar_operations(&frame, 320, 200, 48);
        let pics = pictures(&ops);
        assert!(pics.contains(&(192, 160, "sb_key1")), "{pics:?}");
        assert!(pics.contains(&(272, 160, "sb_quad")), "{pics:?}");
        assert!(pics.contains(&(296, 160, "sb_sigil2")), "{pics:?}");
        let chars: Vec<(i32, i32, i32)> = ops
            .iter()
            .filter_map(|op| match op {
                NativeQ1HudOperation::Char { x, y, code } => Some((*x, *y, *code)),
                _ => None,
            })
            .collect();
        // " 25": leading space skipped, digits at x=14/22, codes 18+N.
        assert!(chars.contains(&(14, 152, 20)), "{chars:?}");
        assert!(chars.contains(&(22, 152, 23)), "{chars:?}");
        assert!(!chars.iter().any(|(x, y, _)| (*x, *y) == (6, 152)), "{chars:?}");
    }

    #[test]
    fn frags_sort_stable_and_bracket_the_viewer() {
        let mut frame = frame();
        frame.deathmatch = true;
        frame.maxclients = 4;
        frame.viewentity = 2;
        frame.scores = vec![
            Q1ScoreEntry {
                name: "a".to_string(),
                frags: 5,
                colors: 0x10,
            },
            Q1ScoreEntry {
                name: "b".to_string(),
                frags: 9,
                colors: 0x21,
            },
            Q1ScoreEntry {
                name: String::new(),
                frags: 99,
                colors: 0,
            },
            Q1ScoreEntry {
                name: "d".to_string(),
                frags: 5,
                colors: 0x32,
            },
        ];
        assert_eq!(q1_sort_frags(&frame), vec![1, 0, 3]);
        let ops = q1_sbar_operations(&frame, 320, 200, 48);
        let chars: Vec<(i32, i32, i32)> = ops
            .iter()
            .filter_map(|op| match op {
                NativeQ1HudOperation::Char { x, y, code } => Some((*x, *y, *code)),
                _ => None,
            })
            .collect();
        // Winner "  9" at row 0 (x=23 cells), brackets around the viewer (row 0, slot 1).
        assert!(chars.contains(&(192, 152, b' ' as i32)), "{chars:?}");
        assert!(chars.contains(&(208, 152, b'9' as i32)), "{chars:?}");
        assert!(chars.contains(&(186, 152, 16)), "{chars:?}");
        assert!(chars.contains(&(212, 152, 17)), "{chars:?}");
    }

    #[test]
    fn solo_scoreboard_shows_tallies_time_and_level() {
        let mut frame = frame();
        frame.showscores = true;
        frame.stats[Q1_STAT_MONSTERS] = 7;
        frame.stats[Q1_STAT_TOTALMONSTERS] = 12;
        frame.stats[Q1_STAT_SECRETS] = 1;
        frame.stats[Q1_STAT_TOTALSECRETS] = 3;
        frame.time = 125.0;
        frame.levelname = "e1m1".to_string();
        let ops = q1_sbar_operations(&frame, 320, 200, 48);
        let pics = pictures(&ops);
        assert!(pics.contains(&(0, 176, "scorebar")), "{pics:?}");
        let text: String = ops
            .iter()
            .filter_map(|op| match op {
                NativeQ1HudOperation::Char { code, .. } => char::from_u32(*code as u32),
                _ => None,
            })
            .collect();
        assert!(text.contains("Monsters:  7 / 12"), "{text:?}");
        assert!(text.contains("Secrets :  1 /  3"), "{text:?}");
        assert!(text.contains("Time :  2:05"), "{text:?}");
        assert!(text.contains("e1m1"), "{text:?}");
    }

    #[test]
    fn deathmatch_bar_draws_uncentered() {
        let frame = frame();
        let centered = q1_sbar_operations(&frame, 640, 480, 48);
        let pics = pictures(&centered);
        assert!(pics.contains(&(160, 456, "sbar")), "{pics:?}");
        let mut dm = frame;
        dm.deathmatch = true;
        dm.maxclients = 2;
        let ops = q1_sbar_operations(&dm, 640, 480, 48);
        let pics = pictures(&ops);
        assert!(pics.contains(&(0, 456, "sbar")), "{pics:?}");
    }

    #[test]
    fn hipnotic_moves_keys_and_adds_weapons() {
        let mut frame = frame();
        frame.product = Q1SbarProduct::Hipnotic;
        frame.items = Q1_IT_KEY1 | (1 << 23);
        frame.stats[Q1_STAT_ACTIVEWEAPON] = 1 << 23;
        frame.time = 100.0;
        let ops = q1_sbar_operations(&frame, 320, 200, 48);
        let pics = pictures(&ops);
        assert!(pics.contains(&(209, 179, "sb_key1")), "{pics:?}");
        assert!(
            !pics.iter().any(|(x, y, l)| (*x, *y) == (192, 160) && *l == "sb_key1"),
            "{pics:?}"
        );
        assert!(pics.contains(&(176, 160, "inv2_laser")), "{pics:?}");
    }

    #[test]
    fn rogue_selects_invbar_armor_and_powered_weapons() {
        let mut frame = frame();
        frame.product = Q1SbarProduct::Rogue;
        frame.items = Q1_RIT_ARMOR2 | Q1_RIT_SHELLS;
        frame.stats[Q1_STAT_ACTIVEWEAPON] = Q1_RIT_LAVA_NAILGUN as i32;
        frame.stats[Q1_STAT_ARMOR] = 100;
        let ops = q1_sbar_operations(&frame, 320, 200, 48);
        let pics = pictures(&ops);
        assert!(pics.contains(&(0, 152, "r_invbar1")), "{pics:?}");
        assert!(pics.contains(&(48, 160, "r_lava")), "{pics:?}");
        assert!(pics.contains(&(0, 176, "sb_armor2")), "{pics:?}");
        assert!(pics.contains(&(224, 176, "sb_shells")), "{pics:?}");
    }

    #[test]
    fn intermission_shows_plates_and_tallies() {
        let mut frame = frame();
        frame.completed_time = 183.0;
        frame.stats[Q1_STAT_SECRETS] = 2;
        frame.stats[Q1_STAT_TOTALSECRETS] = 5;
        frame.stats[Q1_STAT_MONSTERS] = 30;
        frame.stats[Q1_STAT_TOTALMONSTERS] = 44;
        let ops = q1_intermission_operations(&frame, 320);
        let pics = pictures(&ops);
        assert!(pics.contains(&(64, 24, "gfx/complete.lmp")), "{pics:?}");
        assert!(pics.contains(&(0, 56, "gfx/inter.lmp")), "{pics:?}");
        assert!(pics.contains(&(234, 64, "num_colon")), "{pics:?}");
        assert!(pics.contains(&(232, 104, "num_slash")), "{pics:?}");
        // 183 s = 3:03; tallies right-aligned in 3-digit fields.
        assert!(pics.contains(&(208, 64, "num_3")), "{pics:?}");
        assert!(pics.contains(&(246, 64, "num_0")), "{pics:?}");
        assert!(pics.contains(&(266, 64, "num_3")), "{pics:?}");
        assert!(pics.contains(&(208, 104, "num_2")), "{pics:?}");
        assert!(pics.contains(&(288, 104, "num_5")), "{pics:?}");
        assert!(pics.contains(&(184, 144, "num_3")), "{pics:?}");
        assert!(pics.contains(&(288, 144, "num_4")), "{pics:?}");
    }
}
