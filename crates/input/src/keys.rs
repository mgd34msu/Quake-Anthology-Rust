//! Shared config key names over the platform's neutral control IDs.
//! Native spelling: Q3 cl_keys.c, Q1/Q2 keys.c; device IDs are engine data.
struct Name {
    text: &'static str,
    control: u16,
    native: Option<u8>,
}
macro_rules! names { ($($text:literal, $control:literal, $native:expr);* $(;)?) => { &[$(Name { text: $text, control: $control, native: $native }),*] }; }
const NAMES: &[Name] = names![
    "a", 4, Some(97);
    "b", 5, Some(98);
    "c", 6, Some(99);
    "d", 7, Some(100);
    "e", 8, Some(101);
    "f", 9, Some(102);
    "g", 10, Some(103);
    "h", 11, Some(104);
    "i", 12, Some(105);
    "j", 13, Some(106);
    "k", 14, Some(107);
    "l", 15, Some(108);
    "m", 16, Some(109);
    "n", 17, Some(110);
    "o", 18, Some(111);
    "p", 19, Some(112);
    "q", 20, Some(113);
    "r", 21, Some(114);
    "s", 22, Some(115);
    "t", 23, Some(116);
    "u", 24, Some(117);
    "v", 25, Some(118);
    "w", 26, Some(119);
    "x", 27, Some(120);
    "y", 28, Some(121);
    "z", 29, Some(122);
    "1", 30, Some(49);
    "2", 31, Some(50);
    "3", 32, Some(51);
    "4", 33, Some(52);
    "5", 34, Some(53);
    "6", 35, Some(54);
    "7", 36, Some(55);
    "8", 37, Some(56);
    "9", 38, Some(57);
    "0", 39, Some(48);
    "ENTER", 40, Some(13);
    "ESCAPE", 41, Some(27);
    "BACKSPACE", 42, Some(127);
    "TAB", 43, Some(9);
    "SPACE", 44, Some(32);
    "-", 45, Some(45);
    "=", 46, Some(61);
    "[", 47, Some(91);
    "]", 48, Some(93);
    "BACKSLASH", 49, Some(92);
    "SEMICOLON", 51, Some(59);
    "'", 52, Some(39);
    "`", 53, Some(96);
    ",", 54, Some(44);
    ".", 55, Some(46);
    "/", 56, Some(47);
    "CAPSLOCK", 57, Some(129);
    "F1", 58, Some(145);
    "F2", 59, Some(146);
    "F3", 60, Some(147);
    "F4", 61, Some(148);
    "F5", 62, Some(149);
    "F6", 63, Some(150);
    "F7", 64, Some(151);
    "F8", 65, Some(152);
    "F9", 66, Some(153);
    "F10", 67, Some(154);
    "F11", 68, Some(155);
    "F12", 69, Some(156);
    "PRINTSCREEN", 70, None;
    "SCROLLLOCK", 71, None;
    "PAUSE", 72, Some(131);
    "INS", 73, Some(139);
    "HOME", 74, Some(143);
    "PGUP", 75, Some(142);
    "DEL", 76, Some(140);
    "END", 77, Some(144);
    "PGDN", 78, Some(141);
    "RIGHTARROW", 79, Some(135);
    "LEFTARROW", 80, Some(134);
    "DOWNARROW", 81, Some(133);
    "UPARROW", 82, Some(132);
    "KP_NUMLOCK", 83, Some(175);
    "KP_SLASH", 84, Some(172);
    "KP_STAR", 85, Some(176);
    "KP_MINUS", 86, Some(173);
    "KP_PLUS", 87, Some(174);
    "KP_ENTER", 88, Some(169);
    "KP_END", 89, Some(166);
    "KP_DOWNARROW", 90, Some(167);
    "KP_PGDN", 91, Some(168);
    "KP_LEFTARROW", 92, Some(163);
    "KP_5", 93, Some(164);
    "KP_RIGHTARROW", 94, Some(165);
    "KP_HOME", 95, Some(160);
    "KP_UPARROW", 96, Some(161);
    "KP_PGUP", 97, Some(162);
    "KP_INS", 98, Some(170);
    "KP_DEL", 99, Some(171);
    "KP_EQUALS", 103, Some(177);
    "F13", 104, Some(157);
    "F14", 105, Some(158);
    "F15", 106, Some(159);
    "F16", 107, None;
    "F17", 108, None;
    "F18", 109, None;
    "F19", 110, None;
    "F20", 111, None;
    "F21", 112, None;
    "F22", 113, None;
    "F23", 114, None;
    "F24", 115, None;
    "CTRL", 224, Some(137);
    "SHIFT", 225, Some(138);
    "ALT", 226, Some(136);
    "COMMAND", 227, Some(128);
    "MOUSE1", 513, Some(178);
    "MOUSE2", 514, Some(179);
    "MOUSE3", 515, Some(180);
    "MOUSE4", 516, Some(181);
    "MOUSE5", 517, Some(182);
    "MOUSE6", 518, None;
    "MOUSE7", 519, None;
    "MOUSE8", 520, None;
    "MOUSE9", 521, None;
    "MOUSE10", 522, None;
    "MOUSE11", 523, None;
    "MOUSE12", 524, None;
    "MOUSE13", 525, None;
    "MOUSE14", 526, None;
    "MOUSE15", 527, None;
    "MOUSE16", 528, None;
    "MOUSE17", 529, None;
    "MOUSE18", 530, None;
    "MOUSE19", 531, None;
    "MOUSE20", 532, None;
    "MOUSE21", 533, None;
    "MOUSE22", 534, None;
    "MOUSE23", 535, None;
    "MOUSE24", 536, None;
    "MOUSE25", 537, None;
    "MOUSE26", 538, None;
    "MOUSE27", 539, None;
    "MOUSE28", 540, None;
    "MOUSE29", 541, None;
    "MOUSE30", 542, None;
    "MOUSE31", 543, None;
    "MOUSE32", 580, None;
    "JOY1", 544, Some(185);
    "JOY2", 545, Some(186);
    "JOY3", 546, Some(187);
    "JOY4", 547, Some(188);
    "JOY5", 548, Some(189);
    "JOY6", 549, Some(190);
    "JOY7", 550, Some(191);
    "JOY8", 551, Some(192);
    "JOY9", 552, Some(193);
    "JOY10", 553, Some(194);
    "JOY11", 554, Some(195);
    "JOY12", 555, Some(196);
    "JOY13", 556, Some(197);
    "JOY14", 557, Some(198);
    "JOY15", 558, Some(199);
    "JOY16", 559, Some(200);
    "JOY17", 560, Some(201);
    "JOY18", 561, Some(202);
    "JOY19", 562, Some(203);
    "JOY20", 563, Some(204);
    "JOY21", 564, Some(205);
    "JOY22", 565, Some(206);
    "JOY23", 566, Some(207);
    "JOY24", 567, Some(208);
    "JOY25", 568, Some(209);
    "JOY26", 569, Some(210);
    "JOY27", 570, Some(211);
    "JOY28", 571, Some(212);
    "JOY29", 572, Some(213);
    "JOY30", 573, Some(214);
    "JOY31", 574, Some(215);
    "JOY32", 575, Some(216);
    "MWHEELUP", 576, Some(184);
    "MWHEELDOWN", 577, Some(183);
    "MWHEELLEFT", 578, None;
    "MWHEELRIGHT", 579, None;
    "AUX1", 640, Some(217);
    "AUX2", 641, Some(218);
    "AUX3", 642, Some(219);
    "AUX4", 643, Some(220);
    "AUX5", 644, Some(221);
    "AUX6", 645, Some(222);
    "AUX7", 646, Some(223);
    "AUX8", 647, Some(224);
    "AUX9", 648, Some(225);
    "AUX10", 649, Some(226);
    "AUX11", 650, Some(227);
    "AUX12", 651, Some(228);
    "AUX13", 652, Some(229);
    "AUX14", 653, Some(230);
    "AUX15", 654, Some(231);
    "AUX16", 655, Some(232);
    "AUX17", 656, None;
    "AUX18", 657, None;
    "AUX19", 658, None;
    "AUX20", 659, None;
    "AUX21", 660, None;
    "AUX22", 661, None;
    "AUX23", 662, None;
    "AUX24", 663, None;
    "AUX25", 664, None;
    "AUX26", 665, None;
    "AUX27", 666, None;
    "AUX28", 667, None;
    "AUX29", 668, None;
    "AUX30", 669, None;
    "AUX31", 670, None;
    "AUX32", 671, None;
    "POWER", 102, Some(130);
    "!", 30, Some(33);
    "@", 31, Some(64);
    "#", 32, Some(35);
    "$", 33, Some(36);
    "%", 34, Some(37);
    "^", 35, Some(94);
    "&", 36, Some(38);
    "*", 37, Some(42);
    "(", 38, Some(40);
    ")", 39, Some(41);
    "_", 45, Some(95);
    "+", 46, Some(43);
    "{", 47, Some(123);
    "}", 48, Some(125);
    "|", 49, Some(124);
    ":", 51, Some(58);
    "\"", 52, Some(34);
    "~", 53, Some(126);
    "<", 54, Some(60);
    ">", 55, Some(62);
    "?", 56, Some(63);
];
pub fn normalize(control: u16) -> u16 {
    match control {
        228 => 224,
        229 => 225,
        230 => 226,
        231 => 227,
        _ => control,
    }
}
pub fn parse(text: &str) -> Option<u16> {
    if let Some(control) = NAMES
        .iter()
        .find(|n| n.text.eq_ignore_ascii_case(text))
        .map(|n| n.control)
    {
        return Some(control);
    }
    if text.len() == 1 {
        return from_native(text.as_bytes()[0]);
    }
    // Native Q3 hex names represent native key numbers, not physical IDs.
    if text.len() == 4
        && text
            .as_bytes()
            .get(..2)
            .is_some_and(|s| s.eq_ignore_ascii_case(b"0x"))
    {
        let code = u8::from_str_radix(&text[2..], 16).ok()?;
        return from_native(code);
    }
    None
}
pub fn name(control: u16) -> Option<&'static str> {
    NAMES
        .iter()
        .find(|n| n.control == normalize(control))
        .map(|n| n.text)
}
pub fn native_number(control: u16) -> Option<u8> {
    let control = normalize(control);
    if (768..1024).contains(&control) {
        Some((control - 768) as u8)
    } else {
        NAMES
            .iter()
            .find(|entry| entry.control == control && entry.native.is_some())
            .and_then(|entry| entry.native)
    }
}

fn from_native(code: u8) -> Option<u16> {
    NAMES
        .iter()
        .find(|entry| entry.native == Some(code.to_ascii_lowercase()))
        .map(|entry| entry.control)
        .or_else(|| {
            // Original key numbers without a physical platform key still have
            // stable slots, so controller/module extensions can bind them.
            Some(768 + u16::from(code))
        })
}
pub fn write_name(control: u16, output: &mut impl std::fmt::Write) -> std::fmt::Result {
    if let Some(name) = name(control) {
        output.write_str(name)
    } else if (768..1024).contains(&control) {
        write!(output, "0x{:02x}", control - 768)
    } else {
        write!(output, "control{control}")
    }
}
