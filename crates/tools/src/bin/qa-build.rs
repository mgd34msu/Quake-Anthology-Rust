//! Workspace build binary (donor `tools/build.ts`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let run = (|| -> Result<(), qa_tools::error::ToolsError> {
        let kind = qa_tools::build::parse_build_kind(&args)?;
        let cwd = std::env::current_dir()
            .map_err(|error| qa_tools::error::ToolsError::io("resolving current directory", error))?;
        qa_tools::build::build_workspace(&cwd, kind)?;
        Ok(())
    })();
    if let Err(error) = run {
        eprintln!("Build failed: {error}");
        std::process::exit(1);
    }
}
