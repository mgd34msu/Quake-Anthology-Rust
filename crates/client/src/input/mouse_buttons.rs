//! Port of Quake-Anthology-TS `src/input/mouse-buttons.ts`

/// Physical mouse button to Quake button: middle and right swap.
///
/// Physical inputs retain SDL button IDs in events and saved seat settings.
#[must_use]
pub fn quake_mouse_button(physical_button: i32) -> i32 {
    if physical_button == 2 {
        3
    } else if physical_button == 3 {
        2
    } else {
        physical_button
    }
}

/// Quake button to physical mouse button (the swap is symmetric).
#[must_use]
pub fn physical_mouse_button(quake_button: i32) -> i32 {
    quake_mouse_button(quake_button)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_and_right_swap() {
        assert_eq!(quake_mouse_button(2), 3);
        assert_eq!(quake_mouse_button(3), 2);
    }

    #[test]
    fn other_buttons_pass_through() {
        for button in [0, 1, 4, 5, -1] {
            assert_eq!(quake_mouse_button(button), button);
        }
    }

    #[test]
    fn physical_round_trips() {
        for button in 0..6 {
            assert_eq!(physical_mouse_button(quake_mouse_button(button)), button);
        }
    }
}
