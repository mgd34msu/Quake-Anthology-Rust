//! medic move tables (`src/content/q2/missionpacks/monsters/tables/rogue-medic.ts`).
//!
//! Original Quake II rogue/m_medic.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `medicFrame`.
pub mod medic_frame {
    /// Frame `walk1`.
    pub const WALK1: i32 = 0;
    /// Frame `walk2`.
    pub const WALK2: i32 = 1;
    /// Frame `walk3`.
    pub const WALK3: i32 = 2;
    /// Frame `walk4`.
    pub const WALK4: i32 = 3;
    /// Frame `walk5`.
    pub const WALK5: i32 = 4;
    /// Frame `walk6`.
    pub const WALK6: i32 = 5;
    /// Frame `walk7`.
    pub const WALK7: i32 = 6;
    /// Frame `walk8`.
    pub const WALK8: i32 = 7;
    /// Frame `walk9`.
    pub const WALK9: i32 = 8;
    /// Frame `walk10`.
    pub const WALK10: i32 = 9;
    /// Frame `walk11`.
    pub const WALK11: i32 = 10;
    /// Frame `walk12`.
    pub const WALK12: i32 = 11;
    /// Frame `wait1`.
    pub const WAIT1: i32 = 12;
    /// Frame `wait2`.
    pub const WAIT2: i32 = 13;
    /// Frame `wait3`.
    pub const WAIT3: i32 = 14;
    /// Frame `wait4`.
    pub const WAIT4: i32 = 15;
    /// Frame `wait5`.
    pub const WAIT5: i32 = 16;
    /// Frame `wait6`.
    pub const WAIT6: i32 = 17;
    /// Frame `wait7`.
    pub const WAIT7: i32 = 18;
    /// Frame `wait8`.
    pub const WAIT8: i32 = 19;
    /// Frame `wait9`.
    pub const WAIT9: i32 = 20;
    /// Frame `wait10`.
    pub const WAIT10: i32 = 21;
    /// Frame `wait11`.
    pub const WAIT11: i32 = 22;
    /// Frame `wait12`.
    pub const WAIT12: i32 = 23;
    /// Frame `wait13`.
    pub const WAIT13: i32 = 24;
    /// Frame `wait14`.
    pub const WAIT14: i32 = 25;
    /// Frame `wait15`.
    pub const WAIT15: i32 = 26;
    /// Frame `wait16`.
    pub const WAIT16: i32 = 27;
    /// Frame `wait17`.
    pub const WAIT17: i32 = 28;
    /// Frame `wait18`.
    pub const WAIT18: i32 = 29;
    /// Frame `wait19`.
    pub const WAIT19: i32 = 30;
    /// Frame `wait20`.
    pub const WAIT20: i32 = 31;
    /// Frame `wait21`.
    pub const WAIT21: i32 = 32;
    /// Frame `wait22`.
    pub const WAIT22: i32 = 33;
    /// Frame `wait23`.
    pub const WAIT23: i32 = 34;
    /// Frame `wait24`.
    pub const WAIT24: i32 = 35;
    /// Frame `wait25`.
    pub const WAIT25: i32 = 36;
    /// Frame `wait26`.
    pub const WAIT26: i32 = 37;
    /// Frame `wait27`.
    pub const WAIT27: i32 = 38;
    /// Frame `wait28`.
    pub const WAIT28: i32 = 39;
    /// Frame `wait29`.
    pub const WAIT29: i32 = 40;
    /// Frame `wait30`.
    pub const WAIT30: i32 = 41;
    /// Frame `wait31`.
    pub const WAIT31: i32 = 42;
    /// Frame `wait32`.
    pub const WAIT32: i32 = 43;
    /// Frame `wait33`.
    pub const WAIT33: i32 = 44;
    /// Frame `wait34`.
    pub const WAIT34: i32 = 45;
    /// Frame `wait35`.
    pub const WAIT35: i32 = 46;
    /// Frame `wait36`.
    pub const WAIT36: i32 = 47;
    /// Frame `wait37`.
    pub const WAIT37: i32 = 48;
    /// Frame `wait38`.
    pub const WAIT38: i32 = 49;
    /// Frame `wait39`.
    pub const WAIT39: i32 = 50;
    /// Frame `wait40`.
    pub const WAIT40: i32 = 51;
    /// Frame `wait41`.
    pub const WAIT41: i32 = 52;
    /// Frame `wait42`.
    pub const WAIT42: i32 = 53;
    /// Frame `wait43`.
    pub const WAIT43: i32 = 54;
    /// Frame `wait44`.
    pub const WAIT44: i32 = 55;
    /// Frame `wait45`.
    pub const WAIT45: i32 = 56;
    /// Frame `wait46`.
    pub const WAIT46: i32 = 57;
    /// Frame `wait47`.
    pub const WAIT47: i32 = 58;
    /// Frame `wait48`.
    pub const WAIT48: i32 = 59;
    /// Frame `wait49`.
    pub const WAIT49: i32 = 60;
    /// Frame `wait50`.
    pub const WAIT50: i32 = 61;
    /// Frame `wait51`.
    pub const WAIT51: i32 = 62;
    /// Frame `wait52`.
    pub const WAIT52: i32 = 63;
    /// Frame `wait53`.
    pub const WAIT53: i32 = 64;
    /// Frame `wait54`.
    pub const WAIT54: i32 = 65;
    /// Frame `wait55`.
    pub const WAIT55: i32 = 66;
    /// Frame `wait56`.
    pub const WAIT56: i32 = 67;
    /// Frame `wait57`.
    pub const WAIT57: i32 = 68;
    /// Frame `wait58`.
    pub const WAIT58: i32 = 69;
    /// Frame `wait59`.
    pub const WAIT59: i32 = 70;
    /// Frame `wait60`.
    pub const WAIT60: i32 = 71;
    /// Frame `wait61`.
    pub const WAIT61: i32 = 72;
    /// Frame `wait62`.
    pub const WAIT62: i32 = 73;
    /// Frame `wait63`.
    pub const WAIT63: i32 = 74;
    /// Frame `wait64`.
    pub const WAIT64: i32 = 75;
    /// Frame `wait65`.
    pub const WAIT65: i32 = 76;
    /// Frame `wait66`.
    pub const WAIT66: i32 = 77;
    /// Frame `wait67`.
    pub const WAIT67: i32 = 78;
    /// Frame `wait68`.
    pub const WAIT68: i32 = 79;
    /// Frame `wait69`.
    pub const WAIT69: i32 = 80;
    /// Frame `wait70`.
    pub const WAIT70: i32 = 81;
    /// Frame `wait71`.
    pub const WAIT71: i32 = 82;
    /// Frame `wait72`.
    pub const WAIT72: i32 = 83;
    /// Frame `wait73`.
    pub const WAIT73: i32 = 84;
    /// Frame `wait74`.
    pub const WAIT74: i32 = 85;
    /// Frame `wait75`.
    pub const WAIT75: i32 = 86;
    /// Frame `wait76`.
    pub const WAIT76: i32 = 87;
    /// Frame `wait77`.
    pub const WAIT77: i32 = 88;
    /// Frame `wait78`.
    pub const WAIT78: i32 = 89;
    /// Frame `wait79`.
    pub const WAIT79: i32 = 90;
    /// Frame `wait80`.
    pub const WAIT80: i32 = 91;
    /// Frame `wait81`.
    pub const WAIT81: i32 = 92;
    /// Frame `wait82`.
    pub const WAIT82: i32 = 93;
    /// Frame `wait83`.
    pub const WAIT83: i32 = 94;
    /// Frame `wait84`.
    pub const WAIT84: i32 = 95;
    /// Frame `wait85`.
    pub const WAIT85: i32 = 96;
    /// Frame `wait86`.
    pub const WAIT86: i32 = 97;
    /// Frame `wait87`.
    pub const WAIT87: i32 = 98;
    /// Frame `wait88`.
    pub const WAIT88: i32 = 99;
    /// Frame `wait89`.
    pub const WAIT89: i32 = 100;
    /// Frame `wait90`.
    pub const WAIT90: i32 = 101;
    /// Frame `run1`.
    pub const RUN1: i32 = 102;
    /// Frame `run2`.
    pub const RUN2: i32 = 103;
    /// Frame `run3`.
    pub const RUN3: i32 = 104;
    /// Frame `run4`.
    pub const RUN4: i32 = 105;
    /// Frame `run5`.
    pub const RUN5: i32 = 106;
    /// Frame `run6`.
    pub const RUN6: i32 = 107;
    /// Frame `paina1`.
    pub const PAINA1: i32 = 108;
    /// Frame `paina2`.
    pub const PAINA2: i32 = 109;
    /// Frame `paina3`.
    pub const PAINA3: i32 = 110;
    /// Frame `paina4`.
    pub const PAINA4: i32 = 111;
    /// Frame `paina5`.
    pub const PAINA5: i32 = 112;
    /// Frame `paina6`.
    pub const PAINA6: i32 = 113;
    /// Frame `paina7`.
    pub const PAINA7: i32 = 114;
    /// Frame `paina8`.
    pub const PAINA8: i32 = 115;
    /// Frame `painb1`.
    pub const PAINB1: i32 = 116;
    /// Frame `painb2`.
    pub const PAINB2: i32 = 117;
    /// Frame `painb3`.
    pub const PAINB3: i32 = 118;
    /// Frame `painb4`.
    pub const PAINB4: i32 = 119;
    /// Frame `painb5`.
    pub const PAINB5: i32 = 120;
    /// Frame `painb6`.
    pub const PAINB6: i32 = 121;
    /// Frame `painb7`.
    pub const PAINB7: i32 = 122;
    /// Frame `painb8`.
    pub const PAINB8: i32 = 123;
    /// Frame `painb9`.
    pub const PAINB9: i32 = 124;
    /// Frame `painb10`.
    pub const PAINB10: i32 = 125;
    /// Frame `painb11`.
    pub const PAINB11: i32 = 126;
    /// Frame `painb12`.
    pub const PAINB12: i32 = 127;
    /// Frame `painb13`.
    pub const PAINB13: i32 = 128;
    /// Frame `painb14`.
    pub const PAINB14: i32 = 129;
    /// Frame `painb15`.
    pub const PAINB15: i32 = 130;
    /// Frame `duck1`.
    pub const DUCK1: i32 = 131;
    /// Frame `duck2`.
    pub const DUCK2: i32 = 132;
    /// Frame `duck3`.
    pub const DUCK3: i32 = 133;
    /// Frame `duck4`.
    pub const DUCK4: i32 = 134;
    /// Frame `duck5`.
    pub const DUCK5: i32 = 135;
    /// Frame `duck6`.
    pub const DUCK6: i32 = 136;
    /// Frame `duck7`.
    pub const DUCK7: i32 = 137;
    /// Frame `duck8`.
    pub const DUCK8: i32 = 138;
    /// Frame `duck9`.
    pub const DUCK9: i32 = 139;
    /// Frame `duck10`.
    pub const DUCK10: i32 = 140;
    /// Frame `duck11`.
    pub const DUCK11: i32 = 141;
    /// Frame `duck12`.
    pub const DUCK12: i32 = 142;
    /// Frame `duck13`.
    pub const DUCK13: i32 = 143;
    /// Frame `duck14`.
    pub const DUCK14: i32 = 144;
    /// Frame `duck15`.
    pub const DUCK15: i32 = 145;
    /// Frame `duck16`.
    pub const DUCK16: i32 = 146;
    /// Frame `death1`.
    pub const DEATH1: i32 = 147;
    /// Frame `death2`.
    pub const DEATH2: i32 = 148;
    /// Frame `death3`.
    pub const DEATH3: i32 = 149;
    /// Frame `death4`.
    pub const DEATH4: i32 = 150;
    /// Frame `death5`.
    pub const DEATH5: i32 = 151;
    /// Frame `death6`.
    pub const DEATH6: i32 = 152;
    /// Frame `death7`.
    pub const DEATH7: i32 = 153;
    /// Frame `death8`.
    pub const DEATH8: i32 = 154;
    /// Frame `death9`.
    pub const DEATH9: i32 = 155;
    /// Frame `death10`.
    pub const DEATH10: i32 = 156;
    /// Frame `death11`.
    pub const DEATH11: i32 = 157;
    /// Frame `death12`.
    pub const DEATH12: i32 = 158;
    /// Frame `death13`.
    pub const DEATH13: i32 = 159;
    /// Frame `death14`.
    pub const DEATH14: i32 = 160;
    /// Frame `death15`.
    pub const DEATH15: i32 = 161;
    /// Frame `death16`.
    pub const DEATH16: i32 = 162;
    /// Frame `death17`.
    pub const DEATH17: i32 = 163;
    /// Frame `death18`.
    pub const DEATH18: i32 = 164;
    /// Frame `death19`.
    pub const DEATH19: i32 = 165;
    /// Frame `death20`.
    pub const DEATH20: i32 = 166;
    /// Frame `death21`.
    pub const DEATH21: i32 = 167;
    /// Frame `death22`.
    pub const DEATH22: i32 = 168;
    /// Frame `death23`.
    pub const DEATH23: i32 = 169;
    /// Frame `death24`.
    pub const DEATH24: i32 = 170;
    /// Frame `death25`.
    pub const DEATH25: i32 = 171;
    /// Frame `death26`.
    pub const DEATH26: i32 = 172;
    /// Frame `death27`.
    pub const DEATH27: i32 = 173;
    /// Frame `death28`.
    pub const DEATH28: i32 = 174;
    /// Frame `death29`.
    pub const DEATH29: i32 = 175;
    /// Frame `death30`.
    pub const DEATH30: i32 = 176;
    /// Frame `attack1`.
    pub const ATTACK1: i32 = 177;
    /// Frame `attack2`.
    pub const ATTACK2: i32 = 178;
    /// Frame `attack3`.
    pub const ATTACK3: i32 = 179;
    /// Frame `attack4`.
    pub const ATTACK4: i32 = 180;
    /// Frame `attack5`.
    pub const ATTACK5: i32 = 181;
    /// Frame `attack6`.
    pub const ATTACK6: i32 = 182;
    /// Frame `attack7`.
    pub const ATTACK7: i32 = 183;
    /// Frame `attack8`.
    pub const ATTACK8: i32 = 184;
    /// Frame `attack9`.
    pub const ATTACK9: i32 = 185;
    /// Frame `attack10`.
    pub const ATTACK10: i32 = 186;
    /// Frame `attack11`.
    pub const ATTACK11: i32 = 187;
    /// Frame `attack12`.
    pub const ATTACK12: i32 = 188;
    /// Frame `attack13`.
    pub const ATTACK13: i32 = 189;
    /// Frame `attack14`.
    pub const ATTACK14: i32 = 190;
    /// Frame `attack15`.
    pub const ATTACK15: i32 = 191;
    /// Frame `attack16`.
    pub const ATTACK16: i32 = 192;
    /// Frame `attack17`.
    pub const ATTACK17: i32 = 193;
    /// Frame `attack18`.
    pub const ATTACK18: i32 = 194;
    /// Frame `attack19`.
    pub const ATTACK19: i32 = 195;
    /// Frame `attack20`.
    pub const ATTACK20: i32 = 196;
    /// Frame `attack21`.
    pub const ATTACK21: i32 = 197;
    /// Frame `attack22`.
    pub const ATTACK22: i32 = 198;
    /// Frame `attack23`.
    pub const ATTACK23: i32 = 199;
    /// Frame `attack24`.
    pub const ATTACK24: i32 = 200;
    /// Frame `attack25`.
    pub const ATTACK25: i32 = 201;
    /// Frame `attack26`.
    pub const ATTACK26: i32 = 202;
    /// Frame `attack27`.
    pub const ATTACK27: i32 = 203;
    /// Frame `attack28`.
    pub const ATTACK28: i32 = 204;
    /// Frame `attack29`.
    pub const ATTACK29: i32 = 205;
    /// Frame `attack30`.
    pub const ATTACK30: i32 = 206;
    /// Frame `attack31`.
    pub const ATTACK31: i32 = 207;
    /// Frame `attack32`.
    pub const ATTACK32: i32 = 208;
    /// Frame `attack33`.
    pub const ATTACK33: i32 = 209;
    /// Frame `attack34`.
    pub const ATTACK34: i32 = 210;
    /// Frame `attack35`.
    pub const ATTACK35: i32 = 211;
    /// Frame `attack36`.
    pub const ATTACK36: i32 = 212;
    /// Frame `attack37`.
    pub const ATTACK37: i32 = 213;
    /// Frame `attack38`.
    pub const ATTACK38: i32 = 214;
    /// Frame `attack39`.
    pub const ATTACK39: i32 = 215;
    /// Frame `attack40`.
    pub const ATTACK40: i32 = 216;
    /// Frame `attack41`.
    pub const ATTACK41: i32 = 217;
    /// Frame `attack42`.
    pub const ATTACK42: i32 = 218;
    /// Frame `attack43`.
    pub const ATTACK43: i32 = 219;
    /// Frame `attack44`.
    pub const ATTACK44: i32 = 220;
    /// Frame `attack45`.
    pub const ATTACK45: i32 = 221;
    /// Frame `attack46`.
    pub const ATTACK46: i32 = 222;
    /// Frame `attack47`.
    pub const ATTACK47: i32 = 223;
    /// Frame `attack48`.
    pub const ATTACK48: i32 = 224;
    /// Frame `attack49`.
    pub const ATTACK49: i32 = 225;
    /// Frame `attack50`.
    pub const ATTACK50: i32 = 226;
    /// Frame `attack51`.
    pub const ATTACK51: i32 = 227;
    /// Frame `attack52`.
    pub const ATTACK52: i32 = 228;
    /// Frame `attack53`.
    pub const ATTACK53: i32 = 229;
    /// Frame `attack54`.
    pub const ATTACK54: i32 = 230;
    /// Frame `attack55`.
    pub const ATTACK55: i32 = 231;
    /// Frame `attack56`.
    pub const ATTACK56: i32 = 232;
    /// Frame `attack57`.
    pub const ATTACK57: i32 = 233;
    /// Frame `attack58`.
    pub const ATTACK58: i32 = 234;
    /// Frame `attack59`.
    pub const ATTACK59: i32 = 235;
    /// Frame `attack60`.
    pub const ATTACK60: i32 = 236;
}

/// `medicMoves` move tables.
pub fn medic_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("medic_move_stand", 12, 101, None, vec![
            monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("medic_idle")], -1),
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
        ]),
        monster_move("medic_move_walk", 0, 11, None, vec![
            monster_frame(MonsterAi::Walk, 6.2, vec![], -1),
            monster_frame(MonsterAi::Walk, 18.1, vec![], -1),
            monster_frame(MonsterAi::Walk, 1.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 9.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 10.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 9.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 11.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 11.6, vec![], -1),
            monster_frame(MonsterAi::Walk, 2.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 9.9, vec![], -1),
            monster_frame(MonsterAi::Walk, 14.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 9.3, vec![], -1),
        ]),
        monster_move("medic_move_run", 102, 107, None, vec![
            monster_frame(MonsterAi::Run, 18.0, vec![], -1),
            monster_frame(MonsterAi::Run, 22.5, vec![], -1),
            monster_frame(MonsterAi::Run, 25.4, vec![MonsterAction::name("monster_done_dodge")], -1),
            monster_frame(MonsterAi::Run, 23.4, vec![], -1),
            monster_frame(MonsterAi::Run, 24.0, vec![], -1),
            monster_frame(MonsterAi::Run, 35.6, vec![], -1),
        ]),
        monster_move("medic_move_pain1", 108, 115, Some("medic_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("medic_move_pain2", 116, 130, Some("medic_run"), vec![
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
        ]),
        monster_move("medic_move_death", 147, 176, Some("medic_dead"), vec![
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
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("medic_move_duck", 131, 146, Some("medic_run"), vec![
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![MonsterAction::name("monster_duck_down")], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![MonsterAction::name("monster_duck_hold")], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![MonsterAction::name("monster_duck_up")], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
            monster_frame(MonsterAi::Move, -1.0, vec![], -1),
        ]),
        monster_move("medic_move_attackHyperBlaster", 191, 206, Some("medic_run"), vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
        ]),
        monster_move("medic_move_attackBlaster", 177, 190, Some("medic_run"), vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 3.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 2.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_continue")], -1),
        ]),
        monster_move("medic_move_attackCable", 209, 236, Some("medic_run"), vec![
            monster_frame(MonsterAi::Charge, 2.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 3.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -4.4, vec![], -1),
            monster_frame(MonsterAi::Charge, -4.7, vec![], -1),
            monster_frame(MonsterAi::Charge, -5.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -6.0, vec![], -1),
            monster_frame(MonsterAi::Charge, -4.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_hook_launch")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_cable_attack")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_cable_attack")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_cable_attack")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_cable_attack")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_cable_attack")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_cable_attack")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_cable_attack")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_cable_attack")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_cable_attack")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_hook_retract")], -1),
            monster_frame(MonsterAi::Move, -1.5, vec![], -1),
            monster_frame(MonsterAi::Move, -1.2, vec![], -1),
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.3, vec![], -1),
            monster_frame(MonsterAi::Move, 0.7, vec![], -1),
            monster_frame(MonsterAi::Move, 1.2, vec![], -1),
            monster_frame(MonsterAi::Move, 1.3, vec![], -1),
        ]),
        monster_move("medic_move_callReinforcements", 209, 236, Some("medic_run"), vec![
            monster_frame(MonsterAi::Charge, 2.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 3.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 4.4, vec![], -1),
            monster_frame(MonsterAi::Charge, 4.7, vec![], -1),
            monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 6.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_start_spawn")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("medic_determine_spawn")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("medic_spawngrows")], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, -15.0, vec![MonsterAction::name("medic_finish_spawn")], -1),
            monster_frame(MonsterAi::Move, -1.5, vec![], -1),
            monster_frame(MonsterAi::Move, -1.2, vec![], -1),
            monster_frame(MonsterAi::Move, -3.0, vec![], -1),
            monster_frame(MonsterAi::Move, -2.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.3, vec![], -1),
            monster_frame(MonsterAi::Move, 0.7, vec![], -1),
            monster_frame(MonsterAi::Move, 1.2, vec![], -1),
            monster_frame(MonsterAi::Move, 1.3, vec![], -1),
        ]),
    ]
}
