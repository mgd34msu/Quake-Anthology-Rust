//! wu-23: the windowed menu loads the real console charset when game
//! content is installed, keeping the synthetic atlas as fallback.
//!
//! Opens the real menu entry twice under a window: once over the Steel
//! corpus (real `conchars` through installed mounts) and once over an
//! empty corpus (synthetic fallback). Proves the Steel capture differs
//! from the fallback capture at the glyph level while both keep legible
//! button and title text.

use std::path::PathBuf;

use qa_app::bootstrap::startup::StartupEntry;
use qa_app::bootstrap::windowed::open_windowed_application;
use qa_app::bootstrap::windowed_menu_text::resolve_menu_charset;
use qa_app::options::ApplicationOptions;
use qa_content::catalog::discover_installed_content;
use qa_content::catalog::DiscoverContentOptions;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

const CORPUS_WITNESSES: [&str; 3] = ["q1/id1/pak0.pak", "q2/baseq2/pak0.pak", "q3a/baseq3/pak0.pk3"];

fn find_steel_corpus() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .map(|dir| dir.join("target"))
        .find(|root| CORPUS_WITNESSES.iter().all(|witness| root.join(witness).is_file()))
}

fn menu_options(corpus_root: &str) -> ApplicationOptions {
    ApplicationOptions {
        windowed: true,
        width: WIDTH,
        height: HEIGHT,
        frame_limit: Some(3),
        corpus_root: corpus_root.to_string(),
        ..ApplicationOptions::default()
    }
}

fn capture_menu(corpus_root: &str) -> Vec<u8> {
    let options = menu_options(corpus_root);
    let mut composed = open_windowed_application(&options, StartupEntry::Menu)
        .unwrap_or_else(|error| panic!("menu entry opens over {corpus_root}: {error}"));
    composed.app.step().expect("menu step works");
    let pixels = composed.app.capture_next_frame().expect("menu capture works");
    assert!(!composed.app.active_game(), "menu entry has no active game");
    composed.app.close().expect("close works");
    assert_eq!(pixels.len(), (WIDTH * HEIGHT * 4) as usize);
    pixels
}

fn column_background(pixels: &[u8], x: u32, y0: u32, y1: u32) -> [u8; 3] {
    let top = y0.saturating_sub(8);
    let bottom = (y1 + 8).min(HEIGHT);
    let mut channels = [Vec::new(), Vec::new(), Vec::new()];
    for y in top..bottom {
        let at = ((y * WIDTH + x) * 4) as usize;
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

fn band_runs(pixels: &[u8], band: (u32, u32, u32, u32)) -> (usize, f64) {
    let (x0, y0, x1, y1) = band;
    let mut runs = 0;
    let mut in_run = false;
    let mut empty = 0;
    let mut ink = 0usize;
    for x in x0..x1.min(WIDTH) {
        let background = column_background(pixels, x, y0, y1.min(HEIGHT));
        let mut column_ink = 0;
        for y in y0..y1.min(HEIGHT) {
            let at = ((y * WIDTH + x) * 4) as usize;
            let distance = (0..3)
                .map(|channel| pixels[at + channel].abs_diff(background[channel]))
                .max()
                .unwrap_or(0);
            if distance > 40 {
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
fn menu_uses_real_charset_with_steel_and_falls_back_without_content() {
    let Some(steel) = find_steel_corpus() else {
        eprintln!("skipped: Steel corpus not found above {}", env!("CARGO_MANIFEST_DIR"));
        return;
    };
    let steel_root = steel.to_string_lossy().into_owned();
    let steel_catalog = discover_installed_content(&DiscoverContentOptions::new(steel.clone())).expect("steel catalog");
    let steel_charset = resolve_menu_charset(&steel_catalog, Some("q2-classic-baseq2"))
        .or_else(|| resolve_menu_charset(&steel_catalog, None));
    let charset = steel_charset.expect("steel resolves a real charset");
    assert!(charset.width.is_multiple_of(16) && charset.height.is_multiple_of(16));
    assert_eq!(
        charset.pixels.len(),
        charset.width as usize * charset.height as usize * 4
    );

    let empty_dir = std::env::temp_dir().join(format!("qa-wu23-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&empty_dir);
    std::fs::create_dir_all(&empty_dir).expect("empty corpus");
    let empty_root = empty_dir.to_string_lossy().into_owned();
    let empty_catalog =
        discover_installed_content(&DiscoverContentOptions::new(empty_dir.clone())).expect("empty catalog");
    assert!(
        resolve_menu_charset(&empty_catalog, None).is_none(),
        "no content resolves no charset"
    );

    let steel_pixels = capture_menu(&steel_root);
    let fallback_pixels = capture_menu(&empty_root);

    let temporary = std::env::temp_dir();
    let steel_path = temporary.join("qa-wu23-menu-steel.ppm");
    let fallback_path = temporary.join("qa-wu23-menu-fallback.ppm");
    write_ppm(&steel_path, &steel_pixels);
    write_ppm(&fallback_path, &fallback_pixels);
    eprintln!("steel menu screenshot: {}", steel_path.display());
    eprintln!("fallback menu screenshot: {}", fallback_path.display());
    eprintln!(
        "steel charset: {} {}x{} {}",
        charset.product, charset.width, charset.height, charset.source
    );

    let differing = steel_pixels
        .iter()
        .zip(fallback_pixels.iter())
        .filter(|(steel, fallback)| steel != fallback)
        .count();
    let ratio = differing as f64 / steel_pixels.len() as f64;
    assert!(
        ratio > 0.001,
        "real charset capture must pixel-differ from synthetic fallback, got {ratio:.4}"
    );

    for (pixels, label) in [(&steel_pixels, "steel"), (&fallback_pixels, "fallback")] {
        let (title_runs, title_ink) = band_runs(pixels, (64, 44, 304, 92));
        assert!(
            title_runs >= 4,
            "{label} title holds glyph clusters, got {title_runs} runs"
        );
        assert!(
            (0.01..0.60).contains(&title_ink),
            "{label} title ink is partial, got {title_ink:.3}"
        );
        for (band, name, minimum) in [
            ((74, 124, 307, 145), "Play a game", 6),
            ((74, 158, 267, 179), "Load Game", 5),
            ((74, 192, 225, 213), "Options", 4),
            ((74, 226, 163, 247), "Quit", 3),
        ] {
            let (runs, ink) = band_runs(pixels, band);
            assert!(runs >= minimum, "{label} {name} holds glyph clusters, got {runs} runs");
            assert!(
                (0.01..0.60).contains(&ink),
                "{label} {name} ink is partial, got {ink:.3}"
            );
        }
    }

    std::fs::remove_dir_all(&empty_dir).expect("cleanup");
}
