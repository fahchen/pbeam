//! Unit tests of the `+S` flag parse. The expected values come from
//! `erl +S <Value>` of the pinned ERTS on a host with 10 CPUs
//! (`erl_init.c:1103-1149`); a unit test fixes the host counts, which a
//! fixture cannot do.

use super::Schedulers;

/// The counts of a host with 10 CPUs.
fn ten() -> Schedulers {
    Schedulers {
        host_total: 10,
        host_online: 10,
        total: 10,
        online: 10,
    }
}

/// Apply `value` to the counts of [`ten`] and return the total and the
/// online count, or the error text.
fn apply(value: &str) -> Result<(usize, usize), String> {
    let mut schedulers = ten();
    schedulers.apply_flag(value.as_bytes())?;
    Ok((schedulers.total(), schedulers.online()))
}

#[test]
fn applies_the_s_flag_as_pinned_erts() {
    for (value, expected) in [
        ("2", (2, 2)),
        ("4:2", (4, 2)),
        (":1", (10, 1)),
        ("-1:1", (9, 1)),
        ("+2:-8", (2, 2)),
        ("0:0", (10, 10)),
        // `strtol` saturates and the `int` store truncates: `LONG_MAX`
        // gives -1, and `LONG_MIN` gives 0.
        ("99999999999999999999", (9, 9)),
        ("-99999999999999999999", (10, 10)),
    ] {
        assert_eq!(apply(value), Ok(expected), "{value}");
    }
}

#[test]
fn rejects_a_bad_s_flag_as_pinned_erts() {
    for (value, expected) in [
        ("x", "bad amount of schedulers x"),
        ("2000", "bad amount of schedulers 2000"),
        (
            "3:5",
            "bad amount of schedulers online 5 (total amount of schedulers 3)",
        ),
    ] {
        assert_eq!(apply(value), Err(expected.to_owned()), "{value}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn counts_the_allowed_processors_online_as_pinned_erts() {
    let status = "Name:\tbeam\nCpus_allowed_list:\t0-3,6,8-9\n";
    // The mask counts only the processors online (`sched_getaffinity`).
    assert_eq!(super::available_of(status, "0-7\n", 1), 5);
    assert_eq!(super::available_of(status, "0-9", 1), 7);
    // No processor of the mask online gives the count online
    // (`erl_misc_utils.c:418-419`).
    assert_eq!(super::available_of(status, "4-5", 2), 2);
    assert_eq!(super::available_of("Name:\tbeam\n", "0-9", 3), 3);
}

/// Write the sysfs files `files` (path, text) below a new directory and
/// return the directory.
#[cfg(target_os = "linux")]
fn sysfs(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("pbeam-sysfs-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    for (path, text) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    root
}

#[cfg(target_os = "linux")]
#[test]
fn reads_the_cpu_topology_as_pinned_erts() {
    // erl_misc_utils.c:841-1031: no node directory; the two threads of core
    // 0 get thread ids 0 and 1; a processor without a core id or with a bad
    // package id is left out.
    let root = sysfs(
        "flat",
        &[
            ("cpu/cpu1/topology/physical_package_id", "0\n"),
            ("cpu/cpu1/topology/core_id", "0\n"),
            ("cpu/cpu0/topology/physical_package_id", "0\n"),
            ("cpu/cpu0/topology/core_id", "0\n"),
            ("cpu/cpu2/topology/physical_package_id", "0\n"),
            ("cpu/cpu2/topology/core_id", "1\n"),
            ("cpu/cpu3/topology/physical_package_id", "0\n"),
            ("cpu/cpu4/topology/physical_package_id", "x\n"),
            ("cpu/cpu4/topology/core_id", "2\n"),
            ("cpu/cpufreq/x", ""),
        ],
    );
    let expected = vec![
        [-1, 0, -1, 0, 0, 0],
        [-1, 0, -1, 0, 1, 1],
        [-1, 0, -1, 1, 0, 2],
    ];
    assert_eq!(super::read_topology(&root, 8, 3), expected);
    // More processors than configured (erl_misc_utils.c:947-948), and fewer
    // than online (erl_misc_utils.c:969-970).
    assert_eq!(super::read_topology(&root, 2, 1), <[[i32; 6]; 0]>::default());
    assert_eq!(super::read_topology(&root, 8, 4), <[[i32; 6]; 0]>::default());
    // No cpu directory (erl_misc_utils.c:908-910).
    let empty = sysfs("empty", &[("other", "")]);
    assert_eq!(super::read_topology(&empty, 8, 0), <[[i32; 6]; 0]>::default());
}

#[cfg(target_os = "linux")]
#[test]
fn moves_a_processor_on_two_nodes_to_processor_nodes_as_pinned_erts() {
    // erl_misc_utils.c:750-797: processor 0 spans nodes 0 and 1, and
    // processor 1 is on node 1 only.
    let root = sysfs(
        "nodes",
        &[
            ("node/node0/cpu0/topology/physical_package_id", "0"),
            ("node/node0/cpu0/topology/core_id", "0"),
            ("node/node1/cpu1/topology/physical_package_id", "0"),
            ("node/node1/cpu1/topology/core_id", "1"),
            ("node/node1/cpu2/topology/physical_package_id", "1"),
            ("node/node1/cpu2/topology/core_id", "0"),
            ("node/has_cpu", "0-2"),
        ],
    );
    let expected = vec![
        [-1, 0, 0, 0, 0, 0],
        [-1, 0, 1, 1, 0, 1],
        [1, 1, -1, 0, 0, 2],
    ];
    assert_eq!(super::read_topology(&root, 8, 3), expected);
    // verify_topology keeps the topology in processor order
    // (erl_cpu_topology.c:1210).
    let ordered = vec![
        [-1, 0, 0, 0, 0, 0],
        [-1, 0, 1, 1, 0, 1],
        [1, 1, -1, 0, 0, 2],
    ];
    assert_eq!(super::processor_order(expected), ordered);
    let swapped = vec![[0, 1, -1, 0, 0, 0], [1, 0, -1, 0, 0, 1]];
    assert_eq!(
        super::processor_order(swapped),
        vec![[1, 0, -1, 0, 0, 1], [0, 1, -1, 0, 0, 0]]
    );
    // A node directory that cannot be read gives no topology
    // (erl_misc_utils.c:908-910).
    let broken = sysfs("broken", &[("node/node0", "")]);
    assert_eq!(super::read_topology(&broken, 8, 0), <[[i32; 6]; 0]>::default());
}
