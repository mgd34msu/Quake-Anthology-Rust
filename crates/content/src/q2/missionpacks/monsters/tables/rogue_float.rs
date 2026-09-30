//! float move tables (`src/content/q2/missionpacks/monsters/tables/rogue-float.ts`).
//!
//! Original Quake II rogue/m_float.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterAction, MonsterMove, monster_frame, monster_move,
};

/// Frame numbers for `floatFrame`.
pub mod float_frame {
    /// Frame `actvat01`.
    pub const ACTVAT01: i32 = 0;
    /// Frame `actvat02`.
    pub const ACTVAT02: i32 = 1;
    /// Frame `actvat03`.
    pub const ACTVAT03: i32 = 2;
    /// Frame `actvat04`.
    pub const ACTVAT04: i32 = 3;
    /// Frame `actvat05`.
    pub const ACTVAT05: i32 = 4;
    /// Frame `actvat06`.
    pub const ACTVAT06: i32 = 5;
    /// Frame `actvat07`.
    pub const ACTVAT07: i32 = 6;
    /// Frame `actvat08`.
    pub const ACTVAT08: i32 = 7;
    /// Frame `actvat09`.
    pub const ACTVAT09: i32 = 8;
    /// Frame `actvat10`.
    pub const ACTVAT10: i32 = 9;
    /// Frame `actvat11`.
    pub const ACTVAT11: i32 = 10;
    /// Frame `actvat12`.
    pub const ACTVAT12: i32 = 11;
    /// Frame `actvat13`.
    pub const ACTVAT13: i32 = 12;
    /// Frame `actvat14`.
    pub const ACTVAT14: i32 = 13;
    /// Frame `actvat15`.
    pub const ACTVAT15: i32 = 14;
    /// Frame `actvat16`.
    pub const ACTVAT16: i32 = 15;
    /// Frame `actvat17`.
    pub const ACTVAT17: i32 = 16;
    /// Frame `actvat18`.
    pub const ACTVAT18: i32 = 17;
    /// Frame `actvat19`.
    pub const ACTVAT19: i32 = 18;
    /// Frame `actvat20`.
    pub const ACTVAT20: i32 = 19;
    /// Frame `actvat21`.
    pub const ACTVAT21: i32 = 20;
    /// Frame `actvat22`.
    pub const ACTVAT22: i32 = 21;
    /// Frame `actvat23`.
    pub const ACTVAT23: i32 = 22;
    /// Frame `actvat24`.
    pub const ACTVAT24: i32 = 23;
    /// Frame `actvat25`.
    pub const ACTVAT25: i32 = 24;
    /// Frame `actvat26`.
    pub const ACTVAT26: i32 = 25;
    /// Frame `actvat27`.
    pub const ACTVAT27: i32 = 26;
    /// Frame `actvat28`.
    pub const ACTVAT28: i32 = 27;
    /// Frame `actvat29`.
    pub const ACTVAT29: i32 = 28;
    /// Frame `actvat30`.
    pub const ACTVAT30: i32 = 29;
    /// Frame `actvat31`.
    pub const ACTVAT31: i32 = 30;
    /// Frame `attak101`.
    pub const ATTAK101: i32 = 31;
    /// Frame `attak102`.
    pub const ATTAK102: i32 = 32;
    /// Frame `attak103`.
    pub const ATTAK103: i32 = 33;
    /// Frame `attak104`.
    pub const ATTAK104: i32 = 34;
    /// Frame `attak105`.
    pub const ATTAK105: i32 = 35;
    /// Frame `attak106`.
    pub const ATTAK106: i32 = 36;
    /// Frame `attak107`.
    pub const ATTAK107: i32 = 37;
    /// Frame `attak108`.
    pub const ATTAK108: i32 = 38;
    /// Frame `attak109`.
    pub const ATTAK109: i32 = 39;
    /// Frame `attak110`.
    pub const ATTAK110: i32 = 40;
    /// Frame `attak111`.
    pub const ATTAK111: i32 = 41;
    /// Frame `attak112`.
    pub const ATTAK112: i32 = 42;
    /// Frame `attak113`.
    pub const ATTAK113: i32 = 43;
    /// Frame `attak114`.
    pub const ATTAK114: i32 = 44;
    /// Frame `attak201`.
    pub const ATTAK201: i32 = 45;
    /// Frame `attak202`.
    pub const ATTAK202: i32 = 46;
    /// Frame `attak203`.
    pub const ATTAK203: i32 = 47;
    /// Frame `attak204`.
    pub const ATTAK204: i32 = 48;
    /// Frame `attak205`.
    pub const ATTAK205: i32 = 49;
    /// Frame `attak206`.
    pub const ATTAK206: i32 = 50;
    /// Frame `attak207`.
    pub const ATTAK207: i32 = 51;
    /// Frame `attak208`.
    pub const ATTAK208: i32 = 52;
    /// Frame `attak209`.
    pub const ATTAK209: i32 = 53;
    /// Frame `attak210`.
    pub const ATTAK210: i32 = 54;
    /// Frame `attak211`.
    pub const ATTAK211: i32 = 55;
    /// Frame `attak212`.
    pub const ATTAK212: i32 = 56;
    /// Frame `attak213`.
    pub const ATTAK213: i32 = 57;
    /// Frame `attak214`.
    pub const ATTAK214: i32 = 58;
    /// Frame `attak215`.
    pub const ATTAK215: i32 = 59;
    /// Frame `attak216`.
    pub const ATTAK216: i32 = 60;
    /// Frame `attak217`.
    pub const ATTAK217: i32 = 61;
    /// Frame `attak218`.
    pub const ATTAK218: i32 = 62;
    /// Frame `attak219`.
    pub const ATTAK219: i32 = 63;
    /// Frame `attak220`.
    pub const ATTAK220: i32 = 64;
    /// Frame `attak221`.
    pub const ATTAK221: i32 = 65;
    /// Frame `attak222`.
    pub const ATTAK222: i32 = 66;
    /// Frame `attak223`.
    pub const ATTAK223: i32 = 67;
    /// Frame `attak224`.
    pub const ATTAK224: i32 = 68;
    /// Frame `attak225`.
    pub const ATTAK225: i32 = 69;
    /// Frame `attak301`.
    pub const ATTAK301: i32 = 70;
    /// Frame `attak302`.
    pub const ATTAK302: i32 = 71;
    /// Frame `attak303`.
    pub const ATTAK303: i32 = 72;
    /// Frame `attak304`.
    pub const ATTAK304: i32 = 73;
    /// Frame `attak305`.
    pub const ATTAK305: i32 = 74;
    /// Frame `attak306`.
    pub const ATTAK306: i32 = 75;
    /// Frame `attak307`.
    pub const ATTAK307: i32 = 76;
    /// Frame `attak308`.
    pub const ATTAK308: i32 = 77;
    /// Frame `attak309`.
    pub const ATTAK309: i32 = 78;
    /// Frame `attak310`.
    pub const ATTAK310: i32 = 79;
    /// Frame `attak311`.
    pub const ATTAK311: i32 = 80;
    /// Frame `attak312`.
    pub const ATTAK312: i32 = 81;
    /// Frame `attak313`.
    pub const ATTAK313: i32 = 82;
    /// Frame `attak314`.
    pub const ATTAK314: i32 = 83;
    /// Frame `attak315`.
    pub const ATTAK315: i32 = 84;
    /// Frame `attak316`.
    pub const ATTAK316: i32 = 85;
    /// Frame `attak317`.
    pub const ATTAK317: i32 = 86;
    /// Frame `attak318`.
    pub const ATTAK318: i32 = 87;
    /// Frame `attak319`.
    pub const ATTAK319: i32 = 88;
    /// Frame `attak320`.
    pub const ATTAK320: i32 = 89;
    /// Frame `attak321`.
    pub const ATTAK321: i32 = 90;
    /// Frame `attak322`.
    pub const ATTAK322: i32 = 91;
    /// Frame `attak323`.
    pub const ATTAK323: i32 = 92;
    /// Frame `attak324`.
    pub const ATTAK324: i32 = 93;
    /// Frame `attak325`.
    pub const ATTAK325: i32 = 94;
    /// Frame `attak326`.
    pub const ATTAK326: i32 = 95;
    /// Frame `attak327`.
    pub const ATTAK327: i32 = 96;
    /// Frame `attak328`.
    pub const ATTAK328: i32 = 97;
    /// Frame `attak329`.
    pub const ATTAK329: i32 = 98;
    /// Frame `attak330`.
    pub const ATTAK330: i32 = 99;
    /// Frame `attak331`.
    pub const ATTAK331: i32 = 100;
    /// Frame `attak332`.
    pub const ATTAK332: i32 = 101;
    /// Frame `attak333`.
    pub const ATTAK333: i32 = 102;
    /// Frame `attak334`.
    pub const ATTAK334: i32 = 103;
    /// Frame `death01`.
    pub const DEATH01: i32 = 104;
    /// Frame `death02`.
    pub const DEATH02: i32 = 105;
    /// Frame `death03`.
    pub const DEATH03: i32 = 106;
    /// Frame `death04`.
    pub const DEATH04: i32 = 107;
    /// Frame `death05`.
    pub const DEATH05: i32 = 108;
    /// Frame `death06`.
    pub const DEATH06: i32 = 109;
    /// Frame `death07`.
    pub const DEATH07: i32 = 110;
    /// Frame `death08`.
    pub const DEATH08: i32 = 111;
    /// Frame `death09`.
    pub const DEATH09: i32 = 112;
    /// Frame `death10`.
    pub const DEATH10: i32 = 113;
    /// Frame `death11`.
    pub const DEATH11: i32 = 114;
    /// Frame `death12`.
    pub const DEATH12: i32 = 115;
    /// Frame `death13`.
    pub const DEATH13: i32 = 116;
    /// Frame `pain101`.
    pub const PAIN101: i32 = 117;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 118;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 119;
    /// Frame `pain104`.
    pub const PAIN104: i32 = 120;
    /// Frame `pain105`.
    pub const PAIN105: i32 = 121;
    /// Frame `pain106`.
    pub const PAIN106: i32 = 122;
    /// Frame `pain107`.
    pub const PAIN107: i32 = 123;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 124;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 125;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 126;
    /// Frame `pain204`.
    pub const PAIN204: i32 = 127;
    /// Frame `pain205`.
    pub const PAIN205: i32 = 128;
    /// Frame `pain206`.
    pub const PAIN206: i32 = 129;
    /// Frame `pain207`.
    pub const PAIN207: i32 = 130;
    /// Frame `pain208`.
    pub const PAIN208: i32 = 131;
    /// Frame `pain301`.
    pub const PAIN301: i32 = 132;
    /// Frame `pain302`.
    pub const PAIN302: i32 = 133;
    /// Frame `pain303`.
    pub const PAIN303: i32 = 134;
    /// Frame `pain304`.
    pub const PAIN304: i32 = 135;
    /// Frame `pain305`.
    pub const PAIN305: i32 = 136;
    /// Frame `pain306`.
    pub const PAIN306: i32 = 137;
    /// Frame `pain307`.
    pub const PAIN307: i32 = 138;
    /// Frame `pain308`.
    pub const PAIN308: i32 = 139;
    /// Frame `pain309`.
    pub const PAIN309: i32 = 140;
    /// Frame `pain310`.
    pub const PAIN310: i32 = 141;
    /// Frame `pain311`.
    pub const PAIN311: i32 = 142;
    /// Frame `pain312`.
    pub const PAIN312: i32 = 143;
    /// Frame `stand101`.
    pub const STAND101: i32 = 144;
    /// Frame `stand102`.
    pub const STAND102: i32 = 145;
    /// Frame `stand103`.
    pub const STAND103: i32 = 146;
    /// Frame `stand104`.
    pub const STAND104: i32 = 147;
    /// Frame `stand105`.
    pub const STAND105: i32 = 148;
    /// Frame `stand106`.
    pub const STAND106: i32 = 149;
    /// Frame `stand107`.
    pub const STAND107: i32 = 150;
    /// Frame `stand108`.
    pub const STAND108: i32 = 151;
    /// Frame `stand109`.
    pub const STAND109: i32 = 152;
    /// Frame `stand110`.
    pub const STAND110: i32 = 153;
    /// Frame `stand111`.
    pub const STAND111: i32 = 154;
    /// Frame `stand112`.
    pub const STAND112: i32 = 155;
    /// Frame `stand113`.
    pub const STAND113: i32 = 156;
    /// Frame `stand114`.
    pub const STAND114: i32 = 157;
    /// Frame `stand115`.
    pub const STAND115: i32 = 158;
    /// Frame `stand116`.
    pub const STAND116: i32 = 159;
    /// Frame `stand117`.
    pub const STAND117: i32 = 160;
    /// Frame `stand118`.
    pub const STAND118: i32 = 161;
    /// Frame `stand119`.
    pub const STAND119: i32 = 162;
    /// Frame `stand120`.
    pub const STAND120: i32 = 163;
    /// Frame `stand121`.
    pub const STAND121: i32 = 164;
    /// Frame `stand122`.
    pub const STAND122: i32 = 165;
    /// Frame `stand123`.
    pub const STAND123: i32 = 166;
    /// Frame `stand124`.
    pub const STAND124: i32 = 167;
    /// Frame `stand125`.
    pub const STAND125: i32 = 168;
    /// Frame `stand126`.
    pub const STAND126: i32 = 169;
    /// Frame `stand127`.
    pub const STAND127: i32 = 170;
    /// Frame `stand128`.
    pub const STAND128: i32 = 171;
    /// Frame `stand129`.
    pub const STAND129: i32 = 172;
    /// Frame `stand130`.
    pub const STAND130: i32 = 173;
    /// Frame `stand131`.
    pub const STAND131: i32 = 174;
    /// Frame `stand132`.
    pub const STAND132: i32 = 175;
    /// Frame `stand133`.
    pub const STAND133: i32 = 176;
    /// Frame `stand134`.
    pub const STAND134: i32 = 177;
    /// Frame `stand135`.
    pub const STAND135: i32 = 178;
    /// Frame `stand136`.
    pub const STAND136: i32 = 179;
    /// Frame `stand137`.
    pub const STAND137: i32 = 180;
    /// Frame `stand138`.
    pub const STAND138: i32 = 181;
    /// Frame `stand139`.
    pub const STAND139: i32 = 182;
    /// Frame `stand140`.
    pub const STAND140: i32 = 183;
    /// Frame `stand141`.
    pub const STAND141: i32 = 184;
    /// Frame `stand142`.
    pub const STAND142: i32 = 185;
    /// Frame `stand143`.
    pub const STAND143: i32 = 186;
    /// Frame `stand144`.
    pub const STAND144: i32 = 187;
    /// Frame `stand145`.
    pub const STAND145: i32 = 188;
    /// Frame `stand146`.
    pub const STAND146: i32 = 189;
    /// Frame `stand147`.
    pub const STAND147: i32 = 190;
    /// Frame `stand148`.
    pub const STAND148: i32 = 191;
    /// Frame `stand149`.
    pub const STAND149: i32 = 192;
    /// Frame `stand150`.
    pub const STAND150: i32 = 193;
    /// Frame `stand151`.
    pub const STAND151: i32 = 194;
    /// Frame `stand152`.
    pub const STAND152: i32 = 195;
    /// Frame `stand201`.
    pub const STAND201: i32 = 196;
    /// Frame `stand202`.
    pub const STAND202: i32 = 197;
    /// Frame `stand203`.
    pub const STAND203: i32 = 198;
    /// Frame `stand204`.
    pub const STAND204: i32 = 199;
    /// Frame `stand205`.
    pub const STAND205: i32 = 200;
    /// Frame `stand206`.
    pub const STAND206: i32 = 201;
    /// Frame `stand207`.
    pub const STAND207: i32 = 202;
    /// Frame `stand208`.
    pub const STAND208: i32 = 203;
    /// Frame `stand209`.
    pub const STAND209: i32 = 204;
    /// Frame `stand210`.
    pub const STAND210: i32 = 205;
    /// Frame `stand211`.
    pub const STAND211: i32 = 206;
    /// Frame `stand212`.
    pub const STAND212: i32 = 207;
    /// Frame `stand213`.
    pub const STAND213: i32 = 208;
    /// Frame `stand214`.
    pub const STAND214: i32 = 209;
    /// Frame `stand215`.
    pub const STAND215: i32 = 210;
    /// Frame `stand216`.
    pub const STAND216: i32 = 211;
    /// Frame `stand217`.
    pub const STAND217: i32 = 212;
    /// Frame `stand218`.
    pub const STAND218: i32 = 213;
    /// Frame `stand219`.
    pub const STAND219: i32 = 214;
    /// Frame `stand220`.
    pub const STAND220: i32 = 215;
    /// Frame `stand221`.
    pub const STAND221: i32 = 216;
    /// Frame `stand222`.
    pub const STAND222: i32 = 217;
    /// Frame `stand223`.
    pub const STAND223: i32 = 218;
    /// Frame `stand224`.
    pub const STAND224: i32 = 219;
    /// Frame `stand225`.
    pub const STAND225: i32 = 220;
    /// Frame `stand226`.
    pub const STAND226: i32 = 221;
    /// Frame `stand227`.
    pub const STAND227: i32 = 222;
    /// Frame `stand228`.
    pub const STAND228: i32 = 223;
    /// Frame `stand229`.
    pub const STAND229: i32 = 224;
    /// Frame `stand230`.
    pub const STAND230: i32 = 225;
    /// Frame `stand231`.
    pub const STAND231: i32 = 226;
    /// Frame `stand232`.
    pub const STAND232: i32 = 227;
    /// Frame `stand233`.
    pub const STAND233: i32 = 228;
    /// Frame `stand234`.
    pub const STAND234: i32 = 229;
    /// Frame `stand235`.
    pub const STAND235: i32 = 230;
    /// Frame `stand236`.
    pub const STAND236: i32 = 231;
    /// Frame `stand237`.
    pub const STAND237: i32 = 232;
    /// Frame `stand238`.
    pub const STAND238: i32 = 233;
    /// Frame `stand239`.
    pub const STAND239: i32 = 234;
    /// Frame `stand240`.
    pub const STAND240: i32 = 235;
    /// Frame `stand241`.
    pub const STAND241: i32 = 236;
    /// Frame `stand242`.
    pub const STAND242: i32 = 237;
    /// Frame `stand243`.
    pub const STAND243: i32 = 238;
    /// Frame `stand244`.
    pub const STAND244: i32 = 239;
    /// Frame `stand245`.
    pub const STAND245: i32 = 240;
    /// Frame `stand246`.
    pub const STAND246: i32 = 241;
    /// Frame `stand247`.
    pub const STAND247: i32 = 242;
    /// Frame `stand248`.
    pub const STAND248: i32 = 243;
    /// Frame `stand249`.
    pub const STAND249: i32 = 244;
    /// Frame `stand250`.
    pub const STAND250: i32 = 245;
    /// Frame `stand251`.
    pub const STAND251: i32 = 246;
    /// Frame `stand252`.
    pub const STAND252: i32 = 247;
}

/// `floatMoves` move tables.
pub fn float_moves() -> Vec<MonsterMove> {
    vec![
        monster_move("floater_move_stand1", 144, 195, None, vec![
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
        monster_move("floater_move_stand2", 196, 247, None, vec![
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
        monster_move("floater_move_activate", 0, 30, None, vec![
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
        monster_move("floater_move_attack1", 31, 44, Some("floater_run"), vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
        ]),
        monster_move("floater_move_attack1a", 31, 44, Some("floater_run"), vec![
            monster_frame(MonsterAi::Charge, 10.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![MonsterAction::name("floater_fire_blaster")], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 10.0, vec![], -1),
        ]),
        monster_move("floater_move_attack2", 45, 69, Some("floater_run"), vec![
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
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("floater_wham")], -1),
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
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
        ]),
        monster_move("floater_move_attack3", 70, 103, Some("floater_run"), vec![
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("floater_zap")], -1),
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
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
        ]),
        monster_move("floater_move_death", 104, 116, Some("floater_dead"), vec![
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
        monster_move("floater_move_pain1", 117, 123, Some("floater_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("floater_move_pain2", 124, 131, Some("floater_run"), vec![
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            monster_frame(MonsterAi::Move, 0.0, vec![], -1),
        ]),
        monster_move("floater_move_pain3", 132, 143, Some("floater_run"), vec![
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
        monster_move("floater_move_walk", 144, 195, None, vec![
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
            monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
        ]),
        monster_move("floater_move_run", 144, 195, None, vec![
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
            monster_frame(MonsterAi::Run, 13.0, vec![], -1),
        ]),
    ]
}
