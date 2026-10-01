//! fixbot move tables (`src/content/q2/missionpacks/monsters/tables/xatrix-fixbot.ts`).
//!
//! Original Quake II xatrix/m_fixbot.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `fixbotFrame`.
pub mod fixbot_frame {
    /// Frame `charging_01`.
    pub const CHARGING_01: i32 = 0;
    /// Frame `charging_02`.
    pub const CHARGING_02: i32 = 1;
    /// Frame `charging_03`.
    pub const CHARGING_03: i32 = 2;
    /// Frame `charging_04`.
    pub const CHARGING_04: i32 = 3;
    /// Frame `charging_05`.
    pub const CHARGING_05: i32 = 4;
    /// Frame `charging_06`.
    pub const CHARGING_06: i32 = 5;
    /// Frame `charging_07`.
    pub const CHARGING_07: i32 = 6;
    /// Frame `charging_08`.
    pub const CHARGING_08: i32 = 7;
    /// Frame `charging_09`.
    pub const CHARGING_09: i32 = 8;
    /// Frame `charging_10`.
    pub const CHARGING_10: i32 = 9;
    /// Frame `charging_11`.
    pub const CHARGING_11: i32 = 10;
    /// Frame `charging_12`.
    pub const CHARGING_12: i32 = 11;
    /// Frame `charging_13`.
    pub const CHARGING_13: i32 = 12;
    /// Frame `charging_14`.
    pub const CHARGING_14: i32 = 13;
    /// Frame `charging_15`.
    pub const CHARGING_15: i32 = 14;
    /// Frame `charging_16`.
    pub const CHARGING_16: i32 = 15;
    /// Frame `charging_17`.
    pub const CHARGING_17: i32 = 16;
    /// Frame `charging_18`.
    pub const CHARGING_18: i32 = 17;
    /// Frame `charging_19`.
    pub const CHARGING_19: i32 = 18;
    /// Frame `charging_20`.
    pub const CHARGING_20: i32 = 19;
    /// Frame `charging_21`.
    pub const CHARGING_21: i32 = 20;
    /// Frame `charging_22`.
    pub const CHARGING_22: i32 = 21;
    /// Frame `charging_23`.
    pub const CHARGING_23: i32 = 22;
    /// Frame `charging_24`.
    pub const CHARGING_24: i32 = 23;
    /// Frame `charging_25`.
    pub const CHARGING_25: i32 = 24;
    /// Frame `charging_26`.
    pub const CHARGING_26: i32 = 25;
    /// Frame `charging_27`.
    pub const CHARGING_27: i32 = 26;
    /// Frame `charging_28`.
    pub const CHARGING_28: i32 = 27;
    /// Frame `charging_29`.
    pub const CHARGING_29: i32 = 28;
    /// Frame `charging_30`.
    pub const CHARGING_30: i32 = 29;
    /// Frame `charging_31`.
    pub const CHARGING_31: i32 = 30;
    /// Frame `landing_01`.
    pub const LANDING_01: i32 = 31;
    /// Frame `landing_02`.
    pub const LANDING_02: i32 = 32;
    /// Frame `landing_03`.
    pub const LANDING_03: i32 = 33;
    /// Frame `landing_04`.
    pub const LANDING_04: i32 = 34;
    /// Frame `landing_05`.
    pub const LANDING_05: i32 = 35;
    /// Frame `landing_06`.
    pub const LANDING_06: i32 = 36;
    /// Frame `landing_07`.
    pub const LANDING_07: i32 = 37;
    /// Frame `landing_08`.
    pub const LANDING_08: i32 = 38;
    /// Frame `landing_09`.
    pub const LANDING_09: i32 = 39;
    /// Frame `landing_10`.
    pub const LANDING_10: i32 = 40;
    /// Frame `landing_11`.
    pub const LANDING_11: i32 = 41;
    /// Frame `landing_12`.
    pub const LANDING_12: i32 = 42;
    /// Frame `landing_13`.
    pub const LANDING_13: i32 = 43;
    /// Frame `landing_14`.
    pub const LANDING_14: i32 = 44;
    /// Frame `landing_15`.
    pub const LANDING_15: i32 = 45;
    /// Frame `landing_16`.
    pub const LANDING_16: i32 = 46;
    /// Frame `landing_17`.
    pub const LANDING_17: i32 = 47;
    /// Frame `landing_18`.
    pub const LANDING_18: i32 = 48;
    /// Frame `landing_19`.
    pub const LANDING_19: i32 = 49;
    /// Frame `landing_20`.
    pub const LANDING_20: i32 = 50;
    /// Frame `landing_21`.
    pub const LANDING_21: i32 = 51;
    /// Frame `landing_22`.
    pub const LANDING_22: i32 = 52;
    /// Frame `landing_23`.
    pub const LANDING_23: i32 = 53;
    /// Frame `landing_24`.
    pub const LANDING_24: i32 = 54;
    /// Frame `landing_25`.
    pub const LANDING_25: i32 = 55;
    /// Frame `landing_26`.
    pub const LANDING_26: i32 = 56;
    /// Frame `landing_27`.
    pub const LANDING_27: i32 = 57;
    /// Frame `landing_28`.
    pub const LANDING_28: i32 = 58;
    /// Frame `landing_29`.
    pub const LANDING_29: i32 = 59;
    /// Frame `landing_30`.
    pub const LANDING_30: i32 = 60;
    /// Frame `landing_31`.
    pub const LANDING_31: i32 = 61;
    /// Frame `landing_32`.
    pub const LANDING_32: i32 = 62;
    /// Frame `landing_33`.
    pub const LANDING_33: i32 = 63;
    /// Frame `landing_34`.
    pub const LANDING_34: i32 = 64;
    /// Frame `landing_35`.
    pub const LANDING_35: i32 = 65;
    /// Frame `landing_36`.
    pub const LANDING_36: i32 = 66;
    /// Frame `landing_37`.
    pub const LANDING_37: i32 = 67;
    /// Frame `landing_38`.
    pub const LANDING_38: i32 = 68;
    /// Frame `landing_39`.
    pub const LANDING_39: i32 = 69;
    /// Frame `landing_40`.
    pub const LANDING_40: i32 = 70;
    /// Frame `landing_41`.
    pub const LANDING_41: i32 = 71;
    /// Frame `landing_42`.
    pub const LANDING_42: i32 = 72;
    /// Frame `landing_43`.
    pub const LANDING_43: i32 = 73;
    /// Frame `landing_44`.
    pub const LANDING_44: i32 = 74;
    /// Frame `landing_45`.
    pub const LANDING_45: i32 = 75;
    /// Frame `landing_46`.
    pub const LANDING_46: i32 = 76;
    /// Frame `landing_47`.
    pub const LANDING_47: i32 = 77;
    /// Frame `landing_48`.
    pub const LANDING_48: i32 = 78;
    /// Frame `landing_49`.
    pub const LANDING_49: i32 = 79;
    /// Frame `landing_50`.
    pub const LANDING_50: i32 = 80;
    /// Frame `landing_51`.
    pub const LANDING_51: i32 = 81;
    /// Frame `landing_52`.
    pub const LANDING_52: i32 = 82;
    /// Frame `landing_53`.
    pub const LANDING_53: i32 = 83;
    /// Frame `landing_54`.
    pub const LANDING_54: i32 = 84;
    /// Frame `landing_55`.
    pub const LANDING_55: i32 = 85;
    /// Frame `landing_56`.
    pub const LANDING_56: i32 = 86;
    /// Frame `landing_57`.
    pub const LANDING_57: i32 = 87;
    /// Frame `landing_58`.
    pub const LANDING_58: i32 = 88;
    /// Frame `pushback_01`.
    pub const PUSHBACK_01: i32 = 89;
    /// Frame `pushback_02`.
    pub const PUSHBACK_02: i32 = 90;
    /// Frame `pushback_03`.
    pub const PUSHBACK_03: i32 = 91;
    /// Frame `pushback_04`.
    pub const PUSHBACK_04: i32 = 92;
    /// Frame `pushback_05`.
    pub const PUSHBACK_05: i32 = 93;
    /// Frame `pushback_06`.
    pub const PUSHBACK_06: i32 = 94;
    /// Frame `pushback_07`.
    pub const PUSHBACK_07: i32 = 95;
    /// Frame `pushback_08`.
    pub const PUSHBACK_08: i32 = 96;
    /// Frame `pushback_09`.
    pub const PUSHBACK_09: i32 = 97;
    /// Frame `pushback_10`.
    pub const PUSHBACK_10: i32 = 98;
    /// Frame `pushback_11`.
    pub const PUSHBACK_11: i32 = 99;
    /// Frame `pushback_12`.
    pub const PUSHBACK_12: i32 = 100;
    /// Frame `pushback_13`.
    pub const PUSHBACK_13: i32 = 101;
    /// Frame `pushback_14`.
    pub const PUSHBACK_14: i32 = 102;
    /// Frame `pushback_15`.
    pub const PUSHBACK_15: i32 = 103;
    /// Frame `pushback_16`.
    pub const PUSHBACK_16: i32 = 104;
    /// Frame `takeoff_01`.
    pub const TAKEOFF_01: i32 = 105;
    /// Frame `takeoff_02`.
    pub const TAKEOFF_02: i32 = 106;
    /// Frame `takeoff_03`.
    pub const TAKEOFF_03: i32 = 107;
    /// Frame `takeoff_04`.
    pub const TAKEOFF_04: i32 = 108;
    /// Frame `takeoff_05`.
    pub const TAKEOFF_05: i32 = 109;
    /// Frame `takeoff_06`.
    pub const TAKEOFF_06: i32 = 110;
    /// Frame `takeoff_07`.
    pub const TAKEOFF_07: i32 = 111;
    /// Frame `takeoff_08`.
    pub const TAKEOFF_08: i32 = 112;
    /// Frame `takeoff_09`.
    pub const TAKEOFF_09: i32 = 113;
    /// Frame `takeoff_10`.
    pub const TAKEOFF_10: i32 = 114;
    /// Frame `takeoff_11`.
    pub const TAKEOFF_11: i32 = 115;
    /// Frame `takeoff_12`.
    pub const TAKEOFF_12: i32 = 116;
    /// Frame `takeoff_13`.
    pub const TAKEOFF_13: i32 = 117;
    /// Frame `takeoff_14`.
    pub const TAKEOFF_14: i32 = 118;
    /// Frame `takeoff_15`.
    pub const TAKEOFF_15: i32 = 119;
    /// Frame `takeoff_16`.
    pub const TAKEOFF_16: i32 = 120;
    /// Frame `ambient_01`.
    pub const AMBIENT_01: i32 = 121;
    /// Frame `ambient_02`.
    pub const AMBIENT_02: i32 = 122;
    /// Frame `ambient_03`.
    pub const AMBIENT_03: i32 = 123;
    /// Frame `ambient_04`.
    pub const AMBIENT_04: i32 = 124;
    /// Frame `ambient_05`.
    pub const AMBIENT_05: i32 = 125;
    /// Frame `ambient_06`.
    pub const AMBIENT_06: i32 = 126;
    /// Frame `ambient_07`.
    pub const AMBIENT_07: i32 = 127;
    /// Frame `ambient_08`.
    pub const AMBIENT_08: i32 = 128;
    /// Frame `ambient_09`.
    pub const AMBIENT_09: i32 = 129;
    /// Frame `ambient_10`.
    pub const AMBIENT_10: i32 = 130;
    /// Frame `ambient_11`.
    pub const AMBIENT_11: i32 = 131;
    /// Frame `ambient_12`.
    pub const AMBIENT_12: i32 = 132;
    /// Frame `ambient_13`.
    pub const AMBIENT_13: i32 = 133;
    /// Frame `ambient_14`.
    pub const AMBIENT_14: i32 = 134;
    /// Frame `ambient_15`.
    pub const AMBIENT_15: i32 = 135;
    /// Frame `ambient_16`.
    pub const AMBIENT_16: i32 = 136;
    /// Frame `ambient_17`.
    pub const AMBIENT_17: i32 = 137;
    /// Frame `ambient_18`.
    pub const AMBIENT_18: i32 = 138;
    /// Frame `ambient_19`.
    pub const AMBIENT_19: i32 = 139;
    /// Frame `paina_01`.
    pub const PAINA_01: i32 = 140;
    /// Frame `paina_02`.
    pub const PAINA_02: i32 = 141;
    /// Frame `paina_03`.
    pub const PAINA_03: i32 = 142;
    /// Frame `paina_04`.
    pub const PAINA_04: i32 = 143;
    /// Frame `paina_05`.
    pub const PAINA_05: i32 = 144;
    /// Frame `paina_06`.
    pub const PAINA_06: i32 = 145;
    /// Frame `painb_01`.
    pub const PAINB_01: i32 = 146;
    /// Frame `painb_02`.
    pub const PAINB_02: i32 = 147;
    /// Frame `painb_03`.
    pub const PAINB_03: i32 = 148;
    /// Frame `painb_04`.
    pub const PAINB_04: i32 = 149;
    /// Frame `painb_05`.
    pub const PAINB_05: i32 = 150;
    /// Frame `painb_06`.
    pub const PAINB_06: i32 = 151;
    /// Frame `painb_07`.
    pub const PAINB_07: i32 = 152;
    /// Frame `painb_08`.
    pub const PAINB_08: i32 = 153;
    /// Frame `pickup_01`.
    pub const PICKUP_01: i32 = 154;
    /// Frame `pickup_02`.
    pub const PICKUP_02: i32 = 155;
    /// Frame `pickup_03`.
    pub const PICKUP_03: i32 = 156;
    /// Frame `pickup_04`.
    pub const PICKUP_04: i32 = 157;
    /// Frame `pickup_05`.
    pub const PICKUP_05: i32 = 158;
    /// Frame `pickup_06`.
    pub const PICKUP_06: i32 = 159;
    /// Frame `pickup_07`.
    pub const PICKUP_07: i32 = 160;
    /// Frame `pickup_08`.
    pub const PICKUP_08: i32 = 161;
    /// Frame `pickup_09`.
    pub const PICKUP_09: i32 = 162;
    /// Frame `pickup_10`.
    pub const PICKUP_10: i32 = 163;
    /// Frame `pickup_11`.
    pub const PICKUP_11: i32 = 164;
    /// Frame `pickup_12`.
    pub const PICKUP_12: i32 = 165;
    /// Frame `pickup_13`.
    pub const PICKUP_13: i32 = 166;
    /// Frame `pickup_14`.
    pub const PICKUP_14: i32 = 167;
    /// Frame `pickup_15`.
    pub const PICKUP_15: i32 = 168;
    /// Frame `pickup_16`.
    pub const PICKUP_16: i32 = 169;
    /// Frame `pickup_17`.
    pub const PICKUP_17: i32 = 170;
    /// Frame `pickup_18`.
    pub const PICKUP_18: i32 = 171;
    /// Frame `pickup_19`.
    pub const PICKUP_19: i32 = 172;
    /// Frame `pickup_20`.
    pub const PICKUP_20: i32 = 173;
    /// Frame `pickup_21`.
    pub const PICKUP_21: i32 = 174;
    /// Frame `pickup_22`.
    pub const PICKUP_22: i32 = 175;
    /// Frame `pickup_23`.
    pub const PICKUP_23: i32 = 176;
    /// Frame `pickup_24`.
    pub const PICKUP_24: i32 = 177;
    /// Frame `pickup_25`.
    pub const PICKUP_25: i32 = 178;
    /// Frame `pickup_26`.
    pub const PICKUP_26: i32 = 179;
    /// Frame `pickup_27`.
    pub const PICKUP_27: i32 = 180;
    /// Frame `freeze_01`.
    pub const FREEZE_01: i32 = 181;
    /// Frame `shoot_01`.
    pub const SHOOT_01: i32 = 182;
    /// Frame `shoot_02`.
    pub const SHOOT_02: i32 = 183;
    /// Frame `shoot_03`.
    pub const SHOOT_03: i32 = 184;
    /// Frame `shoot_04`.
    pub const SHOOT_04: i32 = 185;
    /// Frame `shoot_05`.
    pub const SHOOT_05: i32 = 186;
    /// Frame `shoot_06`.
    pub const SHOOT_06: i32 = 187;
    /// Frame `weldstart_01`.
    pub const WELDSTART_01: i32 = 188;
    /// Frame `weldstart_02`.
    pub const WELDSTART_02: i32 = 189;
    /// Frame `weldstart_03`.
    pub const WELDSTART_03: i32 = 190;
    /// Frame `weldstart_04`.
    pub const WELDSTART_04: i32 = 191;
    /// Frame `weldstart_05`.
    pub const WELDSTART_05: i32 = 192;
    /// Frame `weldstart_06`.
    pub const WELDSTART_06: i32 = 193;
    /// Frame `weldstart_07`.
    pub const WELDSTART_07: i32 = 194;
    /// Frame `weldstart_08`.
    pub const WELDSTART_08: i32 = 195;
    /// Frame `weldstart_09`.
    pub const WELDSTART_09: i32 = 196;
    /// Frame `weldstart_10`.
    pub const WELDSTART_10: i32 = 197;
    /// Frame `weldmiddle_01`.
    pub const WELDMIDDLE_01: i32 = 198;
    /// Frame `weldmiddle_02`.
    pub const WELDMIDDLE_02: i32 = 199;
    /// Frame `weldmiddle_03`.
    pub const WELDMIDDLE_03: i32 = 200;
    /// Frame `weldmiddle_04`.
    pub const WELDMIDDLE_04: i32 = 201;
    /// Frame `weldmiddle_05`.
    pub const WELDMIDDLE_05: i32 = 202;
    /// Frame `weldmiddle_06`.
    pub const WELDMIDDLE_06: i32 = 203;
    /// Frame `weldmiddle_07`.
    pub const WELDMIDDLE_07: i32 = 204;
    /// Frame `weldend_01`.
    pub const WELDEND_01: i32 = 205;
    /// Frame `weldend_02`.
    pub const WELDEND_02: i32 = 206;
    /// Frame `weldend_03`.
    pub const WELDEND_03: i32 = 207;
    /// Frame `weldend_04`.
    pub const WELDEND_04: i32 = 208;
    /// Frame `weldend_05`.
    pub const WELDEND_05: i32 = 209;
    /// Frame `weldend_06`.
    pub const WELDEND_06: i32 = 210;
    /// Frame `weldend_07`.
    pub const WELDEND_07: i32 = 211;
}

/// `fixbotMoves` move tables.
pub fn fixbot_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "fixbot_move_landing",
            31,
            88,
            None,
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("fly_vertical2")], -1),
            ],
        ),
        monster_move(
            "fixbot_move_stand",
            121,
            139,
            None,
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("change_to_roam")], -1),
            ],
        ),
        monster_move(
            "fixbot_move_stand2",
            121,
            139,
            None,
            vec![
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "fixbot_move_pickup",
            154,
            180,
            None,
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "fixbot_move_roamgoal",
            181,
            181,
            None,
            vec![monster_frame(
                MonsterAi::Move,
                0.0,
                vec![MonsterAction::name("roam_goal")],
                -1,
            )],
        ),
        monster_move(
            "fixbot_move_turn",
            181,
            181,
            None,
            vec![monster_frame(
                MonsterAi::Source("ai_facing".to_string()),
                0.0,
                vec![],
                -1,
            )],
        ),
        monster_move(
            "fixbot_move_takeoff",
            105,
            120,
            None,
            vec![
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
                monster_frame(MonsterAi::Move, 0.01, vec![MonsterAction::name("fly_vertical")], -1),
            ],
        ),
        monster_move(
            "fixbot_move_paina",
            140,
            145,
            Some("fixbot_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "fixbot_move_painb",
            146,
            153,
            Some("fixbot_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "fixbot_move_pain3",
            181,
            181,
            Some("fixbot_run"),
            vec![monster_frame(MonsterAi::Move, -1.0, vec![], -1)],
        ),
        monster_move(
            "fixbot_move_land",
            181,
            181,
            None,
            vec![monster_frame(MonsterAi::Move, 0.0, vec![], -1)],
        ),
        monster_move(
            "fixbot_move_forward",
            181,
            181,
            None,
            vec![monster_frame(
                MonsterAi::Source("ai_movetogoal".to_string()),
                5.0,
                vec![MonsterAction::name("use_scanner")],
                -1,
            )],
        ),
        monster_move(
            "fixbot_move_walk",
            181,
            181,
            None,
            vec![monster_frame(MonsterAi::Walk, 5.0, vec![], -1)],
        ),
        monster_move(
            "fixbot_move_run",
            181,
            181,
            None,
            vec![monster_frame(MonsterAi::Run, 10.0, vec![], -1)],
        ),
        monster_move(
            "fixbot_move_death1",
            181,
            181,
            Some("fixbot_dead"),
            vec![monster_frame(MonsterAi::Move, 0.0, vec![], -1)],
        ),
        monster_move(
            "fixbot_move_backward",
            181,
            181,
            None,
            vec![monster_frame(MonsterAi::Move, 0.0, vec![], -1)],
        ),
        monster_move(
            "fixbot_move_start_attack",
            181,
            181,
            Some("fixbot_attack"),
            vec![monster_frame(MonsterAi::Charge, 0.0, vec![], -1)],
        ),
        monster_move(
            "fixbot_move_attack1",
            182,
            187,
            None,
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    -10.0,
                    vec![MonsterAction::name("fixbot_fire_blaster")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "fixbot_move_laserattack",
            182,
            187,
            None,
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_laser")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_laser")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_laser")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_laser")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_laser")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_laser")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "fixbot_move_attack2",
            0,
            30,
            Some("fixbot_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -10.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_blaster")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "fixbot_move_weld_start",
            188,
            197,
            None,
            vec![
                monster_frame(MonsterAi::Source("ai_move2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), 0.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Source("ai_move2".to_string()),
                    0.0,
                    vec![MonsterAction::name("weldstate")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "fixbot_move_weld",
            198,
            204,
            None,
            vec![
                monster_frame(
                    MonsterAi::Source("ai_move2".to_string()),
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_welder")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Source("ai_move2".to_string()),
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_welder")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Source("ai_move2".to_string()),
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_welder")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Source("ai_move2".to_string()),
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_welder")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Source("ai_move2".to_string()),
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_welder")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Source("ai_move2".to_string()),
                    0.0,
                    vec![MonsterAction::name("fixbot_fire_welder")],
                    -1,
                ),
                monster_frame(
                    MonsterAi::Source("ai_move2".to_string()),
                    0.0,
                    vec![MonsterAction::name("weldstate")],
                    -1,
                ),
            ],
        ),
        monster_move(
            "fixbot_move_weld_end",
            205,
            211,
            None,
            vec![
                monster_frame(MonsterAi::Source("ai_move2".to_string()), -2.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), -2.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), -2.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), -2.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), -2.0, vec![], -1),
                monster_frame(MonsterAi::Source("ai_move2".to_string()), -2.0, vec![], -1),
                monster_frame(
                    MonsterAi::Source("ai_move2".to_string()),
                    -2.0,
                    vec![MonsterAction::name("weldstate")],
                    -1,
                ),
            ],
        ),
    ]
}
