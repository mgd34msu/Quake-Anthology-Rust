use qa_app::renderer::cpu_band_setting;
use qa_console::cvars::Cvars;
use qa_render::cpu::RasterBands;

#[test]
fn benchmark_override_wins_and_zero_requests_auto() {
    let mut cvars = Cvars::new().unwrap();
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

#[test]
fn pending_selection_is_read_only_and_override_does_not_validate_unused_config() {
    use qa_app::renderer::pending_cpu_band_setting;
    use qa_console::views::Context;
    let mut cvars = Cvars::new().unwrap();
    let view = cvars.bind("r_cpuBands", Context::default()).unwrap();
    let handle = view.canonical();
    cvars.set_latch_active(handle, true);
    cvars.write(view, "4").unwrap();
    let revision = cvars.revision();
    let pending_generation = cvars.view_update_generation(view);
    assert_eq!(cvars.integer(handle), 0);
    assert_eq!(
        pending_cpu_band_setting(&cvars, view, None).unwrap(),
        Some(RasterBands::Four)
    );
    assert_eq!(cpu_band_setting(&cvars, None).unwrap().1, None);
    assert_eq!(cvars.revision(), revision);
    assert_eq!(cvars.view_update_generation(view), pending_generation);
    assert_eq!(cvars.latched(view).unwrap().unwrap().as_str(), "4");

    cvars.write(view, "3").unwrap();
    assert!(pending_cpu_band_setting(&cvars, view, None).is_err());
    assert_eq!(
        pending_cpu_band_setting(&cvars, view, Some(RasterBands::Eight)).unwrap(),
        Some(RasterBands::Eight)
    );
    assert_eq!(cvars.integer(handle), 0);
    assert_eq!(cvars.latched(view).unwrap().unwrap().as_str(), "3");
}
