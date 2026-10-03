//! wu-14: the startup menu renders legible button and title text.
//!
//! Regression coverage for blank menu labels: the menu entry painted its
//! backdrop, panel, and buttons but every label was empty because text runs
//! were captured as solid bars and glyph UVs never reached GL. This test
//! opens the real menu entry under a window, captures one frame, and proves
//! that text bands hold multiple glyph-shaped ink clusters instead of flat
//! fills or blanks. It fails loudly (no skips): the menu needs no catalog
//! content, so an X server is the only requirement.

use std::path::PathBuf;

use qa_app::bootstrap::startup::StartupEntry;
use qa_app::bootstrap::windowed::open_windowed_application;
use qa_app::options::ApplicationOptions;

/// Capture size: 640 by 480 maps UI units to pixels one-to-one, so text
/// bands below are the menu's own layout coordinates.
const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

/// Title `QUAKE` at (64, 44) scale 6: five 48-pixel cells.
const TITLE_BAND: (u32, u32, u32, u32) = (64, 44, 304, 92);

/// Button labels at x 74, y `124 + row * 34`, scale 2.6 (20.8-pixel cells).
const LABEL_BANDS: [(u32, u32, u32, u32, &str, usize); 4] = [
    (74, 124, 307, 145, "Play a game", 6),
    (74, 158, 267, 179, "Load Game", 5),
    (74, 192, 225, 213, "Options", 4),
    (74, 226, 163, 247, "Quit", 3),
];

/// Shade difference that separates ink from a flat fill.
const INK_DISTANCE: u8 = 40;

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
        frame_limit: Some(3),
        corpus_root,
        ..ApplicationOptions::default()
    }
}

/// Median of one column over an extended band: text never fills the column,
/// so the median is the flat fill behind the glyphs at that x.
fn column_background(pixels: &[u8], width: u32, height: u32, x: u32, y0: u32, y1: u32) -> [u8; 3] {
    let top = y0.saturating_sub(8);
    let bottom = (y1 + 8).min(height);
    let mut channels = [Vec::new(), Vec::new(), Vec::new()];
    for y in top..bottom {
        let at = ((y * width + x) * 4) as usize;
        for channel in 0..3 {
            channels[channel].push(pixels[at + channel]);
        }
    }
    let mut background = [0u8; 3];
    for (channel, values) in channels.iter_mut().enumerate() {
        values.sort_unstable();
        background[channel] = values[values.len() / 2];
    }
    background
}

/// Count glyph-shaped ink clusters in a band: columns holding at least two
/// ink pixels, split into runs on two consecutive empty columns. Returns
/// the run count and the ink fraction of the band.
fn band_runs(pixels: &[u8], width: u32, height: u32, band: (u32, u32, u32, u32)) -> (usize, f64) {
    let (x0, y0, x1, y1) = band;
    let mut runs = 0;
    let mut in_run = false;
    let mut empty = 0;
    let mut ink = 0usize;
    for x in x0..x1.min(width) {
        let background = column_background(pixels, width, height, x, y0, y1.min(height));
        let mut column_ink = 0;
        for y in y0..y1.min(height) {
            let at = ((y * width + x) * 4) as usize;
            let distance = (0..3)
                .map(|channel| pixels[at + channel].abs_diff(background[channel]))
                .max()
                .unwrap_or(0);
            if distance > INK_DISTANCE {
                column_ink += 1;
            }
        }
        ink += column_ink;
        if column_ink >= 2 {
            if !in_run {
                runs += 1;
                in_run = true;
            }
            empty = 0;
        } else {
            empty += 1;
            if empty >= 2 {
                in_run = false;
            }
        }
    }
    let area = (x1.saturating_sub(x0) * y1.saturating_sub(y0)) as f64;
    (runs, ink as f64 / area.max(1.0))
}

fn write_ppm(path: &std::path::Path, pixels: &[u8]) {
    let mut rgb = Vec::with_capacity((WIDTH * HEIGHT * 3) as usize);
    for pixel in pixels.as_chunks::<4>().0 {
        rgb.extend_from_slice(&pixel[0..3]);
    }
    let mut file = std::fs::File::create(path).unwrap();
    use std::io::Write;
    file.write_all(format!("P6\n{WIDTH} {HEIGHT}\n255\n").as_bytes())
        .unwrap();
    file.write_all(&rgb).unwrap();
}

#[test]
fn menu_entry_renders_glyph_clusters_for_title_and_buttons() {
    let options = menu_options();
    let mut composed = open_windowed_application(&options, StartupEntry::Menu)
        .unwrap_or_else(|error| panic!("menu entry opens a window: {error}"));
    composed.app.step().expect("menu step works");
    let pixels = composed.app.capture_next_frame().expect("menu capture works");
    assert!(!composed.app.active_game(), "menu entry has no active game");
    composed.app.close().expect("close works");
    assert_eq!(pixels.len(), (WIDTH * HEIGHT * 4) as usize);

    let path = std::env::temp_dir().join("qa-wu14-menu.ppm");
    write_ppm(&path, &pixels);
    eprintln!("menu screenshot: {}", path.display());

    let (title_runs, title_ink) = band_runs(&pixels, WIDTH, HEIGHT, TITLE_BAND);
    assert!(
        title_runs >= 4,
        "title holds QUAKE in glyph clusters, got {title_runs} runs"
    );
    assert!(
        (0.01..0.60).contains(&title_ink),
        "title ink is partial, got {title_ink:.3}"
    );
    for (x0, y0, x1, y1, label, minimum) in LABEL_BANDS {
        let (runs, ink) = band_runs(&pixels, WIDTH, HEIGHT, (x0, y0, x1, y1));
        assert!(runs >= minimum, "{label} holds glyph clusters, got {runs} runs");
        assert!((0.01..0.60).contains(&ink), "{label} ink is partial, got {ink:.3}");
    }
}
