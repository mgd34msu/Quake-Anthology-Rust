//! wu-16: the startup menu is usable from keyboard and gamepad.
//!
//! End-to-end proof that the no-args menu navigates, activates, launches,
//! and quits. One test opens windowed menu entries sequentially (a single
//! thread, so SDL video init/quit never races itself) and drives input two
//! ways: router [`SeatInputEvent`] values through
//! [`StartupApplication::input`](qa_app::bootstrap::startup::StartupApplication::input),
//! and raw [`SdlEvent`] values through the backend's test-only event pump
//! (the same events an X server would deliver). It asserts focus and
//! selection state changes, captures X screenshots before and after an
//! injected Down key to show the focus bar moving rows, activates
//! Quit into a clean drive-loop exit, and — when the Steel corpus is
//! present — plays through Custom game into a live game view.

use std::path::PathBuf;

use qa_app::bootstrap::startup::StartupEntry;
use qa_app::bootstrap::windowed::drive_windowed_application;
use qa_app::bootstrap::windowed::open_windowed_application;
use qa_app::options::ApplicationOptions;
use qa_client::input::router::SeatInputEvent;
use qa_platform::controller::ControllerEvent;
use qa_platform::sdl::SdlEvent;

/// Capture size: 640 by 480 maps UI units to pixels one-to-one.
const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

/// Quake key codes the menu navigates on (`ui/keycodes.h`).
const KEY_DOWN: i32 = 133;
const KEY_UP: i32 = 132;
const KEY_ENTER: i32 = 13;
const KEY_ESCAPE: i32 = 27;

/// SDL scancode/keycode pairs (flagged keycodes carry `0x4000_0000`).
const SDL_DOWN: (i32, i32) = (81, 0x4000_0000 | 81);
const SDL_UP: (i32, i32) = (82, 0x4000_0000 | 82);
const SDL_ENTER: (i32, i32) = (40, 13);
const SDL_ESCAPE: (i32, i32) = (41, 27);

/// Gamepad buttons the menu navigates on (dpad up/down, A).
const PAD_UP: u8 = 11;
const PAD_DOWN: u8 = 12;
const PAD_A: u8 = 0;

/// Witness files proving a Steel corpus root holds all three families.
const CORPUS_WITNESSES: [&str; 3] = ["q1/id1/pak0.pak", "q2/baseq2/pak0.pak", "q3a/baseq3/pak0.pk3"];

/// Locate the Steel corpus root without hardcoding any absolute path.
fn find_steel_corpus() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .map(|dir| dir.join("target"))
        .find(|root| CORPUS_WITNESSES.iter().all(|witness| root.join(witness).is_file()))
}

/// Windowed options over the ancestor `target/` directory (the menu mounts
/// no content, so any readable corpus root works).
fn menu_options() -> ApplicationOptions {
    let corpus_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .to_string_lossy()
        .into_owned();
    ApplicationOptions {
        windowed: true,
        width: WIDTH,
        height: HEIGHT,
        frame_limit: Some(600),
        corpus_root,
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
    eprintln!("menu screenshot: {}", path.display());
}

/// Whether a pixel reads as the menu's focused-fill bar
/// (`UiSkinColors.focused` (0.30, 0.19, 0.09) over the dark backdrop):
/// brown, strictly red > green > blue. Baked Q1/Q2 charsets draw menu
/// text white (donor `menu-font.ts` picks "baked" for non-Q3), so focus
/// is tracked through the bar fill, not text tint. White labels fail on
/// red-vs-green, and the dark fills fail on red outright.
fn is_focus_bar(pixel: &[u8]) -> bool {
    let (red, green, blue) = (pixel[0], pixel[1], pixel[2]);
    (35..130).contains(&red) && (20..85).contains(&green) && (8..60).contains(&blue) && red > green && green > blue
}

/// Count focus-bar pixels in one menu row band (buttons sit at x 64..288,
/// y `118 + row * 34`, 30 tall).
fn row_focused(pixels: &[u8], row: u32) -> usize {
    let mut bar = 0;
    for y in (118 + row * 34)..(118 + row * 34 + 30) {
        for x in 64..288 {
            let at = ((y * WIDTH + x) * 4) as usize;
            if is_focus_bar(&pixels[at..at + 3]) {
                bar += 1;
            }
        }
    }
    bar
}

/// Fraction of pixels that differ between two captures inside one row band.
fn row_diff(before: &[u8], after: &[u8], row: u32) -> f64 {
    let mut changed = 0usize;
    let mut total = 0usize;
    for y in (118 + row * 34)..(118 + row * 34 + 30) {
        for x in 64..288 {
            let at = ((y * WIDTH + x) * 4) as usize;
            total += 1;
            if before[at..at + 3] != after[at..at + 3] {
                changed += 1;
            }
        }
    }
    changed as f64 / total as f64
}

/// Press and release one SDL key through the test-only pump.
fn press_sdl(composed: &mut qa_app::bootstrap::windowed::WindowedApplication, key: (i32, i32), stamp: &mut u32) {
    for down in [true, false] {
        *stamp += 1;
        composed
            .app
            .backend_mut()
            .inject_platform_event(SdlEvent::Key {
                timestamp: *stamp,
                down,
                repeat: false,
                scancode: key.0,
                keycode: key.1,
                modifiers: 0,
            })
            .expect("pump delivers");
    }
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

/// Press and release one gamepad button through the test-only pump.
fn press_pad(composed: &mut qa_app::bootstrap::windowed::WindowedApplication, button: u8, stamp: &mut u32) {
    for down in [true, false] {
        *stamp += 1;
        composed
            .app
            .backend_mut()
            .inject_controller_event(ControllerEvent::Button {
                timestamp: *stamp,
                instance: 7,
                slot: Some(0),
                button,
                down,
            })
            .expect("pad delivers");
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

#[test]
fn startup_menu_navigates_activates_launches_and_quits() {
    // Phase A: keyboard/gamepad navigation, activation, screenshots, quit.
    let menu_pixels = {
        let options = menu_options();
        let mut composed = open_windowed_application(&options, StartupEntry::Menu)
            .unwrap_or_else(|error| panic!("menu entry opens a window: {error}"));
        let seat = composed.app.input_seat().expect("menu entry has an input seat");
        assert!(composed.app.backend().menu_open());
        assert_eq!(
            composed.app.backend().menu_active_menu().as_deref(),
            Some("menu:startup:main")
        );
        assert_eq!(
            composed.app.backend().menu_focus_control().as_deref(),
            Some("ui:startup:native")
        );
        assert!(!composed.app.active_game(), "menu entry has no active game");
        composed.app.step().expect("menu step works");
        let before = composed.app.capture_next_frame().expect("menu capture works");
        assert_eq!(before.len(), (WIDTH * HEIGHT * 4) as usize);
        write_ppm("qa-wu16-menu-before.ppm", &before);

        // Inject a raw SDL Down key through the test-only pump: the same
        // event an X server would deliver for the Down arrow.
        let mut stamp = 1000u32;
        press_sdl(&mut composed, SDL_DOWN, &mut stamp);
        assert_eq!(
            composed.app.backend().menu_focus_control().as_deref(),
            Some("ui:startup:load"),
            "injected Down moves focus to Load Game"
        );
        composed.app.step().expect("menu step works");
        let after = composed.app.capture_next_frame().expect("menu capture works");
        write_ppm("qa-wu16-menu-after.ppm", &after);

        // The focus bar leaves row 0 for row 1; untouched rows rest.
        let bar_before = [row_focused(&before, 0), row_focused(&before, 1)];
        let bar_after = [row_focused(&after, 0), row_focused(&after, 1)];
        assert!(bar_before[0] >= 20, "row 0 holds the focus bar, got {}", bar_before[0]);
        assert!(
            bar_before[0] > 3 * bar_before[1].max(1),
            "focus bar sits on row 0 before, got {bar_before:?}"
        );
        assert!(bar_after[1] >= 20, "row 1 holds the focus bar, got {}", bar_after[1]);
        assert!(
            bar_after[1] > 3 * bar_after[0].max(1),
            "focus bar sits on row 1 after, got {bar_after:?}"
        );
        assert!(row_diff(&before, &after, 0) > 0.02, "row 0 repaints without focus");
        assert!(row_diff(&before, &after, 1) > 0.02, "row 1 repaints with focus");
        assert!(row_diff(&before, &after, 2) < 0.01, "row 2 rests");
        assert!(row_diff(&before, &after, 3) < 0.01, "row 3 rests");

        // Router seat events drive the same path: Up returns to Play a
        // game, Enter opens the family menu, Escape returns to main.
        press_key(&mut composed, &seat, KEY_UP, &mut stamp);
        assert_eq!(
            composed.app.backend().menu_focus_control().as_deref(),
            Some("ui:startup:native")
        );
        press_key(&mut composed, &seat, KEY_ENTER, &mut stamp);
        assert_eq!(
            composed.app.backend().menu_active_menu().as_deref(),
            Some("menu:startup:native-family"),
            "activating Play a game opens the family menu"
        );
        press_key(&mut composed, &seat, KEY_ESCAPE, &mut stamp);
        assert_eq!(
            composed.app.backend().menu_active_menu().as_deref(),
            Some("menu:startup:main")
        );

        // Gamepad: assign a pad, walk with the dpad, activate with A.
        composed
            .app
            .backend_mut()
            .inject_controller_event(ControllerEvent::Assignment {
                timestamp: stamp,
                slot: 0,
                previous: None,
                instance: Some(7),
            })
            .expect("pad assigns");
        press_pad(&mut composed, PAD_DOWN, &mut stamp);
        assert_eq!(
            composed.app.backend().menu_focus_control().as_deref(),
            Some("ui:startup:load"),
            "dpad down moves focus"
        );
        press_pad(&mut composed, PAD_UP, &mut stamp);
        assert_eq!(
            composed.app.backend().menu_focus_control().as_deref(),
            Some("ui:startup:native")
        );
        press_pad(&mut composed, PAD_A, &mut stamp);
        assert_eq!(
            composed.app.backend().menu_active_menu().as_deref(),
            Some("menu:startup:native-family"),
            "gamepad A activates Play a game"
        );
        press_sdl(&mut composed, SDL_ESCAPE, &mut stamp);
        assert_eq!(
            composed.app.backend().menu_active_menu().as_deref(),
            Some("menu:startup:main")
        );
        press_sdl(&mut composed, SDL_UP, &mut stamp);
        assert_eq!(
            composed.app.backend().menu_focus_control().as_deref(),
            Some("ui:startup:quit"),
            "Up wraps from the first item to Quit"
        );

        // Activating Quit sets the quit flag and the drive loop exits
        // cleanly, which is the binary's exit-0 path.
        assert!(!composed.quit.get());
        press_sdl(&mut composed, SDL_ENTER, &mut stamp);
        assert!(composed.quit.get(), "activating Quit sets the quit flag");
        let frames =
            drive_windowed_application(&mut composed.app, &composed.quit, Some(600)).expect("quit drive exits cleanly");
        assert!(composed.app.is_closed());
        eprintln!("quit drive stepped {frames} frames and closed");
        before
    };

    // Phase B: Play launches into a game view (needs the Steel corpus).
    let Some(corpus) = find_steel_corpus() else {
        eprintln!("skipped: Steel corpus not found above {}", env!("CARGO_MANIFEST_DIR"));
        return;
    };
    let mut options = menu_options();
    options.corpus_root = corpus.to_string_lossy().into_owned();
    let mut composed = open_windowed_application(&options, StartupEntry::Menu).expect("menu entry opens a window");
    let seat = composed.app.input_seat().expect("menu entry has an input seat");
    let mut stamp = 2000u32;
    press_key(&mut composed, &seat, KEY_ENTER, &mut stamp);
    assert_eq!(
        composed.app.backend().menu_active_menu().as_deref(),
        Some("menu:startup:native-family")
    );
    focus_to(&mut composed, &seat, "ui:startup:custom", &mut stamp);
    press_key(&mut composed, &seat, KEY_ENTER, &mut stamp);
    assert_eq!(
        composed.app.backend().menu_active_menu().as_deref(),
        Some("menu:startup:session"),
        "Custom game opens the session menu"
    );
    focus_to(&mut composed, &seat, "ui:startup:play", &mut stamp);
    press_key(&mut composed, &seat, KEY_ENTER, &mut stamp);
    composed.app.step().expect("launch step works");
    assert!(!composed.app.backend().menu_open(), "launch drops the menu overlay");
    assert!(composed.app.active_game(), "Play launches into a game view");
    composed.app.step().expect("game step works");
    let launched = composed.app.capture_next_frame().expect("game capture works");
    write_ppm("qa-wu16-menu-launched.ppm", &launched);
    assert!(
        frame_diff(&menu_pixels, &launched) > 0.05,
        "the game view replaces the menu on screen"
    );
    composed.app.close().expect("close works");
}
