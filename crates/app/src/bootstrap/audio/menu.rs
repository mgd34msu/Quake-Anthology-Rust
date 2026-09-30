//! Menu sound selection by game family.
//!
//! Port of donor `src/app/bootstrap/audio/menu.ts` (`menuSoundPath`).

use qa_client::ui::common::controller::UiSound;
use qa_content::contract::GameFamily;

/// Menu sound path for a family and UI event.
#[must_use]
pub fn menu_sound_path(family: GameFamily, event: UiSound) -> String {
    let index = if family == GameFamily::Q1 {
        match event {
            UiSound::Open => 2,
            UiSound::Move => 1,
            _ => 3,
        }
    } else {
        match event {
            UiSound::Open => 1,
            UiSound::Close => 3,
            UiSound::Reject if family == GameFamily::Q3 => 4,
            _ => 2,
        }
    };
    format!("misc/menu{index}.wav")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q1_uses_open_move_other() {
        assert_eq!(menu_sound_path(GameFamily::Q1, UiSound::Open), "misc/menu2.wav");
        assert_eq!(menu_sound_path(GameFamily::Q1, UiSound::Move), "misc/menu1.wav");
        assert_eq!(menu_sound_path(GameFamily::Q1, UiSound::Close), "misc/menu3.wav");
        assert_eq!(menu_sound_path(GameFamily::Q1, UiSound::Change), "misc/menu3.wav");
        assert_eq!(menu_sound_path(GameFamily::Q1, UiSound::Reject), "misc/menu3.wav");
    }

    #[test]
    fn q2_uses_open_close_other() {
        assert_eq!(menu_sound_path(GameFamily::Q2, UiSound::Open), "misc/menu1.wav");
        assert_eq!(menu_sound_path(GameFamily::Q2, UiSound::Close), "misc/menu3.wav");
        assert_eq!(menu_sound_path(GameFamily::Q2, UiSound::Move), "misc/menu2.wav");
        assert_eq!(menu_sound_path(GameFamily::Q2, UiSound::Change), "misc/menu2.wav");
        assert_eq!(menu_sound_path(GameFamily::Q2, UiSound::Reject), "misc/menu2.wav");
    }

    #[test]
    fn q3_rejects_with_track_4() {
        assert_eq!(menu_sound_path(GameFamily::Q3, UiSound::Open), "misc/menu1.wav");
        assert_eq!(menu_sound_path(GameFamily::Q3, UiSound::Close), "misc/menu3.wav");
        assert_eq!(menu_sound_path(GameFamily::Q3, UiSound::Reject), "misc/menu4.wav");
        assert_eq!(menu_sound_path(GameFamily::Q3, UiSound::Move), "misc/menu2.wav");
        assert_eq!(menu_sound_path(GameFamily::Q3, UiSound::Change), "misc/menu2.wav");
    }
}
