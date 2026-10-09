//! Load-time CPU topology, restricted to the process's allowed CPU mask.
use std::collections::BTreeSet;

/// Physical cores available to this process, rather than SMT thread count.
/// Unknown topology falls back to Rust's affinity-aware parallelism count.
pub fn physical_core_count() -> usize {
    let fallback = std::thread::available_parallelism().map_or(1, usize::from);
    #[cfg(target_os = "linux")]
    {
        linux_physical_cores().unwrap_or(fallback)
    }
    #[cfg(not(target_os = "linux"))]
    {
        fallback
    }
}

#[cfg(target_os = "linux")]
fn linux_physical_cores() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let allowed = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))?;
    let cpus = cpu_list(allowed)?;
    let mut cores = BTreeSet::new();
    for cpu in cpus {
        let root = format!("/sys/devices/system/cpu/cpu{cpu}/topology");
        let package: i32 = std::fs::read_to_string(format!("{root}/physical_package_id"))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        let core: i32 = std::fs::read_to_string(format!("{root}/core_id"))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        if package < 0 || core < 0 {
            return None;
        }
        cores.insert((package, core));
    }
    (!cores.is_empty()).then_some(cores.len())
}

#[cfg(target_os = "linux")]
fn cpu_list(text: &str) -> Option<Vec<usize>> {
    let mut cpus = Vec::new();
    for range in text.trim().split(',') {
        let (first, last) = range.split_once('-').unwrap_or((range, range));
        let first: usize = first.parse().ok()?;
        let last: usize = last.parse().ok()?;
        // Linux's nr_cpu_ids is bounded. Reject malformed ranges before any
        // allocation, including a usize::MAX end that cannot be iterated.
        if last < first || last >= 1_048_576 {
            return None;
        }
        cpus.extend(first..=last);
    }
    Some(cpus)
}

#[cfg(test)]
#[path = "../tests/cpu/topology.rs"]
mod tests;
