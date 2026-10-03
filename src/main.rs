//! `quake-anthology` command line, ported from `src/main.ts`.
//!
//! All parsing and dispatch lives in [`qa_app::cli`]; this binary only
//! forwards process arguments and stdio.

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    std::process::exit(qa_app::cli::run(
        &argv,
        &mut stdout,
        &mut stderr,
        env!("CARGO_PKG_VERSION"),
    ));
}
