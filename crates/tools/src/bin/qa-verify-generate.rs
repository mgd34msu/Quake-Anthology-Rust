//! Composition manifest generator binary (donor `tools/verify/generate.ts`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match qa_tools::verify::generate::run(&args) {
        Ok(line) => println!("{line}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
