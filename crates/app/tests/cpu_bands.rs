use qa_app::renderer::cpu_band_setting;
use qa_console::cvars::Cvars;
use qa_render::cpu::RasterBands;

#[test]
fn benchmark_override_wins_and_zero_requests_auto() {
    let mut cvars = Cvars::new();
    let handle = cvars.find("r_cpuBands").unwrap();
    assert_eq!(cpu_band_setting(&cvars, None).unwrap(), (handle, None));
    cvars.set(handle, 4.0).unwrap();
    assert_eq!(
        cpu_band_setting(&cvars, None).unwrap().1,
        Some(RasterBands::Four)
    );
    assert_eq!(
        cpu_band_setting(&cvars, Some(RasterBands::Two)).unwrap().1,
        Some(RasterBands::Two)
    );
    cvars.set(handle, 3.0).unwrap();
    assert!(cpu_band_setting(&cvars, None).is_err());
    assert_eq!(
        cpu_band_setting(&cvars, Some(RasterBands::Eight))
            .unwrap()
            .1,
        Some(RasterBands::Eight)
    );
}
