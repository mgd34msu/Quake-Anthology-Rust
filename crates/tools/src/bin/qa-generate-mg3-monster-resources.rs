//! MG3 monster resource generator (donor `tools/content/generate-mg3-monster-resources.ts`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match qa_tools::content::generate_mg3_monster_resources::run(&args) {
        Ok(path) => println!("{}", path.display()),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
