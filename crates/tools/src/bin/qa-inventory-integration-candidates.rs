//! Integration candidate snapshot binary (donor `tools/inventory/integration-candidates.ts`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(error) = qa_tools::inventory::integration_candidates::run(&args) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
