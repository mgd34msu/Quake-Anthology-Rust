//! Feature ledger aggregation binary (donor `tools/inventory/aggregate-features.ts`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(error) = qa_tools::inventory::aggregate_features::run(&args) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
