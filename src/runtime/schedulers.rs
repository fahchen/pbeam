//! The scheduler counts of a node: the default of the host and the `+S`
//! emulator flag, as the pinned `erl_init.c` sets them.

/// The most schedulers of a node (`ERTS_MAX_NO_OF_SCHEDULERS`,
/// `erl_process.h:82`).
const MAX_SCHEDULERS: i32 = 1024;

/// The number of schedulers and the number of schedulers online of a node.
///
/// ERTS starts one scheduler for each configured logical processor, and
/// sets one scheduler online for each processor that the process can use
/// (`erl_init.c:848-860`). PBeam reads one count, the processors that the
/// process can use (`std::thread::available_parallelism`), for both (row
/// "Scheduling" of `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let mut schedulers = Schedulers::host();
/// schedulers.apply_flag(b"4:2")?;
/// assert_eq!((schedulers.total(), schedulers.online()), (4, 2));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedulers {
    /// The default count of schedulers (`no_schedulers` before the flags).
    host_total: i32,
    /// The default count of schedulers online.
    host_online: i32,
    /// The count of schedulers (`schdlrs`).
    total: i32,
    /// The count of schedulers online (`schdlrs_onln`).
    online: i32,
}

impl Schedulers {
    /// Return the default counts of the host.
    pub fn host() -> Self {
        let count = std::thread::available_parallelism().map_or(1, |count| {
            i32::try_from(count.get()).unwrap_or(MAX_SCHEDULERS)
        });
        Schedulers {
            host_total: count,
            host_online: count,
            total: count,
            online: count,
        }
    }

    /// Return the count of schedulers, which `erlang:system_info(schedulers)`
    /// gives.
    pub fn total(&self) -> usize {
        usize::try_from(self.total).expect("a checked count is positive")
    }

    /// Return the count of schedulers online, which
    /// `erlang:system_info(schedulers_online)` gives. The node starts one
    /// scheduler thread for each.
    pub fn online(&self) -> usize {
        usize::try_from(self.online).expect("a checked count is positive")
    }

    /// Apply the value `argument` of a `+S` flag, as `erl_init.c:1103-1149`
    /// does: `Total:Online`, `Total`, or `:Online`. A value of 0 or less
    /// counts down from the default of the host. A later flag starts from
    /// the counts of the earlier flags.
    ///
    /// # Errors
    ///
    /// The text that ERTS writes before its usage text: `bad amount of
    /// schedulers <Argument>` for a value that does not parse, `bad amount
    /// of schedulers <Total>` for a total outside 1 to 1024, and `bad amount
    /// of schedulers online <Online> (total amount of schedulers <Total>)`
    /// for an online count outside 1 to the total.
    pub fn apply_flag(&mut self, argument: &[u8]) -> Result<(), String> {
        let (total, online) = match scan_int(argument) {
            // `sscanf(arg, "%d:%d")` gives 2, or 1 with the default online
            // count (`erl_init.c:1115-1117`).
            Some((total, rest)) => match rest.strip_prefix(b":").and_then(scan_int) {
                Some((online, _)) => (total, online),
                None => (total, total.min(self.online)),
            },
            // `sscanf(arg, ":%d")` (`erl_init.c:1107-1114`).
            None => match argument.strip_prefix(b":").and_then(scan_int) {
                Some((online, _)) => (self.host_total, online),
                None => {
                    return Err(format!(
                        "bad amount of schedulers {}",
                        String::from_utf8_lossy(argument)
                    ));
                }
            },
        };
        let count = if total > 0 {
            total
        } else {
            self.host_total.wrapping_add(total)
        };
        let count_online = if online > 0 {
            online
        } else {
            self.host_online.wrapping_add(online)
        };
        if !(1..=MAX_SCHEDULERS).contains(&count) {
            return Err(format!("bad amount of schedulers {total}"));
        }
        if count_online < 1 || count < count_online {
            return Err(format!(
                "bad amount of schedulers online {count_online} (total amount of schedulers {count})"
            ));
        }
        self.total = count;
        self.online = count_online;
        Ok(())
    }
}

/// Return the count of logical processors that the process can use, as
/// `erts_cpu_info_update` (`erl_misc_utils.c:338-419`) counts them: the
/// processors of the affinity mask of the process, at most the processors
/// online. On Linux PBeam reads the mask from `Cpus_allowed_list` of
/// `/proc/self/status` and the processors online from
/// `/sys/devices/system/cpu/online`, and counts the processors in both, as
/// `sched_getaffinity` gives only the active processors of the mask.
#[cfg(target_os = "linux")]
pub(in crate::runtime) fn logical_processors_available() -> usize {
    available_of(
        &std::fs::read_to_string("/proc/self/status").unwrap_or_default(),
        &std::fs::read_to_string("/sys/devices/system/cpu/online").unwrap_or_default(),
        Schedulers::host().total(),
    )
}

/// Count the processors of `Cpus_allowed_list` in the text `status` of
/// `/proc/self/status` that are also in the CPU list `online`. No such
/// processor gives `fallback`, the count online (`erl_misc_utils.c:418-419`).
#[cfg(target_os = "linux")]
fn available_of(status: &str, online: &str, fallback: usize) -> usize {
    let allowed = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .map(cpu_list)
        .unwrap_or_default();
    let online = cpu_list(online);
    match allowed.iter().filter(|cpu| online.contains(cpu)).count() {
        0 => fallback,
        available => available,
    }
}

/// Return the count of logical processors that the process can use. A
/// host without an affinity mask gives the processors online
/// (`erl_misc_utils.c:418-419`), which `available_parallelism` reads with
/// `sysconf(_SC_NPROCESSORS_ONLN)`, as ERTS does
/// (`erl_misc_utils.c:302-306`).
#[cfg(not(target_os = "linux"))]
pub(in crate::runtime) fn logical_processors_available() -> usize {
    Schedulers::host().total()
}

/// Return the processors of a Linux CPU list such as `0-3,6`.
#[cfg(target_os = "linux")]
fn cpu_list(text: &str) -> Vec<u32> {
    text.trim()
        .split(',')
        .filter_map(|range| match range.split_once('-') {
            Some((first, last)) => Some(first.parse().ok()?..=last.parse().ok()?),
            None => range.parse().ok().map(|cpu| cpu..=cpu),
        })
        .flatten()
        .collect()
}

/// Scan one `%d` conversion of `sscanf` at the start of `text`: white space,
/// an optional sign, and at least one digit. Return the value and the rest
/// of `text`. The C library reads the digits as a `long`, which saturates,
/// and stores the `long` in an `int`, which truncates it.
fn scan_int(text: &[u8]) -> Option<(i32, &[u8])> {
    let text = text.trim_ascii_start();
    let (negative, digits) = match text.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, text),
    };
    let end = digits
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let value = digits[..end].iter().fold(0i64, |value, digit| {
        let digit = i64::from(digit - b'0');
        if negative {
            value.saturating_mul(10).saturating_sub(digit)
        } else {
            value.saturating_mul(10).saturating_add(digit)
        }
    });
    Some((value as i32, &digits[end..]))
}

/// The ids of one logical processor that ERTS reads from the processor
/// topology of the host: node, processor, processor node, core, thread, and
/// logical id (`erts_cpu_topology_t`, `erl_misc_utils.h`).
#[cfg(target_os = "linux")]
pub(in crate::runtime) type CpuIds = [i32; 6];

/// Return the processor topology of the host, as `read_topology`
/// (`erl_misc_utils.c:841-1031`) reads it from `/sys/devices/system`. An
/// empty list means that ERTS reads no topology. PBeam reads it once, when
/// `BeamProgram::boot` starts the node, and keeps it, as ERTS keeps the
/// topology that it reads when the node starts (`erl_cpu_topology.c:1686-1703`). ERTS takes the configured
/// processors from `sysconf(_SC_NPROCESSORS_CONF)`, which the C library
/// counts from the `cpuN` entries of `/sys/devices/system/cpu`, and the
/// processors online from `/sys/devices/system/cpu/online`.
#[cfg(target_os = "linux")]
pub(in crate::runtime) fn cpu_topology() -> &'static [CpuIds] {
    static TOPOLOGY: std::sync::OnceLock<Vec<CpuIds>> = std::sync::OnceLock::new();
    TOPOLOGY.get_or_init(read_host_topology)
}

/// Read the processor topology of the host (see [`cpu_topology`]).
#[cfg(target_os = "linux")]
fn read_host_topology() -> Vec<CpuIds> {
    let root = std::path::Path::new("/sys/devices/system");
    let configured = numbered_entries(&root.join("cpu"), "cpu").map_or(0, |cpus| cpus.len());
    let online = cpu_list(&std::fs::read_to_string(root.join("cpu/online")).unwrap_or_default());
    processor_order(read_topology(root, configured, online.len()))
}

/// Sort `topology` by processor, node, processor node, core, and thread, as
/// `verify_topology` sorts the topology that ERTS keeps
/// (`processor_order_cmp`, `erl_cpu_topology.c:424-441`,
/// `erl_cpu_topology.c:1210`). The entities of a topology that
/// `verify_topology` accepts are unique, so the order is total.
#[cfg(target_os = "linux")]
fn processor_order(mut topology: Vec<CpuIds>) -> Vec<CpuIds> {
    topology.sort_unstable_by_key(|&[node, processor, processor_node, core, thread, _]| {
        (processor, node, processor_node, core, thread)
    });
    topology
}

/// Read the processor topology below `root` as `read_topology`
/// (`erl_misc_utils.c:841-1031`) reads it below `/sys/devices/system`.
/// Each `nodeN` directory of `root/node` gives the node `N` of its `cpuN`
/// entries; with no `root/node` directory, each entry of `root/cpu` has
/// node -1. A processor without a readable `physical_package_id` or
/// `core_id` is left out (`erl_misc_utils.c:921-939`). More processors than
/// `configured`, no processor, or fewer processors than `online` give no
/// topology (`erl_misc_utils.c:947-948`, `erl_misc_utils.c:969-970`).
#[cfg(target_os = "linux")]
fn read_topology(root: &std::path::Path, configured: usize, online: usize) -> Vec<CpuIds> {
    let directories: Vec<(i32, std::path::PathBuf)> =
        match numbered_entries(&root.join("node"), "node") {
            Some(nodes) => nodes
                .into_iter()
                .map(|node| (node, root.join(format!("node/node{node}"))))
                .collect(),
            None => vec![(-1, root.join("cpu"))],
        };
    let mut topology = Vec::new();
    for (node, directory) in &directories {
        let Some(cpus) = numbered_entries(directory, "cpu") else {
            return Vec::new();
        };
        for cpu in cpus {
            let id = |name: &str| {
                let path = directory.join(format!("cpu{cpu}/topology/{name}"));
                scan_int(&std::fs::read(path).ok()?).map(|(id, _)| id)
            };
            let (Some(processor), Some(core)) = (id("physical_package_id"), id("core_id")) else {
                continue;
            };
            if topology.len() >= configured {
                return Vec::new();
            }
            topology.push([*node, processor, -1, core, 0, cpu]);
        }
    }
    if topology.is_empty() || topology.len() < online {
        return Vec::new();
    }
    if directories.len() > 1 {
        adjust_processor_nodes(&mut topology);
    }
    // `cpu_cmp` (`erl_misc_utils.c:729-748`) orders the ids in their order.
    topology.sort_unstable();
    for index in 1..topology.len() {
        let [previous, this] = [topology[index - 1], topology[index]];
        if previous[..4] == this[..4] {
            topology[index][4] = previous[4] + 1;
        }
    }
    topology
}

/// Move the node of each processor that spans more than one node to its
/// processor node, and set its node to -1 (`adjust_processor_nodes`,
/// `erl_misc_utils.c:750-797`).
#[cfg(target_os = "linux")]
fn adjust_processor_nodes(topology: &mut [CpuIds]) {
    // `pn_cmp` (`erl_misc_utils.c:708-727`) orders by processor first.
    topology.sort_unstable_by_key(|[node, processor, rest @ ..]| (*processor, *node, *rest));
    for group in topology.chunk_by_mut(|left, right| left[1] == right[1]) {
        let node = group[0][0];
        if group.iter().any(|cpu| cpu[0] != node) {
            for cpu in group {
                cpu[2] = cpu[0];
                cpu[0] = -1;
            }
        }
    }
}

/// Return the numbers `N` of the entries `<prefix>N` of `directory`, as
/// `sscanf(name, "<prefix>%d")` reads them, or `None` when the directory
/// cannot be read.
#[cfg(target_os = "linux")]
fn numbered_entries(directory: &std::path::Path, prefix: &str) -> Option<Vec<i32>> {
    let entries = std::fs::read_dir(directory).ok()?;
    Some(
        entries
            .filter_map(|entry| {
                let name = entry.ok()?.file_name();
                let rest = name.as_encoded_bytes().strip_prefix(prefix.as_bytes())?;
                scan_int(rest).map(|(number, _)| number)
            })
            .collect(),
    )
}

#[cfg(test)]
#[path = "../../tests/runtime/schedulers.rs"]
mod tests;
