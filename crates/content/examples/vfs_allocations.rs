#[path = "../../../tools/probes/allocation_counter.rs"]
mod allocation_counter;
use qa_content::vfs::Vfs;

fn main() -> Result<(), &'static str> {
    let root = std::env::temp_dir().join(format!("qa-rust-vfs-probe-{}", std::process::id()));
    std::fs::create_dir_all(root.join("maps")).map_err(|_| "create fixture")?;
    std::fs::write(root.join("maps/fixture.bsp"), [42; 256]).map_err(|_| "write fixture")?;
    let mut vfs = Vfs::default();
    vfs.mount_directory(&root, 0).map_err(|_| "mount")?;
    let reference = vfs.open(b"MAPS\\FIXTURE.BSP").ok_or("resolve")?;
    if vfs.take_lookup_count() != 1 {
        return Err("load lookup counter");
    }
    let mut buffer = [0; 256];
    allocation_counter::start();
    for _ in 0..10_000 {
        let bytes = vfs
            .read_at(reference, 0, std::hint::black_box(&mut buffer))
            .map_err(|_| "read")?;
        if bytes != 256 || buffer != [42; 256] {
            return Err("fixture bytes differ");
        }
    }
    let allocations = allocation_counter::stop();
    let lookups = vfs.take_lookup_count();
    drop(vfs);
    std::fs::remove_dir_all(root).map_err(|_| "remove own fixture")?;
    println!(
        "{{\"scope\":\"headless resolved VFS reads, not gameplay\",\"reads\":10000,\"path_lookups\":{lookups},\"allocations_after_load\":{allocations}}}"
    );
    if allocations == 0 && lookups == 0 {
        Ok(())
    } else {
        Err("VFS hot read allocation or lookup")
    }
}
