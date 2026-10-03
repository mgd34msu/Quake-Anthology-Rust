//! `qa-dedicated`: dedicated-server entry point.
//!
//! Forwards to the same [`qa_app::cli`] dispatch as `quake-anthology`, injecting
//! `--dedicated` when the caller did not pass it. Refuses `--menu`, which
//! requires a local non-dedicated application.

fn main() {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|arg| arg == "--menu") {
        eprintln!("qa-dedicated: --menu requires a local, non-dedicated application");
        std::process::exit(1);
    }
    if !argv.iter().any(|arg| arg == "--dedicated") {
        argv.insert(0, "--dedicated".to_string());
    }
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    std::process::exit(qa_app::cli::run(
        &argv,
        &mut stdout,
        &mut stderr,
        env!("CARGO_PKG_VERSION"),
    ));
}
