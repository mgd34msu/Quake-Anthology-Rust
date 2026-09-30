//! Product inventory binary (donor `tools/inventory/products.ts`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match qa_tools::inventory::products::run(&args) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
