//! wu-19: a campaign preset launches into the game view from the menu.
//!
//! End-to-end proof that the windowed menu's campaign Play item resolves
//! through the live ported preset chain (`resolve_preset` over
//! `application_preset` + `resolve_launch`) and lands in an active game.
//! The test opens the menu entry over the Steel corpus, walks
//! main -> Play a game -> Quake II classic -> Quake II campaign ->
//! difficulty -> Play, steps once to drain the launch queue, and asserts
//! the menu overlay drops, a game is active, and the captured frame shows
//! lit world geometry instead of the menu. X screenshots land next to the
//! wu-16 captures in the temp directory.

use std::path::PathBuf;

use qa_app::bootstrap::live_proof::{require_live_corpus, CORPUS_WITNESSES};
use qa_app::bootstrap::startup::StartupEntry;
use qa_app::bootstrap::windowed::open_windowed_application;
use qa_app::options::ApplicationOptions;
use qa_client::input::router::SeatInputEvent;

/// Capture size: 640 by 480 maps UI units to pixels one-to-one.
const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

/// Quake key codes the menu navigates on (`ui/keycodes.h`).
const KEY_DOWN: i32 = 133;
const KEY_ENTER: i32 = 13;

/// Windowed options over the Steel corpus root.
fn menu_options(corpus: PathBuf) -> ApplicationOptions {
    ApplicationOptions {
        windowed: true,
        width: WIDTH,
        height: HEIGHT,
        frame_limit: Some(600),
        corpus_root: corpus.to_string_lossy().into_owned(),
        ..ApplicationOptions::default()
    }
}

fn write_ppm(name: &str, pixels: &[u8]) {
    let path = std::env::temp_dir().join(name);
    let mut rgb = Vec::with_capacity((WIDTH * HEIGHT * 3) as usize);
    for pixel in pixels.as_chunks::<4>().0 {
        rgb.extend_from_slice(&pixel[0..3]);
    }
    let mut file = std::fs::File::create(&path).unwrap();
    use std::io::Write;
    file.write_all(format!("P6\n{WIDTH} {HEIGHT}\n255\n").as_bytes())
        .unwrap();
    file.write_all(&rgb).unwrap();
    eprintln!("preset screenshot: {}", path.display());
}

/// Press and release one router key through application input.
fn press_key(
    composed: &mut qa_app::bootstrap::windowed::WindowedApplication,
    seat: &qa_core::identity::SeatId,
    code: i32,
    stamp: &mut u32,
) {
    for down in [true, false] {
        assert!(
            composed.app.input(&SeatInputEvent::Key {
                seat: seat.clone(),
                time_ms: f64::from(*stamp),
                code,
                down,
            }),
            "menu consumes key {code}"
        );
        *stamp += 1;
    }
}

/// Walk focus Down until one control holds it.
fn focus_to(
    composed: &mut qa_app::bootstrap::windowed::WindowedApplication,
    seat: &qa_core::identity::SeatId,
    control: &str,
    stamp: &mut u32,
) {
    for _ in 0..12 {
        if composed.app.backend().menu_focus_control().as_deref() == Some(control) {
            return;
        }
        press_key(composed, seat, KEY_DOWN, stamp);
    }
    panic!("focus never reached {control}");
}

/// Fraction of pixels that differ between two whole captures.
fn frame_diff(before: &[u8], after: &[u8]) -> f64 {
    let mut changed = 0usize;
    for (a, b) in before.as_chunks::<4>().0.iter().zip(after.as_chunks::<4>().0.iter()) {
        if a[0..3] != b[0..3] {
            changed += 1;
        }
    }
    changed as f64 / (WIDTH * HEIGHT) as f64
}

/// Pixels that differ from pure black (any nonzero RGB channel).
fn count_non_black(pixels: &[u8]) -> usize {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
        .count()
}

#[test]
#[ignore = "live proof: needs Steel corpus/display"]
fn campaign_preset_launches_into_the_game_view() {
    let Some(corpus) = require_live_corpus("Steel corpus", &CORPUS_WITNESSES) else {
        return;
    };
    let options = menu_options(corpus);
    let mut composed = open_windowed_application(&options, StartupEntry::Menu).expect("menu entry opens a window");
    let seat = composed.app.input_seat().expect("menu entry has an input seat");
    assert!(composed.app.backend().menu_open());
    assert!(!composed.app.active_game(), "menu entry has no active game");
    composed.app.step().expect("menu step works");
    let menu_pixels = composed.app.capture_next_frame().expect("menu capture works");
    assert_eq!(menu_pixels.len(), (WIDTH * HEIGHT * 4) as usize);
    write_ppm("qa-wu19-menu.ppm", &menu_pixels);

    // Main -> Play a game -> family menu.
    let mut stamp = 3000u32;
    press_key(&mut composed, &seat, KEY_ENTER, &mut stamp);
    assert_eq!(
        composed.app.backend().menu_active_menu().as_deref(),
        Some("menu:startup:native-family")
    );

    // Family -> Quake classic -> campaign menu. (Quake II's campaign
    // needs players/male, which this Steel corpus does not ship, so the
    // Quake campaign carries the launch proof.)
    focus_to(&mut composed, &seat, "ui:startup:game:q1:classic", &mut stamp);
    press_key(&mut composed, &seat, KEY_ENTER, &mut stamp);
    assert_eq!(
        composed.app.backend().menu_active_menu().as_deref(),
        Some("menu:startup:native-campaign")
    );

    // Campaign -> Quake -> difficulty menu.
    focus_to(&mut composed, &seat, "ui:startup:campaign:q1-classic-id1", &mut stamp);
    press_key(&mut composed, &seat, KEY_ENTER, &mut stamp);
    assert_eq!(
        composed.app.backend().menu_active_menu().as_deref(),
        Some("menu:startup:native-difficulty")
    );

    // Difficulty -> Play queues the preset; one step launches it.
    focus_to(&mut composed, &seat, "ui:startup:play-preset", &mut stamp);
    press_key(&mut composed, &seat, KEY_ENTER, &mut stamp);
    composed.app.step().expect("launch step works");
    assert!(
        !composed.app.backend().menu_open(),
        "preset launch drops the menu overlay"
    );
    assert!(composed.app.active_game(), "preset launch activates the game view");
    composed.app.step().expect("game step works");
    let launched = composed.app.capture_next_frame().expect("game capture works");
    write_ppm("qa-wu19-preset-launched.ppm", &launched);
    assert!(
        frame_diff(&menu_pixels, &launched) > 0.05,
        "the game view replaces the menu on screen"
    );
    let lit = count_non_black(&launched);
    let fraction = lit as f64 / f64::from(WIDTH * HEIGHT);
    assert!(fraction > 0.5, "expected map imagery, got {lit} non-black pixels");
    composed.app.close().expect("close works");
}
