//! Mission-pack finale text
//! (`src/content/q1/missionpacks/world/finale-text.ts`).
//!
//! Original mission pack client.qc / ending.qc strings.
//! GPL-2.0-or-later.

use crate::q1::foundation::types::Q1Edition;

/// Classic mission-pack finales.
pub const MISSION_FINALES: &[(&str, &str)] = &[
    ("$qc_finale_r1", "Victory! The Overlord's mangled\nremains are the evidence.  His evil\nWrath army?  Cast out to wander\naimlessly throughout time.\n\nAs the Slipgate fog surrounds you,\nthoughts rage into your consciousness:\nHas Quake's oppressive reign ended?\nIs it Salvation, or Damnation, which\nwaits beyond the Vortex?\n\nAnother thought, not quite your own,\nrazors through the haze.  \"Forgiveness\ncan yet be granted; Our Master remains\nto absolve your sins against his Chosen.\nFall down upon your knees-pray for\nQuake's mercy.\""),
    ("$qc_finale_hip1", "Deep within the bowels of the\nResearch Facility, you discover the\npassage that the followers of Quake\nhave used to enter our world.\nThe bastards used some type of\ngigantic teleporter to overload\none of our own slipgates!  As long as\nthis portal exists, Earth will never\nbe safe from Quake's cruel minions."),
    ("$qc_finale_hip2", "After destroying the power generator,\nyou pass beyond the gate of Mortum's\nKeep.  A wave of nausea suddenly flows\nover you and you find yourself cast\nout into a liquid void.  You float\nlifelessly, yet aware, in a lavender\nsea of energy."),
    ("$qc_finale_hipend", "After the last echoes of Armagon's\ndeath yell fade away, you breathe a\nheavy sigh of relief.  With the loss\nof his magic, Armagon's fortress\nbegins to collapse.  The rift he\ncreated to send his grisly troops\nthrough time slowly closes and seals\nitself forever.  In the chaos that\nensues, a wall collapses, revealing\none remaining time portal.  With your\nchances to escape rapidly growing\nslim, you race for the portal,\nmindless of your destination.  In a\nflash of light, you find yourself\nback at Command HQ, safe and sound."),
    ("$qc_finale_hip1m4", "If you can find the source of the\nportal's power, you can shut it\ndown--possibly forever!  With only a\nmoment's consideration for your own\nsafety, you re-enter the dark domain,\nknowing Hell would be a better fate\nthan experiencing the reign of Quake."),
    ("$qc_finale_hip2m5", "After what seems like an eternity,\nyou feel the presence of a diabolical\nintelligence.  You are held helpless\nfor a moment as your mind is open to\nthat of Armagon--Quake's General and\nmaster of this realm.  Recognizing\nyou as the one who foiled his\nattempt to conquer Earth, a hellish\nhowl fills your mind and blots out\nall consciousness.  When you awake,\nyou find yourself on the shores of\nreality, but in a time and place\nunknown to you."),
    ("$qc_finale_hipend2", "Congratulations!  You are victorious!\nThe minions of Quake have once again\nfallen before your mighty hand.\nIs this the last you will see of\nQuake's hellions?\n\nOnly time will tell..."),
    ("$qc_finale_coop", "You have destroyed Quake's\nTemporal Teleporter. His assault\non Time has been defeated."),
    ("$qc_finale_rogue_end", "\nFinally, Quake's Temporal Teleporter\nyields to your assault. A high\npitched scream emits from the\ndevastated device as stressed steel\nblasts outward to rock the cavern.\nThe machine is devoured by molten lava.\n\nThe ground shudders as reality shifts\nback to its predestined path.\n\nYou run to enter the charged time pod,\nscrambling in as the chamber closes.\nYour consciousness fades as you realize\nyou have halted Quake's plans for...\n\nThe Dissolution of Eternity."),
];

/// Mission-pack finale text (`missionFinaleText`). Only the classic
/// edition resolves keys.
#[must_use]
pub fn mission_finale_text(edition: Q1Edition, key: &str) -> String {
    match edition {
        Q1Edition::Classic => MISSION_FINALES
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, text)| (*text).to_string())
            .unwrap_or_else(|| key.to_string()),
        Q1Edition::Rerelease => key.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mission_finales_resolve_classic_only() {
        assert!(mission_finale_text(Q1Edition::Classic, "$qc_finale_r1").contains("Victory!"));
        assert_eq!(
            mission_finale_text(Q1Edition::Rerelease, "$qc_finale_r1"),
            "$qc_finale_r1"
        );
    }
}
