//! Verification runner binary (donor `tools/verify/cli.ts`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match qa_tools::verify::cli::run(&args) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
