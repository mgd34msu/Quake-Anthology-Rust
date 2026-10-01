//! berserk move tables (`src/content/q2/base/monsters/tables/berserk.ts`).
//!
//! Original Quake II m_berserk.c frame order and distances. id Software, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `berserkFrame`.
pub mod berserk_frame {
    /// Frame `stand1`.
    pub const STAND1: i32 = 0;
    /// Frame `stand2`.
    pub const STAND2: i32 = 1;
    /// Frame `stand3`.
    pub const STAND3: i32 = 2;
    /// Frame `stand4`.
    pub const STAND4: i32 = 3;
    /// Frame `stand5`.
    pub const STAND5: i32 = 4;
    /// Frame `standb1`.
    pub const STANDB1: i32 = 5;
    /// Frame `standb2`.
    pub const STANDB2: i32 = 6;
    /// Frame `standb3`.
    pub const STANDB3: i32 = 7;
    /// Frame `standb4`.
    pub const STANDB4: i32 = 8;
    /// Frame `standb5`.
    pub const STANDB5: i32 = 9;
    /// Frame `standb6`.
    pub const STANDB6: i32 = 10;
    /// Frame `standb7`.
    pub const STANDB7: i32 = 11;
    /// Frame `standb8`.
    pub const STANDB8: i32 = 12;
    /// Frame `standb9`.
    pub const STANDB9: i32 = 13;
    /// Frame `standb10`.
    pub const STANDB10: i32 = 14;
    /// Frame `standb11`.
    pub const STANDB11: i32 = 15;
    /// Frame `standb12`.
    pub const STANDB12: i32 = 16;
    /// Frame `standb13`.
    pub const STANDB13: i32 = 17;
    /// Frame `standb14`.
    pub const STANDB14: i32 = 18;
    /// Frame `standb15`.
    pub const STANDB15: i32 = 19;
    /// Frame `standb16`.
    pub const STANDB16: i32 = 20;
    /// Frame `standb17`.
    pub const STANDB17: i32 = 21;
    /// Frame `standb18`.
    pub const STANDB18: i32 = 22;
    /// Frame `standb19`.
    pub const STANDB19: i32 = 23;
    /// Frame `standb20`.
    pub const STANDB20: i32 = 24;
    /// Frame `walkc1`.
    pub const WALKC1: i32 = 25;
    /// Frame `walkc2`.
    pub const WALKC2: i32 = 26;
    /// Frame `walkc3`.
    pub const WALKC3: i32 = 27;
    /// Frame `walkc4`.
    pub const WALKC4: i32 = 28;
    /// Frame `walkc5`.
    pub const WALKC5: i32 = 29;
    /// Frame `walkc6`.
    pub const WALKC6: i32 = 30;
    /// Frame `walkc7`.
    pub const WALKC7: i32 = 31;
    /// Frame `walkc8`.
    pub const WALKC8: i32 = 32;
    /// Frame `walkc9`.
    pub const WALKC9: i32 = 33;
    /// Frame `walkc10`.
    pub const WALKC10: i32 = 34;
    /// Frame `walkc11`.
    pub const WALKC11: i32 = 35;
    /// Frame `run1`.
    pub const RUN1: i32 = 36;
    /// Frame `run2`.
    pub const RUN2: i32 = 37;
    /// Frame `run3`.
    pub const RUN3: i32 = 38;
    /// Frame `run4`.
    pub const RUN4: i32 = 39;
    /// Frame `run5`.
    pub const RUN5: i32 = 40;
    /// Frame `run6`.
    pub const RUN6: i32 = 41;
    /// Frame `att_a1`.
    pub const ATT_A1: i32 = 42;
    /// Frame `att_a2`.
    pub const ATT_A2: i32 = 43;
    /// Frame `att_a3`.
    pub const ATT_A3: i32 = 44;
    /// Frame `att_a4`.
    pub const ATT_A4: i32 = 45;
    /// Frame `att_a5`.
    pub const ATT_A5: i32 = 46;
    /// Frame `att_a6`.
    pub const ATT_A6: i32 = 47;
    /// Frame `att_a7`.
    pub const ATT_A7: i32 = 48;
    /// Frame `att_a8`.
    pub const ATT_A8: i32 = 49;
    /// Frame `att_a9`.
    pub const ATT_A9: i32 = 50;
    /// Frame `att_a10`.
    pub const ATT_A10: i32 = 51;
    /// Frame `att_a11`.
    pub const ATT_A11: i32 = 52;
    /// Frame `att_a12`.
    pub const ATT_A12: i32 = 53;
    /// Frame `att_a13`.
    pub const ATT_A13: i32 = 54;
    /// Frame `att_b1`.
    pub const ATT_B1: i32 = 55;
    /// Frame `att_b2`.
    pub const ATT_B2: i32 = 56;
    /// Frame `att_b3`.
    pub const ATT_B3: i32 = 57;
    /// Frame `att_b4`.
    pub const ATT_B4: i32 = 58;
    /// Frame `att_b5`.
    pub const ATT_B5: i32 = 59;
    /// Frame `att_b6`.
    pub const ATT_B6: i32 = 60;
    /// Frame `att_b7`.
    pub const ATT_B7: i32 = 61;
    /// Frame `att_b8`.
    pub const ATT_B8: i32 = 62;
    /// Frame `att_b9`.
    pub const ATT_B9: i32 = 63;
    /// Frame `att_b10`.
    pub const ATT_B10: i32 = 64;
    /// Frame `att_b11`.
    pub const ATT_B11: i32 = 65;
    /// Frame `att_b12`.
    pub const ATT_B12: i32 = 66;
    /// Frame `att_b13`.
    pub const ATT_B13: i32 = 67;
    /// Frame `att_b14`.
    pub const ATT_B14: i32 = 68;
    /// Frame `att_b15`.
    pub const ATT_B15: i32 = 69;
    /// Frame `att_b16`.
    pub const ATT_B16: i32 = 70;
    /// Frame `att_b17`.
    pub const ATT_B17: i32 = 71;
    /// Frame `att_b18`.
    pub const ATT_B18: i32 = 72;
    /// Frame `att_b19`.
    pub const ATT_B19: i32 = 73;
    /// Frame `att_b20`.
    pub const ATT_B20: i32 = 74;
    /// Frame `att_b21`.
    pub const ATT_B21: i32 = 75;
    /// Frame `att_c1`.
    pub const ATT_C1: i32 = 76;
    /// Frame `att_c2`.
    pub const ATT_C2: i32 = 77;
    /// Frame `att_c3`.
    pub const ATT_C3: i32 = 78;
    /// Frame `att_c4`.
    pub const ATT_C4: i32 = 79;
    /// Frame `att_c5`.
    pub const ATT_C5: i32 = 80;
    /// Frame `att_c6`.
    pub const ATT_C6: i32 = 81;
    /// Frame `att_c7`.
    pub const ATT_C7: i32 = 82;
    /// Frame `att_c8`.
    pub const ATT_C8: i32 = 83;
    /// Frame `att_c9`.
    pub const ATT_C9: i32 = 84;
    /// Frame `att_c10`.
    pub const ATT_C10: i32 = 85;
    /// Frame `att_c11`.
    pub const ATT_C11: i32 = 86;
    /// Frame `att_c12`.
    pub const ATT_C12: i32 = 87;
    /// Frame `att_c13`.
    pub const ATT_C13: i32 = 88;
    /// Frame `att_c14`.
    pub const ATT_C14: i32 = 89;
    /// Frame `att_c15`.
    pub const ATT_C15: i32 = 90;
    /// Frame `att_c16`.
    pub const ATT_C16: i32 = 91;
    /// Frame `att_c17`.
    pub const ATT_C17: i32 = 92;
    /// Frame `att_c18`.
    pub const ATT_C18: i32 = 93;
    /// Frame `att_c19`.
    pub const ATT_C19: i32 = 94;
    /// Frame `att_c20`.
    pub const ATT_C20: i32 = 95;
    /// Frame `att_c21`.
    pub const ATT_C21: i32 = 96;
    /// Frame `att_c22`.
    pub const ATT_C22: i32 = 97;
    /// Frame `att_c23`.
    pub const ATT_C23: i32 = 98;
    /// Frame `att_c24`.
    pub const ATT_C24: i32 = 99;
    /// Frame `att_c25`.
    pub const ATT_C25: i32 = 100;
    /// Frame `att_c26`.
    pub const ATT_C26: i32 = 101;
    /// Frame `att_c27`.
    pub const ATT_C27: i32 = 102;
    /// Frame `att_c28`.
    pub const ATT_C28: i32 = 103;
    /// Frame `att_c29`.
    pub const ATT_C29: i32 = 104;
    /// Frame `att_c30`.
    pub const ATT_C30: i32 = 105;
    /// Frame `att_c31`.
    pub const ATT_C31: i32 = 106;
    /// Frame `att_c32`.
    pub const ATT_C32: i32 = 107;
    /// Frame `att_c33`.
    pub const ATT_C33: i32 = 108;
    /// Frame `att_c34`.
    pub const ATT_C34: i32 = 109;
    /// Frame `r_att1`.
    pub const R_ATT1: i32 = 110;
    /// Frame `r_att2`.
    pub const R_ATT2: i32 = 111;
    /// Frame `r_att3`.
    pub const R_ATT3: i32 = 112;
    /// Frame `r_att4`.
    pub const R_ATT4: i32 = 113;
    /// Frame `r_att5`.
    pub const R_ATT5: i32 = 114;
    /// Frame `r_att6`.
    pub const R_ATT6: i32 = 115;
    /// Frame `r_att7`.
    pub const R_ATT7: i32 = 116;
    /// Frame `r_att8`.
    pub const R_ATT8: i32 = 117;
    /// Frame `r_att9`.
    pub const R_ATT9: i32 = 118;
    /// Frame `r_att10`.
    pub const R_ATT10: i32 = 119;
    /// Frame `r_att11`.
    pub const R_ATT11: i32 = 120;
    /// Frame `r_att12`.
    pub const R_ATT12: i32 = 121;
    /// Frame `r_att13`.
    pub const R_ATT13: i32 = 122;
    /// Frame `r_att14`.
    pub const R_ATT14: i32 = 123;
    /// Frame `r_att15`.
    pub const R_ATT15: i32 = 124;
    /// Frame `r_att16`.
    pub const R_ATT16: i32 = 125;
    /// Frame `r_att17`.
    pub const R_ATT17: i32 = 126;
    /// Frame `r_att18`.
    pub const R_ATT18: i32 = 127;
    /// Frame `r_attb1`.
    pub const R_ATTB1: i32 = 128;
    /// Frame `r_attb2`.
    pub const R_ATTB2: i32 = 129;
    /// Frame `r_attb3`.
    pub const R_ATTB3: i32 = 130;
    /// Frame `r_attb4`.
    pub const R_ATTB4: i32 = 131;
    /// Frame `r_attb5`.
    pub const R_ATTB5: i32 = 132;
    /// Frame `r_attb6`.
    pub const R_ATTB6: i32 = 133;
    /// Frame `r_attb7`.
    pub const R_ATTB7: i32 = 134;
    /// Frame `r_attb8`.
    pub const R_ATTB8: i32 = 135;
    /// Frame `r_attb9`.
    pub const R_ATTB9: i32 = 136;
    /// Frame `r_attb10`.
    pub const R_ATTB10: i32 = 137;
    /// Frame `r_attb11`.
    pub const R_ATTB11: i32 = 138;
    /// Frame `r_attb12`.
    pub const R_ATTB12: i32 = 139;
    /// Frame `r_attb13`.
    pub const R_ATTB13: i32 = 140;
    /// Frame `r_attb14`.
    pub const R_ATTB14: i32 = 141;
    /// Frame `r_attb15`.
    pub const R_ATTB15: i32 = 142;
    /// Frame `r_attb16`.
    pub const R_ATTB16: i32 = 143;
    /// Frame `r_attb17`.
    pub const R_ATTB17: i32 = 144;
    /// Frame `r_attb18`.
    pub const R_ATTB18: i32 = 145;
    /// Frame `slam1`.
    pub const SLAM1: i32 = 146;
    /// Frame `slam2`.
    pub const SLAM2: i32 = 147;
    /// Frame `slam3`.
    pub const SLAM3: i32 = 148;
    /// Frame `slam4`.
    pub const SLAM4: i32 = 149;
    /// Frame `slam5`.
    pub const SLAM5: i32 = 150;
    /// Frame `slam6`.
    pub const SLAM6: i32 = 151;
    /// Frame `slam7`.
    pub const SLAM7: i32 = 152;
    /// Frame `slam8`.
    pub const SLAM8: i32 = 153;
    /// Frame `slam9`.
    pub const SLAM9: i32 = 154;
    /// Frame `slam10`.
    pub const SLAM10: i32 = 155;
    /// Frame `slam11`.
    pub const SLAM11: i32 = 156;
    /// Frame `slam12`.
    pub const SLAM12: i32 = 157;
    /// Frame `slam13`.
    pub const SLAM13: i32 = 158;
    /// Frame `slam14`.
    pub const SLAM14: i32 = 159;
    /// Frame `slam15`.
    pub const SLAM15: i32 = 160;
    /// Frame `slam16`.
    pub const SLAM16: i32 = 161;
    /// Frame `slam17`.
    pub const SLAM17: i32 = 162;
    /// Frame `slam18`.
    pub const SLAM18: i32 = 163;
    /// Frame `slam19`.
    pub const SLAM19: i32 = 164;
    /// Frame `slam20`.
    pub const SLAM20: i32 = 165;
    /// Frame `slam21`.
    pub const SLAM21: i32 = 166;
    /// Frame `slam22`.
    pub const SLAM22: i32 = 167;
    /// Frame `slam23`.
    pub const SLAM23: i32 = 168;
    /// Frame `duck1`.
    pub const DUCK1: i32 = 169;
    /// Frame `duck2`.
    pub const DUCK2: i32 = 170;
    /// Frame `duck3`.
    pub const DUCK3: i32 = 171;
    /// Frame `duck4`.
    pub const DUCK4: i32 = 172;
    /// Frame `duck5`.
    pub const DUCK5: i32 = 173;
    /// Frame `duck6`.
    pub const DUCK6: i32 = 174;
    /// Frame `duck7`.
    pub const DUCK7: i32 = 175;
    /// Frame `duck8`.
    pub const DUCK8: i32 = 176;
    /// Frame `duck9`.
    pub const DUCK9: i32 = 177;
    /// Frame `duck10`.
    pub const DUCK10: i32 = 178;
    /// Frame `fall1`.
    pub const FALL1: i32 = 179;
    /// Frame `fall2`.
    pub const FALL2: i32 = 180;
    /// Frame `fall3`.
    pub const FALL3: i32 = 181;
    /// Frame `fall4`.
    pub const FALL4: i32 = 182;
    /// Frame `fall5`.
    pub const FALL5: i32 = 183;
    /// Frame `fall6`.
    pub const FALL6: i32 = 184;
    /// Frame `fall7`.
    pub const FALL7: i32 = 185;
    /// Frame `fall8`.
    pub const FALL8: i32 = 186;
    /// Frame `fall9`.
    pub const FALL9: i32 = 187;
    /// Frame `fall10`.
    pub const FALL10: i32 = 188;
    /// Frame `fall11`.
    pub const FALL11: i32 = 189;
    /// Frame `fall12`.
    pub const FALL12: i32 = 190;
    /// Frame `fall13`.
    pub const FALL13: i32 = 191;
    /// Frame `fall14`.
    pub const FALL14: i32 = 192;
    /// Frame `fall15`.
    pub const FALL15: i32 = 193;
    /// Frame `fall16`.
    pub const FALL16: i32 = 194;
    /// Frame `fall17`.
    pub const FALL17: i32 = 195;
    /// Frame `fall18`.
    pub const FALL18: i32 = 196;
    /// Frame `fall19`.
    pub const FALL19: i32 = 197;
    /// Frame `fall20`.
    pub const FALL20: i32 = 198;
    /// Frame `painc1`.
    pub const PAINC1: i32 = 199;
    /// Frame `painc2`.
    pub const PAINC2: i32 = 200;
    /// Frame `painc3`.
    pub const PAINC3: i32 = 201;
    /// Frame `painc4`.
    pub const PAINC4: i32 = 202;
    /// Frame `painb1`.
    pub const PAINB1: i32 = 203;
    /// Frame `painb2`.
    pub const PAINB2: i32 = 204;
    /// Frame `painb3`.
    pub const PAINB3: i32 = 205;
    /// Frame `painb4`.
    pub const PAINB4: i32 = 206;
    /// Frame `painb5`.
    pub const PAINB5: i32 = 207;
    /// Frame `painb6`.
    pub const PAINB6: i32 = 208;
    /// Frame `painb7`.
    pub const PAINB7: i32 = 209;
    /// Frame `painb8`.
    pub const PAINB8: i32 = 210;
    /// Frame `painb9`.
    pub const PAINB9: i32 = 211;
    /// Frame `painb10`.
    pub const PAINB10: i32 = 212;
    /// Frame `painb11`.
    pub const PAINB11: i32 = 213;
    /// Frame `painb12`.
    pub const PAINB12: i32 = 214;
    /// Frame `painb13`.
    pub const PAINB13: i32 = 215;
    /// Frame `painb14`.
    pub const PAINB14: i32 = 216;
    /// Frame `painb15`.
    pub const PAINB15: i32 = 217;
    /// Frame `painb16`.
    pub const PAINB16: i32 = 218;
    /// Frame `painb17`.
    pub const PAINB17: i32 = 219;
    /// Frame `painb18`.
    pub const PAINB18: i32 = 220;
    /// Frame `painb19`.
    pub const PAINB19: i32 = 221;
    /// Frame `painb20`.
    pub const PAINB20: i32 = 222;
    /// Frame `death1`.
    pub const DEATH1: i32 = 223;
    /// Frame `death2`.
    pub const DEATH2: i32 = 224;
    /// Frame `death3`.
    pub const DEATH3: i32 = 225;
    /// Frame `death4`.
    pub const DEATH4: i32 = 226;
    /// Frame `death5`.
    pub const DEATH5: i32 = 227;
    /// Frame `death6`.
    pub const DEATH6: i32 = 228;
    /// Frame `death7`.
    pub const DEATH7: i32 = 229;
    /// Frame `death8`.
    pub const DEATH8: i32 = 230;
    /// Frame `death9`.
    pub const DEATH9: i32 = 231;
    /// Frame `death10`.
    pub const DEATH10: i32 = 232;
    /// Frame `death11`.
    pub const DEATH11: i32 = 233;
    /// Frame `death12`.
    pub const DEATH12: i32 = 234;
    /// Frame `death13`.
    pub const DEATH13: i32 = 235;
    /// Frame `deathc1`.
    pub const DEATHC1: i32 = 236;
    /// Frame `deathc2`.
    pub const DEATHC2: i32 = 237;
    /// Frame `deathc3`.
    pub const DEATHC3: i32 = 238;
    /// Frame `deathc4`.
    pub const DEATHC4: i32 = 239;
    /// Frame `deathc5`.
    pub const DEATHC5: i32 = 240;
    /// Frame `deathc6`.
    pub const DEATHC6: i32 = 241;
    /// Frame `deathc7`.
    pub const DEATHC7: i32 = 242;
    /// Frame `deathc8`.
    pub const DEATHC8: i32 = 243;
}

/// `berserkMoves` move tables.
pub fn berserk_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "berserk_move_stand",
            0,
            4,
            None,
            vec![
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("berserk_fidget")], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "berserk_move_stand_fidget",
            5,
            24,
            Some("berserk_stand"),
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
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "berserk_move_walk",
            25,
            35,
            None,
            vec![
                monster_frame(MonsterAi::Walk, 9.1, vec![], -1),
                monster_frame(MonsterAi::Walk, 6.3, vec![], -1),
                monster_frame(MonsterAi::Walk, 4.9, vec![], -1),
                monster_frame(MonsterAi::Walk, 6.7, vec![], -1),
                monster_frame(MonsterAi::Walk, 6.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 8.2, vec![], -1),
                monster_frame(MonsterAi::Walk, 7.2, vec![], -1),
                monster_frame(MonsterAi::Walk, 6.1, vec![], -1),
                monster_frame(MonsterAi::Walk, 4.9, vec![], -1),
                monster_frame(MonsterAi::Walk, 4.7, vec![], -1),
                monster_frame(MonsterAi::Walk, 4.7, vec![], -1),
                monster_frame(MonsterAi::Walk, 4.8, vec![], -1),
            ],
        ),
        monster_move(
            "berserk_move_run1",
            36,
            41,
            None,
            vec![
                monster_frame(MonsterAi::Run, 21.0, vec![], -1),
                monster_frame(MonsterAi::Run, 11.0, vec![], -1),
                monster_frame(MonsterAi::Run, 21.0, vec![], -1),
                monster_frame(MonsterAi::Run, 25.0, vec![], -1),
                monster_frame(MonsterAi::Run, 18.0, vec![], -1),
                monster_frame(MonsterAi::Run, 19.0, vec![], -1),
            ],
        ),
        monster_move(
            "berserk_move_attack_spike",
            76,
            83,
            Some("berserk_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("berserk_swing")], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("berserk_attack_spike")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "berserk_move_attack_club",
            84,
            95,
            Some("berserk_run"),
            vec![
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("berserk_swing")], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("berserk_attack_club")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "berserk_move_attack_strike",
            96,
            109,
            Some("berserk_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("berserk_swing")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("berserk_strike")], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 9.7, vec![], -1),
                monster_frame(MonsterAi::Move, 13.6, vec![], -1),
            ],
        ),
        monster_move(
            "berserk_move_pain1",
            199,
            202,
            Some("berserk_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "berserk_move_pain2",
            203,
            222,
            Some("berserk_run"),
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
            ],
        ),
        monster_move(
            "berserk_move_death1",
            223,
            235,
            Some("berserk_dead"),
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
            ],
        ),
        monster_move(
            "berserk_move_death2",
            236,
            243,
            Some("berserk_dead"),
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
    ]
}
