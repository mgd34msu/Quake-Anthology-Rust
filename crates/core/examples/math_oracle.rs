#[path = "../../../tools/probes/allocation_counter.rs"]
mod allocation_counter;
use qa_core::{math::*, primitives::Vec3};

fn floats(path: &str) -> Result<Vec<f32>, &'static str> {
    let bytes = std::fs::read(path).map_err(|_| "read oracle")?;
    if !bytes.len().is_multiple_of(4) {
        return Err("oracle size");
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .collect())
}

fn main() -> Result<(), &'static str> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 {
        return Err("input and three C reference outputs required");
    }
    let cases = floats(&args[1])?;
    let references = [floats(&args[2])?, floats(&args[3])?, floats(&args[4])?];
    if !cases.len().is_multiple_of(7) || references.iter().any(|r| r.len() != cases.len() / 7 * 14)
    {
        return Err("oracle case counts");
    }
    let mut matches = 0;
    allocation_counter::start();
    for (profile, reference) in references.iter().enumerate() {
        for (case, expected) in cases
            .as_chunks::<7>()
            .0
            .iter()
            .zip(reference.as_chunks::<14>().0.iter())
        {
            let angles = Vec3([case[0], case[1], case[2]]);
            let basis = if profile == 2 {
                angle_vectors_radians(radians_from_degrees_f32(angles))
            } else {
                angle_vectors(angles)
            };
            let mut vector = Vec3([case[3], case[4], case[5]]);
            let len = normalize(&mut vector);
            let actual = [basis.forward.0, basis.right.0, basis.up.0, vector.0]
                .into_iter()
                .flatten()
                .chain([len, anglemod(case[6])]);
            for (a, &b) in actual.zip(expected) {
                if a.to_bits() != b.to_bits() {
                    allocation_counter::stop();
                    println!(
                        "mismatch profile={profile} case={} actual={a:?} expected={b:?} actual_bits={} expected_bits={}",
                        matches % (cases.len() / 7),
                        a.to_bits(),
                        b.to_bits()
                    );
                    return Err("original C math differs");
                }
            }
            matches += 1;
        }
    }
    let allocations = allocation_counter::stop();
    println!(
        "{{\"scope\":\"headless math comparison with three original C sources\",\"profiles\":3,\"cases_per_profile\":{},\"bit_exact_cases\":{matches},\"allocations_after_load\":{allocations}}}",
        cases.len() / 7
    );
    if allocations == 0 {
        Ok(())
    } else {
        Err("math allocated")
    }
}
