//! Source camera binary (donor `tools/source-camera.ts`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(error) = qa_tools::source_camera::camera_tool(&args) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
