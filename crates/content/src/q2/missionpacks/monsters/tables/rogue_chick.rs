//! chick move tables (`src/content/q2/missionpacks/monsters/tables/rogue-chick.ts`).
//!
//! Original Quake II rogue/m_chick.c frame order and distances. ZeniMax Media, GPL-2.0-or-later.

use crate::q2::foundation::monsters::types::{monster_frame, monster_move, MonsterAction, MonsterAi, MonsterMove};

/// Frame numbers for `chickFrame`.
pub mod chick_frame {
    /// Frame `attak101`.
    pub const ATTAK101: i32 = 0;
    /// Frame `attak102`.
    pub const ATTAK102: i32 = 1;
    /// Frame `attak103`.
    pub const ATTAK103: i32 = 2;
    /// Frame `attak104`.
    pub const ATTAK104: i32 = 3;
    /// Frame `attak105`.
    pub const ATTAK105: i32 = 4;
    /// Frame `attak106`.
    pub const ATTAK106: i32 = 5;
    /// Frame `attak107`.
    pub const ATTAK107: i32 = 6;
    /// Frame `attak108`.
    pub const ATTAK108: i32 = 7;
    /// Frame `attak109`.
    pub const ATTAK109: i32 = 8;
    /// Frame `attak110`.
    pub const ATTAK110: i32 = 9;
    /// Frame `attak111`.
    pub const ATTAK111: i32 = 10;
    /// Frame `attak112`.
    pub const ATTAK112: i32 = 11;
    /// Frame `attak113`.
    pub const ATTAK113: i32 = 12;
    /// Frame `attak114`.
    pub const ATTAK114: i32 = 13;
    /// Frame `attak115`.
    pub const ATTAK115: i32 = 14;
    /// Frame `attak116`.
    pub const ATTAK116: i32 = 15;
    /// Frame `attak117`.
    pub const ATTAK117: i32 = 16;
    /// Frame `attak118`.
    pub const ATTAK118: i32 = 17;
    /// Frame `attak119`.
    pub const ATTAK119: i32 = 18;
    /// Frame `attak120`.
    pub const ATTAK120: i32 = 19;
    /// Frame `attak121`.
    pub const ATTAK121: i32 = 20;
    /// Frame `attak122`.
    pub const ATTAK122: i32 = 21;
    /// Frame `attak123`.
    pub const ATTAK123: i32 = 22;
    /// Frame `attak124`.
    pub const ATTAK124: i32 = 23;
    /// Frame `attak125`.
    pub const ATTAK125: i32 = 24;
    /// Frame `attak126`.
    pub const ATTAK126: i32 = 25;
    /// Frame `attak127`.
    pub const ATTAK127: i32 = 26;
    /// Frame `attak128`.
    pub const ATTAK128: i32 = 27;
    /// Frame `attak129`.
    pub const ATTAK129: i32 = 28;
    /// Frame `attak130`.
    pub const ATTAK130: i32 = 29;
    /// Frame `attak131`.
    pub const ATTAK131: i32 = 30;
    /// Frame `attak132`.
    pub const ATTAK132: i32 = 31;
    /// Frame `attak201`.
    pub const ATTAK201: i32 = 32;
    /// Frame `attak202`.
    pub const ATTAK202: i32 = 33;
    /// Frame `attak203`.
    pub const ATTAK203: i32 = 34;
    /// Frame `attak204`.
    pub const ATTAK204: i32 = 35;
    /// Frame `attak205`.
    pub const ATTAK205: i32 = 36;
    /// Frame `attak206`.
    pub const ATTAK206: i32 = 37;
    /// Frame `attak207`.
    pub const ATTAK207: i32 = 38;
    /// Frame `attak208`.
    pub const ATTAK208: i32 = 39;
    /// Frame `attak209`.
    pub const ATTAK209: i32 = 40;
    /// Frame `attak210`.
    pub const ATTAK210: i32 = 41;
    /// Frame `attak211`.
    pub const ATTAK211: i32 = 42;
    /// Frame `attak212`.
    pub const ATTAK212: i32 = 43;
    /// Frame `attak213`.
    pub const ATTAK213: i32 = 44;
    /// Frame `attak214`.
    pub const ATTAK214: i32 = 45;
    /// Frame `attak215`.
    pub const ATTAK215: i32 = 46;
    /// Frame `attak216`.
    pub const ATTAK216: i32 = 47;
    /// Frame `death101`.
    pub const DEATH101: i32 = 48;
    /// Frame `death102`.
    pub const DEATH102: i32 = 49;
    /// Frame `death103`.
    pub const DEATH103: i32 = 50;
    /// Frame `death104`.
    pub const DEATH104: i32 = 51;
    /// Frame `death105`.
    pub const DEATH105: i32 = 52;
    /// Frame `death106`.
    pub const DEATH106: i32 = 53;
    /// Frame `death107`.
    pub const DEATH107: i32 = 54;
    /// Frame `death108`.
    pub const DEATH108: i32 = 55;
    /// Frame `death109`.
    pub const DEATH109: i32 = 56;
    /// Frame `death110`.
    pub const DEATH110: i32 = 57;
    /// Frame `death111`.
    pub const DEATH111: i32 = 58;
    /// Frame `death112`.
    pub const DEATH112: i32 = 59;
    /// Frame `death201`.
    pub const DEATH201: i32 = 60;
    /// Frame `death202`.
    pub const DEATH202: i32 = 61;
    /// Frame `death203`.
    pub const DEATH203: i32 = 62;
    /// Frame `death204`.
    pub const DEATH204: i32 = 63;
    /// Frame `death205`.
    pub const DEATH205: i32 = 64;
    /// Frame `death206`.
    pub const DEATH206: i32 = 65;
    /// Frame `death207`.
    pub const DEATH207: i32 = 66;
    /// Frame `death208`.
    pub const DEATH208: i32 = 67;
    /// Frame `death209`.
    pub const DEATH209: i32 = 68;
    /// Frame `death210`.
    pub const DEATH210: i32 = 69;
    /// Frame `death211`.
    pub const DEATH211: i32 = 70;
    /// Frame `death212`.
    pub const DEATH212: i32 = 71;
    /// Frame `death213`.
    pub const DEATH213: i32 = 72;
    /// Frame `death214`.
    pub const DEATH214: i32 = 73;
    /// Frame `death215`.
    pub const DEATH215: i32 = 74;
    /// Frame `death216`.
    pub const DEATH216: i32 = 75;
    /// Frame `death217`.
    pub const DEATH217: i32 = 76;
    /// Frame `death218`.
    pub const DEATH218: i32 = 77;
    /// Frame `death219`.
    pub const DEATH219: i32 = 78;
    /// Frame `death220`.
    pub const DEATH220: i32 = 79;
    /// Frame `death221`.
    pub const DEATH221: i32 = 80;
    /// Frame `death222`.
    pub const DEATH222: i32 = 81;
    /// Frame `death223`.
    pub const DEATH223: i32 = 82;
    /// Frame `duck01`.
    pub const DUCK01: i32 = 83;
    /// Frame `duck02`.
    pub const DUCK02: i32 = 84;
    /// Frame `duck03`.
    pub const DUCK03: i32 = 85;
    /// Frame `duck04`.
    pub const DUCK04: i32 = 86;
    /// Frame `duck05`.
    pub const DUCK05: i32 = 87;
    /// Frame `duck06`.
    pub const DUCK06: i32 = 88;
    /// Frame `duck07`.
    pub const DUCK07: i32 = 89;
    /// Frame `pain101`.
    pub const PAIN101: i32 = 90;
    /// Frame `pain102`.
    pub const PAIN102: i32 = 91;
    /// Frame `pain103`.
    pub const PAIN103: i32 = 92;
    /// Frame `pain104`.
    pub const PAIN104: i32 = 93;
    /// Frame `pain105`.
    pub const PAIN105: i32 = 94;
    /// Frame `pain201`.
    pub const PAIN201: i32 = 95;
    /// Frame `pain202`.
    pub const PAIN202: i32 = 96;
    /// Frame `pain203`.
    pub const PAIN203: i32 = 97;
    /// Frame `pain204`.
    pub const PAIN204: i32 = 98;
    /// Frame `pain205`.
    pub const PAIN205: i32 = 99;
    /// Frame `pain301`.
    pub const PAIN301: i32 = 100;
    /// Frame `pain302`.
    pub const PAIN302: i32 = 101;
    /// Frame `pain303`.
    pub const PAIN303: i32 = 102;
    /// Frame `pain304`.
    pub const PAIN304: i32 = 103;
    /// Frame `pain305`.
    pub const PAIN305: i32 = 104;
    /// Frame `pain306`.
    pub const PAIN306: i32 = 105;
    /// Frame `pain307`.
    pub const PAIN307: i32 = 106;
    /// Frame `pain308`.
    pub const PAIN308: i32 = 107;
    /// Frame `pain309`.
    pub const PAIN309: i32 = 108;
    /// Frame `pain310`.
    pub const PAIN310: i32 = 109;
    /// Frame `pain311`.
    pub const PAIN311: i32 = 110;
    /// Frame `pain312`.
    pub const PAIN312: i32 = 111;
    /// Frame `pain313`.
    pub const PAIN313: i32 = 112;
    /// Frame `pain314`.
    pub const PAIN314: i32 = 113;
    /// Frame `pain315`.
    pub const PAIN315: i32 = 114;
    /// Frame `pain316`.
    pub const PAIN316: i32 = 115;
    /// Frame `pain317`.
    pub const PAIN317: i32 = 116;
    /// Frame `pain318`.
    pub const PAIN318: i32 = 117;
    /// Frame `pain319`.
    pub const PAIN319: i32 = 118;
    /// Frame `pain320`.
    pub const PAIN320: i32 = 119;
    /// Frame `pain321`.
    pub const PAIN321: i32 = 120;
    /// Frame `stand101`.
    pub const STAND101: i32 = 121;
    /// Frame `stand102`.
    pub const STAND102: i32 = 122;
    /// Frame `stand103`.
    pub const STAND103: i32 = 123;
    /// Frame `stand104`.
    pub const STAND104: i32 = 124;
    /// Frame `stand105`.
    pub const STAND105: i32 = 125;
    /// Frame `stand106`.
    pub const STAND106: i32 = 126;
    /// Frame `stand107`.
    pub const STAND107: i32 = 127;
    /// Frame `stand108`.
    pub const STAND108: i32 = 128;
    /// Frame `stand109`.
    pub const STAND109: i32 = 129;
    /// Frame `stand110`.
    pub const STAND110: i32 = 130;
    /// Frame `stand111`.
    pub const STAND111: i32 = 131;
    /// Frame `stand112`.
    pub const STAND112: i32 = 132;
    /// Frame `stand113`.
    pub const STAND113: i32 = 133;
    /// Frame `stand114`.
    pub const STAND114: i32 = 134;
    /// Frame `stand115`.
    pub const STAND115: i32 = 135;
    /// Frame `stand116`.
    pub const STAND116: i32 = 136;
    /// Frame `stand117`.
    pub const STAND117: i32 = 137;
    /// Frame `stand118`.
    pub const STAND118: i32 = 138;
    /// Frame `stand119`.
    pub const STAND119: i32 = 139;
    /// Frame `stand120`.
    pub const STAND120: i32 = 140;
    /// Frame `stand121`.
    pub const STAND121: i32 = 141;
    /// Frame `stand122`.
    pub const STAND122: i32 = 142;
    /// Frame `stand123`.
    pub const STAND123: i32 = 143;
    /// Frame `stand124`.
    pub const STAND124: i32 = 144;
    /// Frame `stand125`.
    pub const STAND125: i32 = 145;
    /// Frame `stand126`.
    pub const STAND126: i32 = 146;
    /// Frame `stand127`.
    pub const STAND127: i32 = 147;
    /// Frame `stand128`.
    pub const STAND128: i32 = 148;
    /// Frame `stand129`.
    pub const STAND129: i32 = 149;
    /// Frame `stand130`.
    pub const STAND130: i32 = 150;
    /// Frame `stand201`.
    pub const STAND201: i32 = 151;
    /// Frame `stand202`.
    pub const STAND202: i32 = 152;
    /// Frame `stand203`.
    pub const STAND203: i32 = 153;
    /// Frame `stand204`.
    pub const STAND204: i32 = 154;
    /// Frame `stand205`.
    pub const STAND205: i32 = 155;
    /// Frame `stand206`.
    pub const STAND206: i32 = 156;
    /// Frame `stand207`.
    pub const STAND207: i32 = 157;
    /// Frame `stand208`.
    pub const STAND208: i32 = 158;
    /// Frame `stand209`.
    pub const STAND209: i32 = 159;
    /// Frame `stand210`.
    pub const STAND210: i32 = 160;
    /// Frame `stand211`.
    pub const STAND211: i32 = 161;
    /// Frame `stand212`.
    pub const STAND212: i32 = 162;
    /// Frame `stand213`.
    pub const STAND213: i32 = 163;
    /// Frame `stand214`.
    pub const STAND214: i32 = 164;
    /// Frame `stand215`.
    pub const STAND215: i32 = 165;
    /// Frame `stand216`.
    pub const STAND216: i32 = 166;
    /// Frame `stand217`.
    pub const STAND217: i32 = 167;
    /// Frame `stand218`.
    pub const STAND218: i32 = 168;
    /// Frame `stand219`.
    pub const STAND219: i32 = 169;
    /// Frame `stand220`.
    pub const STAND220: i32 = 170;
    /// Frame `stand221`.
    pub const STAND221: i32 = 171;
    /// Frame `stand222`.
    pub const STAND222: i32 = 172;
    /// Frame `stand223`.
    pub const STAND223: i32 = 173;
    /// Frame `stand224`.
    pub const STAND224: i32 = 174;
    /// Frame `stand225`.
    pub const STAND225: i32 = 175;
    /// Frame `stand226`.
    pub const STAND226: i32 = 176;
    /// Frame `stand227`.
    pub const STAND227: i32 = 177;
    /// Frame `stand228`.
    pub const STAND228: i32 = 178;
    /// Frame `stand229`.
    pub const STAND229: i32 = 179;
    /// Frame `stand230`.
    pub const STAND230: i32 = 180;
    /// Frame `walk01`.
    pub const WALK01: i32 = 181;
    /// Frame `walk02`.
    pub const WALK02: i32 = 182;
    /// Frame `walk03`.
    pub const WALK03: i32 = 183;
    /// Frame `walk04`.
    pub const WALK04: i32 = 184;
    /// Frame `walk05`.
    pub const WALK05: i32 = 185;
    /// Frame `walk06`.
    pub const WALK06: i32 = 186;
    /// Frame `walk07`.
    pub const WALK07: i32 = 187;
    /// Frame `walk08`.
    pub const WALK08: i32 = 188;
    /// Frame `walk09`.
    pub const WALK09: i32 = 189;
    /// Frame `walk10`.
    pub const WALK10: i32 = 190;
    /// Frame `walk11`.
    pub const WALK11: i32 = 191;
    /// Frame `walk12`.
    pub const WALK12: i32 = 192;
    /// Frame `walk13`.
    pub const WALK13: i32 = 193;
    /// Frame `walk14`.
    pub const WALK14: i32 = 194;
    /// Frame `walk15`.
    pub const WALK15: i32 = 195;
    /// Frame `walk16`.
    pub const WALK16: i32 = 196;
    /// Frame `walk17`.
    pub const WALK17: i32 = 197;
    /// Frame `walk18`.
    pub const WALK18: i32 = 198;
    /// Frame `walk19`.
    pub const WALK19: i32 = 199;
    /// Frame `walk20`.
    pub const WALK20: i32 = 200;
    /// Frame `walk21`.
    pub const WALK21: i32 = 201;
    /// Frame `walk22`.
    pub const WALK22: i32 = 202;
    /// Frame `walk23`.
    pub const WALK23: i32 = 203;
    /// Frame `walk24`.
    pub const WALK24: i32 = 204;
    /// Frame `walk25`.
    pub const WALK25: i32 = 205;
    /// Frame `walk26`.
    pub const WALK26: i32 = 206;
    /// Frame `walk27`.
    pub const WALK27: i32 = 207;
    /// Frame `recln201`.
    pub const RECLN201: i32 = 208;
    /// Frame `recln202`.
    pub const RECLN202: i32 = 209;
    /// Frame `recln203`.
    pub const RECLN203: i32 = 210;
    /// Frame `recln204`.
    pub const RECLN204: i32 = 211;
    /// Frame `recln205`.
    pub const RECLN205: i32 = 212;
    /// Frame `recln206`.
    pub const RECLN206: i32 = 213;
    /// Frame `recln207`.
    pub const RECLN207: i32 = 214;
    /// Frame `recln208`.
    pub const RECLN208: i32 = 215;
    /// Frame `recln209`.
    pub const RECLN209: i32 = 216;
    /// Frame `recln210`.
    pub const RECLN210: i32 = 217;
    /// Frame `recln211`.
    pub const RECLN211: i32 = 218;
    /// Frame `recln212`.
    pub const RECLN212: i32 = 219;
    /// Frame `recln213`.
    pub const RECLN213: i32 = 220;
    /// Frame `recln214`.
    pub const RECLN214: i32 = 221;
    /// Frame `recln215`.
    pub const RECLN215: i32 = 222;
    /// Frame `recln216`.
    pub const RECLN216: i32 = 223;
    /// Frame `recln217`.
    pub const RECLN217: i32 = 224;
    /// Frame `recln218`.
    pub const RECLN218: i32 = 225;
    /// Frame `recln219`.
    pub const RECLN219: i32 = 226;
    /// Frame `recln220`.
    pub const RECLN220: i32 = 227;
    /// Frame `recln221`.
    pub const RECLN221: i32 = 228;
    /// Frame `recln222`.
    pub const RECLN222: i32 = 229;
    /// Frame `recln223`.
    pub const RECLN223: i32 = 230;
    /// Frame `recln224`.
    pub const RECLN224: i32 = 231;
    /// Frame `recln225`.
    pub const RECLN225: i32 = 232;
    /// Frame `recln226`.
    pub const RECLN226: i32 = 233;
    /// Frame `recln227`.
    pub const RECLN227: i32 = 234;
    /// Frame `recln228`.
    pub const RECLN228: i32 = 235;
    /// Frame `recln229`.
    pub const RECLN229: i32 = 236;
    /// Frame `recln230`.
    pub const RECLN230: i32 = 237;
    /// Frame `recln231`.
    pub const RECLN231: i32 = 238;
    /// Frame `recln232`.
    pub const RECLN232: i32 = 239;
    /// Frame `recln233`.
    pub const RECLN233: i32 = 240;
    /// Frame `recln234`.
    pub const RECLN234: i32 = 241;
    /// Frame `recln235`.
    pub const RECLN235: i32 = 242;
    /// Frame `recln236`.
    pub const RECLN236: i32 = 243;
    /// Frame `recln237`.
    pub const RECLN237: i32 = 244;
    /// Frame `recln238`.
    pub const RECLN238: i32 = 245;
    /// Frame `recln239`.
    pub const RECLN239: i32 = 246;
    /// Frame `recln240`.
    pub const RECLN240: i32 = 247;
    /// Frame `recln101`.
    pub const RECLN101: i32 = 248;
    /// Frame `recln102`.
    pub const RECLN102: i32 = 249;
    /// Frame `recln103`.
    pub const RECLN103: i32 = 250;
    /// Frame `recln104`.
    pub const RECLN104: i32 = 251;
    /// Frame `recln105`.
    pub const RECLN105: i32 = 252;
    /// Frame `recln106`.
    pub const RECLN106: i32 = 253;
    /// Frame `recln107`.
    pub const RECLN107: i32 = 254;
    /// Frame `recln108`.
    pub const RECLN108: i32 = 255;
    /// Frame `recln109`.
    pub const RECLN109: i32 = 256;
    /// Frame `recln110`.
    pub const RECLN110: i32 = 257;
    /// Frame `recln111`.
    pub const RECLN111: i32 = 258;
    /// Frame `recln112`.
    pub const RECLN112: i32 = 259;
    /// Frame `recln113`.
    pub const RECLN113: i32 = 260;
    /// Frame `recln114`.
    pub const RECLN114: i32 = 261;
    /// Frame `recln115`.
    pub const RECLN115: i32 = 262;
    /// Frame `recln116`.
    pub const RECLN116: i32 = 263;
    /// Frame `recln117`.
    pub const RECLN117: i32 = 264;
    /// Frame `recln118`.
    pub const RECLN118: i32 = 265;
    /// Frame `recln119`.
    pub const RECLN119: i32 = 266;
    /// Frame `recln120`.
    pub const RECLN120: i32 = 267;
    /// Frame `recln121`.
    pub const RECLN121: i32 = 268;
    /// Frame `recln122`.
    pub const RECLN122: i32 = 269;
    /// Frame `recln123`.
    pub const RECLN123: i32 = 270;
    /// Frame `recln124`.
    pub const RECLN124: i32 = 271;
    /// Frame `recln125`.
    pub const RECLN125: i32 = 272;
    /// Frame `recln126`.
    pub const RECLN126: i32 = 273;
    /// Frame `recln127`.
    pub const RECLN127: i32 = 274;
    /// Frame `recln128`.
    pub const RECLN128: i32 = 275;
    /// Frame `recln129`.
    pub const RECLN129: i32 = 276;
    /// Frame `recln130`.
    pub const RECLN130: i32 = 277;
    /// Frame `recln131`.
    pub const RECLN131: i32 = 278;
    /// Frame `recln132`.
    pub const RECLN132: i32 = 279;
    /// Frame `recln133`.
    pub const RECLN133: i32 = 280;
    /// Frame `recln134`.
    pub const RECLN134: i32 = 281;
    /// Frame `recln135`.
    pub const RECLN135: i32 = 282;
    /// Frame `recln136`.
    pub const RECLN136: i32 = 283;
    /// Frame `recln137`.
    pub const RECLN137: i32 = 284;
    /// Frame `recln138`.
    pub const RECLN138: i32 = 285;
    /// Frame `recln139`.
    pub const RECLN139: i32 = 286;
    /// Frame `recln140`.
    pub const RECLN140: i32 = 287;
}

/// `chickMoves` move tables.
pub fn chick_moves() -> Vec<MonsterMove> {
    vec![
        monster_move(
            "chick_move_fidget",
            151,
            180,
            Some("chick_stand"),
            vec![
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![], -1),
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("ChickMoan")], -1),
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
            ],
        ),
        monster_move(
            "chick_move_stand",
            121,
            150,
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
                monster_frame(MonsterAi::Stand, 0.0, vec![MonsterAction::name("chick_fidget")], -1),
            ],
        ),
        monster_move(
            "chick_move_start_run",
            181,
            190,
            Some("chick_run"),
            vec![
                monster_frame(MonsterAi::Run, 1.0, vec![], -1),
                monster_frame(MonsterAi::Run, 0.0, vec![], -1),
                monster_frame(MonsterAi::Run, 0.0, vec![], -1),
                monster_frame(MonsterAi::Run, -1.0, vec![], -1),
                monster_frame(MonsterAi::Run, -1.0, vec![], -1),
                monster_frame(MonsterAi::Run, 0.0, vec![], -1),
                monster_frame(MonsterAi::Run, 1.0, vec![], -1),
                monster_frame(MonsterAi::Run, 3.0, vec![], -1),
                monster_frame(MonsterAi::Run, 6.0, vec![], -1),
                monster_frame(MonsterAi::Run, 3.0, vec![], -1),
            ],
        ),
        monster_move(
            "chick_move_run",
            191,
            200,
            None,
            vec![
                monster_frame(MonsterAi::Run, 6.0, vec![], -1),
                monster_frame(MonsterAi::Run, 8.0, vec![], -1),
                monster_frame(MonsterAi::Run, 13.0, vec![], -1),
                monster_frame(MonsterAi::Run, 5.0, vec![MonsterAction::name("monster_done_dodge")], -1),
                monster_frame(MonsterAi::Run, 7.0, vec![], -1),
                monster_frame(MonsterAi::Run, 4.0, vec![], -1),
                monster_frame(MonsterAi::Run, 11.0, vec![], -1),
                monster_frame(MonsterAi::Run, 5.0, vec![], -1),
                monster_frame(MonsterAi::Run, 9.0, vec![], -1),
                monster_frame(MonsterAi::Run, 7.0, vec![], -1),
            ],
        ),
        monster_move(
            "chick_move_walk",
            191,
            200,
            None,
            vec![
                monster_frame(MonsterAi::Walk, 6.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 8.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 13.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 7.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 4.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 11.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 5.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 9.0, vec![], -1),
                monster_frame(MonsterAi::Walk, 7.0, vec![], -1),
            ],
        ),
        monster_move(
            "chick_move_pain1",
            90,
            94,
            Some("chick_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "chick_move_pain2",
            95,
            99,
            Some("chick_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "chick_move_pain3",
            100,
            120,
            Some("chick_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, -6.0, vec![], -1),
                monster_frame(MonsterAi::Move, 3.0, vec![], -1),
                monster_frame(MonsterAi::Move, 11.0, vec![], -1),
                monster_frame(MonsterAi::Move, 3.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 4.0, vec![], -1),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, -3.0, vec![], -1),
                monster_frame(MonsterAi::Move, -4.0, vec![], -1),
                monster_frame(MonsterAi::Move, 5.0, vec![], -1),
                monster_frame(MonsterAi::Move, 7.0, vec![], -1),
                monster_frame(MonsterAi::Move, -2.0, vec![], -1),
                monster_frame(MonsterAi::Move, 3.0, vec![], -1),
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
                monster_frame(MonsterAi::Move, -2.0, vec![], -1),
                monster_frame(MonsterAi::Move, -8.0, vec![], -1),
                monster_frame(MonsterAi::Move, 2.0, vec![], -1),
            ],
        ),
        monster_move(
            "chick_move_death2",
            60,
            82,
            Some("chick_dead"),
            vec![
                monster_frame(MonsterAi::Move, -6.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, -1.0, vec![], -1),
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, -1.0, vec![], -1),
                monster_frame(MonsterAi::Move, -2.0, vec![], -1),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
                monster_frame(MonsterAi::Move, 10.0, vec![], -1),
                monster_frame(MonsterAi::Move, 2.0, vec![], -1),
                monster_frame(MonsterAi::Move, 3.0, vec![], -1),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
                monster_frame(MonsterAi::Move, 2.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 3.0, vec![], -1),
                monster_frame(MonsterAi::Move, 3.0, vec![], -1),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
                monster_frame(MonsterAi::Move, -3.0, vec![], -1),
                monster_frame(MonsterAi::Move, -5.0, vec![], -1),
                monster_frame(MonsterAi::Move, 4.0, vec![], -1),
                monster_frame(MonsterAi::Move, 15.0, vec![], -1),
                monster_frame(MonsterAi::Move, 14.0, vec![], -1),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            ],
        ),
        monster_move(
            "chick_move_death1",
            48,
            59,
            Some("chick_dead"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, 0.0, vec![], -1),
                monster_frame(MonsterAi::Move, -7.0, vec![], -1),
                monster_frame(MonsterAi::Move, 4.0, vec![], -1),
                monster_frame(MonsterAi::Move, 11.0, vec![], -1),
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
            "chick_move_duck",
            83,
            89,
            Some("chick_run"),
            vec![
                monster_frame(MonsterAi::Move, 0.0, vec![MonsterAction::name("monster_duck_down")], -1),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
                monster_frame(MonsterAi::Move, 4.0, vec![MonsterAction::name("monster_duck_hold")], -1),
                monster_frame(MonsterAi::Move, -4.0, vec![], -1),
                monster_frame(MonsterAi::Move, -5.0, vec![MonsterAction::name("monster_duck_up")], -1),
                monster_frame(MonsterAi::Move, 3.0, vec![], -1),
                monster_frame(MonsterAi::Move, 1.0, vec![], -1),
            ],
        ),
        monster_move(
            "chick_move_start_attack1",
            0,
            12,
            None,
            vec![
                monster_frame(
                    MonsterAi::Charge,
                    0.0,
                    vec![MonsterAction::name("Chick_PreAttack1")],
                    -1,
                ),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -3.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 3.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 7.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![MonsterAction::name("chick_attack1")], -1),
            ],
        ),
        monster_move(
            "chick_move_attack1",
            13,
            26,
            None,
            vec![
                monster_frame(MonsterAi::Charge, 19.0, vec![MonsterAction::name("ChickRocket")], -1),
                monster_frame(MonsterAi::Charge, -6.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -5.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -2.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -7.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 10.0, vec![MonsterAction::name("ChickReload")], -1),
                monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 5.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 6.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 6.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 4.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 3.0, vec![MonsterAction::name("chick_rerocket")], -1),
            ],
        ),
        monster_move(
            "chick_move_end_attack1",
            27,
            31,
            Some("chick_run"),
            vec![
                monster_frame(MonsterAi::Charge, -3.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -6.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -4.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -2.0, vec![], -1),
            ],
        ),
        monster_move(
            "chick_move_slash",
            35,
            43,
            None,
            vec![
                monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 7.0, vec![MonsterAction::name("ChickSlash")], -1),
                monster_frame(MonsterAi::Charge, -7.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -1.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -2.0, vec![MonsterAction::name("chick_reslash")], -1),
            ],
        ),
        monster_move(
            "chick_move_end_slash",
            44,
            47,
            Some("chick_run"),
            vec![
                monster_frame(MonsterAi::Charge, -6.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -1.0, vec![], -1),
                monster_frame(MonsterAi::Charge, -6.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 0.0, vec![], -1),
            ],
        ),
        monster_move(
            "chick_move_start_slash",
            32,
            34,
            Some("chick_slash"),
            vec![
                monster_frame(MonsterAi::Charge, 1.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 8.0, vec![], -1),
                monster_frame(MonsterAi::Charge, 3.0, vec![], -1),
            ],
        ),
    ]
}
