//! Boot the node with the pinned `init` and a fixed boot script, and compare
//! the result with the pinned ERTS.

use std::{
    env,
    os::unix::{ffi::OsStrExt, process::ExitStatusExt},
    process::{Command, Stdio},
};

use pbeam_runtime::{BeamError, BeamProgram, Schedulers, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

/// The boot file of the probe, without its `.boot` extension, as the
/// `-boot` flag of `erl` takes it (init.erl:1256-1267).
const BOOT_FILE: &str = "fixtures/boot/init_boot_probe";
const PROBE: &[u8] = include_bytes!("../../fixtures/boot/init_boot_probe.beam");
/// The boot file of the probe that kills `init`.
const KILL_BOOT_FILE: &str = "fixtures/boot/init_kill_probe";
const KILL_PROBE: &[u8] = include_bytes!("../../fixtures/boot/init_kill_probe.beam");
const LOADED_ORDER_PROBE: &[u8] = include_bytes!("../../fixtures/boot/loaded_order_probe.beam");
const SYSTEM_PROCESS_PROBE: &[u8] = include_bytes!("../../fixtures/boot/system_process_probe.beam");

/// The environment variable that `boots_init_with_a_fixed_boot_script_as_pinned_erts`
/// and the kernel boot tests set for their child, so the boot runs in a
/// process of its own: the probe halts the node, and the kernel boot
/// starts `user_drv`, which sets signal dispositions of the process
/// (prim_tty.erl:284-298). For the kernel boot, the value is the `-s`
/// arguments of the probe.
const ISOLATED: &str = "PBEAM_INIT_BOOT_ISOLATED";

/// The runtime error text that `isolated_kernel_boot_error` expects.
const ISOLATED_ERROR: &str = "PBEAM_INIT_BOOT_ISOLATED_ERROR";

/// The pinned modules that the boot runs before the probe: `erl_init` and
/// the modules that `erl_init:start/2` and `init` call
/// (erl_init.erl:35-50; init.erl:586-596, 1164-1358).
const BOOT_MODULES: [&str; 11] = [
    "erl_init",
    "erts_code_purger",
    "init",
    "erl_prim_loader",
    "prim_file",
    "zlib",
    "erl_tracer",
    "prim_buffer",
    "erlang",
    "erts_internal",
    "lists",
];

/// The `root` and `bindir` flags that `erl` gives the pinned emulator, as
/// `init:get_argument/1` gives them.
fn emulator_flags() -> (Vec<u8>, Vec<u8>) {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            "{ok, [[R]]} = init:get_argument(root), {ok, [[B]]} = init:get_argument(bindir), \
             io:put_chars([R, $\\n, B]), halt().",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let (root, bindir) = output.stdout.split_at(
        output
            .stdout
            .iter()
            .position(|&byte| byte == b'\n')
            .unwrap(),
    );
    (root.to_vec(), bindir[1..].to_vec())
}

#[test]
fn boots_init_with_a_fixed_boot_script_as_pinned_erts() {
    // `erl -boot File` runs the boot script of `File.boot` in the boot
    // process of `init` (init.erl:1192-1217, 1256-1308). The probe records
    // the boot status of `init` at each step of the script, waits for the
    // exit of the boot process, and displays the registered name of `init`,
    // the steps, and the status `{started, started}` of the main loop of
    // `init` (init.erl:700-704). The names include `erts_code_purger`, which
    // the emulator starts after `init` (erl_init.c:2550-2557). It then halts
    // with status 0.
    let reference = Command::new("erl")
        .args(["-noshell", "-boot", BOOT_FILE, "-pa", "fixtures/boot"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let expected = "{boot_exit,normal,init,\"<0.0.0>\",{registered_name,init},\
        status,{started,started},script_id,{\"init_boot_probe\",\"1\"},\
        recorder_linked_to_init,true,\
        steps,[{first,{starting,init_kernel_started}},{second,{starting,applications_loaded}}],\
        names,[{init,true},{erl_prim_loader,true},{init__boot__on_load_handler,true},\
        {init_boot_probe_recorder,true},{erts_code_purger,true}],alive,true}\r\n";
    assert_eq!(
        (
            String::from_utf8_lossy(&reference.stdout).as_ref(),
            reference.stderr.as_slice(),
            reference.status.code()
        ),
        (expected, &b""[..], Some(0)),
        "ERTS"
    );

    let child = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "isolated_init_boot",
            "--include-ignored",
            "--quiet",
        ])
        .env(ISOLATED, "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&child.stdout);
    assert!(
        child.status.success(),
        "child exited with {}\n{stdout}",
        child.status
    );
    let lines = stdout
        .split_inclusive('\n')
        .filter(|line| line.starts_with("{boot_exit"))
        .collect::<Vec<_>>();
    assert_eq!(lines, [expected], "PBeam");
}

/// The child side of `boots_init_with_a_fixed_boot_script_as_pinned_erts`:
/// boot the node with the arguments that `erl` gives the emulator, and
/// expect the halt of the probe.
#[test]
#[ignore = "boots_init_with_a_fixed_boot_script_as_pinned_erts runs it in a child process"]
fn isolated_init_boot() {
    assert!(
        env::var_os(ISOLATED).is_some(),
        "run it through its parent test"
    );
    let halted = boot_probe(PROBE, BOOT_FILE);
    assert_eq!(halted.unwrap_err(), BeamError::Halt(0));
}

/// Boot the node with `probe` and the boot modules, with the arguments that
/// `erl -noshell -boot BootFile -pa fixtures/boot` gives the emulator.
fn boot_probe(probe: &[u8], boot_file: &str) -> Result<pbeam_runtime::HostTerm, BeamError> {
    boot_node(
        &[probe],
        &[b"-boot", boot_file.as_bytes(), b"-pa", b"fixtures/boot"],
    )
}

/// Boot the node with the boot modules and `probes`, with the arguments
/// that `erl -noshell Flags` gives the emulator.
fn boot_node(probes: &[&[u8]], flags: &[&[u8]]) -> Result<pbeam_runtime::HostTerm, BeamError> {
    let beams = BOOT_MODULES.map(pinned_beam);
    let mut slices = beams.iter().map(Vec::as_slice).collect::<Vec<_>>();
    slices.extend(probes);
    let program = BeamProgram::load(&slices).unwrap();
    let (root, bindir) = emulator_flags();
    let home = env::var_os("HOME").unwrap();
    let mut arguments: Vec<&[u8]> = vec![
        b"-root",
        &root,
        b"-bindir",
        &bindir,
        b"-progname",
        b"erl",
        b"--",
        b"-home",
        home.as_bytes(),
        b"--",
        b"-noshell",
    ];
    arguments.extend(flags);
    Err(program.boot(&arguments, Schedulers::host()))
}

/// The environment of the kernel boot: a home directory and a user
/// configuration directory that do not exist, so `c:erlangrc/0` finds no
/// `.erlang` file of the host user (c.erl:1051-1072).
const KERNEL_BOOT_ENVIRONMENT: [(&str, &str); 2] = [
    ("HOME", "fixtures/boot/no_home"),
    ("XDG_CONFIG_HOME", "fixtures/boot/no_home"),
];

/// The inetrc file `fixtures/boot/kernel_boot.inetrc`, whose domain keeps
/// `inet_config:init/0` from the host name lookup on Linux
/// (inet_config.erl:63-64, 110-133), so the output of the boot does not
/// depend on the host name.
const KERNEL_BOOT_INETRC: (&str, &str) = ("ERL_INETRC", "fixtures/boot/kernel_boot.inetrc");

/// The flags of `erl` for the kernel boot of `kernel_boot_probe`, with the
/// `-s` arguments `arguments`: the function of the probe and its
/// arguments, for example `main 3`.
fn kernel_boot_flags(arguments: &str) -> Vec<String> {
    let mut flags = [
        "-boot",
        "start_clean",
        "-pa",
        "fixtures/boot",
        "-s",
        "kernel_boot_probe",
    ]
    .map(String::from)
    .to_vec();
    flags.extend(arguments.split_whitespace().map(String::from));
    flags.extend(["-kernel", "logger_level", "warning"].map(String::from));
    flags
}

/// Run the kernel boot of `kernel_boot_probe` with the `-s` arguments
/// `arguments` (see [`kernel_boot_flags`]), the environment of the kernel
/// boot, and `ERL_INETRC` only when `inetrc` is true, on the pinned ERTS
/// and on PBeam in a child process. Check that ERTS writes nothing to
/// stderr and that the child passes. Return the stdout and the exit code of
/// ERTS, and the lines of the child that start with one of `prefixes`: the
/// test harness writes its own lines around the output of the node.
fn run_kernel_boot(
    arguments: &str,
    inetrc: bool,
    prefixes: &[&str],
) -> (String, Option<i32>, String) {
    let environment = |command: &mut Command| {
        command
            .envs(KERNEL_BOOT_ENVIRONMENT)
            .env_remove(KERNEL_BOOT_INETRC.0)
            .stdin(Stdio::null());
        if inetrc {
            command.env(KERNEL_BOOT_INETRC.0, KERNEL_BOOT_INETRC.1);
        }
    };
    let mut reference = Command::new("erl");
    reference
        .args(["-noshell"])
        .args(kernel_boot_flags(arguments));
    environment(&mut reference);
    let reference = reference.output().unwrap();
    let mut child = Command::new(env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "isolated_kernel_boot",
            "--include-ignored",
            "--quiet",
        ])
        .env(ISOLATED, arguments);
    environment(&mut child);
    let child = child.output().unwrap();
    let stdout = String::from_utf8_lossy(&child.stdout);
    // The `inet_gethost` program writes its warnings and fatal errors to
    // fd 2 (inet_gethost.c:2643-2667); PBeam writes none (see "Ports" in
    // docs/erts-differences.md).
    let errors = String::from_utf8_lossy(&reference.stderr);
    let errors: Vec<&str> = errors
        .split_inclusive('\n')
        .filter(|line| !line.contains("/inet_gethost["))
        .collect();
    assert_eq!(errors, Vec::<&str>::new(), "ERTS with {arguments:?}");
    assert!(
        child.status.success(),
        "child exited with {}\n{stdout}",
        child.status
    );
    let node = stdout
        .split_inclusive('\n')
        .filter(|line| prefixes.iter().any(|prefix| line.starts_with(prefix)))
        .collect();
    (
        String::from_utf8_lossy(&reference.stdout).into_owned(),
        reference.status.code(),
        node,
    )
}

#[test]
fn starts_kernel_and_stops_at_init_stop_as_pinned_erts() {
    // `-boot start_clean` runs the pinned `start_clean.boot` of
    // `$ROOT/bin` (init.erl:1256-1308). Its `primLoad` commands load the
    // kernel and stdlib modules through `erl_prim_loader`
    // (init.erl:1342, 1360-1408), its `kernelProcess` commands start
    // `heart`, `logger`, and `application_controller` as linked kernel
    // processes of `init` (init.erl:1519-1528), and its `apply` commands
    // start `kernel` and `stdlib` with `application:start_boot/2`.
    // `kernel_boot_probe:main/0,1` displays the results of the
    // application API and calls `init:stop/0,1`. `init` shuts down
    // `application_controller`, which stops each application
    // (init.erl:924-1062; application_controller.erl:1263-1300), kills
    // each process that is not a system process (init.erl:1072-1093), and
    // halts with the status (init.erl:943-948). `-kernel logger_level
    // warning` keeps the notice of the stop of a temporary application,
    // which has a time stamp, out of the output. The last `apply` command
    // runs `c:erlangrc/0`, which evaluates the `.erlang` file of the home
    // directory and of the user configuration directory (c.erl:1051-1072),
    // so both runs get a home directory that does not exist.
    let expected = "{controller,{registered_name,application_controller},\
        controller_linked_to_init,true,\
        kernel,[{stdlib,\"ERTS  CXC 138 10\",\"8.1\"},{kernel,\"ERTS  CXC 138 10\",\"11.0.4\"}],\
        start_kernel,{error,{already_started,kernel}},\
        start_missing,{error,{\"no such file or directory\",\"kernel_boot_missing.app\"}},\
        ensure_kernel,{ok,[]},ensure_dep,{ok,[kernel_boot_app,kernel_boot_dep]},\
        running,[{kernel_boot_dep,\"kernel boot dependency probe\",\"1\"},\
        {kernel_boot_app,\"kernel boot probe\",\"1\"},\
        {stdlib,\"ERTS  CXC 138 10\",\"8.1\"},{kernel,\"ERTS  CXC 138 10\",\"11.0.4\"}],\
        start_again,{error,{already_started,kernel_boot_app}},\
        stop_dep,ok,stop_dep_again,{error,{not_started,kernel_boot_dep}},\
        ensure_temporary,{ok,[kernel_boot_dep]},\
        ensure_missing,{error,{kernel_boot_missing,{\"no such file or directory\",\"kernel_boot_missing.app\"}}},\
        stop_badarg,{error,badarg},system,[true,false,false],system_badarg,{error,badarg},\
        status,{starting,started}}\r\n\
        {stop,kernel_boot_app,[],undefined,{stopping,started}}\r\n";
    for (arguments, code) in [("main", 0), ("main 3", 3)] {
        let (reference, status, node) =
            run_kernel_boot(arguments, true, &["{controller", "{stop,"]);
        assert_eq!(
            (reference.as_str(), status),
            (expected, Some(code)),
            "ERTS with {arguments:?}"
        );
        assert_eq!(node, expected, "PBeam with {arguments:?}");
    }
}

#[test]
fn looks_up_the_host_name_at_the_kernel_boot_as_pinned_erts() {
    // Without a domain in an inetrc file, `inet_config:init/0` looks up
    // the host name on Linux at the kernel start, when the domain is empty
    // (inet_config.erl:110-133). The native lookup opens the
    // `inet_gethost` port program of `$ROOT/erts-*/bin` with the settings
    // `[overlapped_io, {args, Args}, {packet, 4}, eof, binary]` and
    // exchanges packets with it (inet_gethost_native.erl:167-190,
    // 412-428). `kernel_boot_probe:lookup/0` then displays the domain and
    // the native lookup of the host name and of `localhost`, on each host
    // OS, and calls `init:stop/0`.
    let (reference, status, node) = run_kernel_boot("lookup", false, &["{lookup,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(reference.starts_with("{lookup,[native],"), "{reference}");
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn serves_the_inet_gethost_port_program_as_pinned_erts() {
    // PBeam runs the `inet_gethost` port program in a thread of the node.
    // `kernel_boot_probe:gethost/0` sends it requests that it answers
    // (inet_gethost.c:1791-1976), discards (inet_gethost.c:876-979,
    // 1548-1585), and ends on with a fatal error (inet_gethost.c:896-901,
    // 1325-1351, 2643-2667), and calls `inet_gethost_native` and `inet`
    // for localhost, 127.0.0.1, ::1, and an unknown name. The replies
    // depend on the resolver of the host, so the test compares the two
    // runs on the host.
    let (reference, status, node) = run_kernel_boot("gethost", false, &["{gethost,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(
        reference.starts_with("{gethost,[none,none,none,none,none,none],[{data,"),
        "{reference}"
    );
    assert!(
        reference.contains("[[{exit_status,1},eof],[{exit_status,1},eof]]"),
        "{reference}"
    );
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn reports_the_socket_support_of_the_host_at_the_kernel_boot_as_pinned_erts() {
    // The pinned ERTS preloads `socket_registry`, `prim_socket`, and
    // `prim_net` (erts/emulator/Makefile.in:804-817), so
    // `erl_init:start/2` runs `prim_socket:on_load/0` (erl_init.erl:42-48).
    // `prim_socket:init/0` stores the lists of `nif_supports/1` as maps in
    // persistent terms (prim_socket.erl:184-255).
    // `kernel_boot_probe:supports/0` displays `nif_supports/0`, the maps as
    // sorted lists, and the results of other keys, and calls
    // `init:stop/0`. The values depend on the host OS and on the host file
    // `/etc/protocols`, so the test compares the two runs on the host.
    let (reference, status, node) = run_kernel_boot("supports", true, &["{supports,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(
        reference.starts_with("{supports,[{sctp,false},{ipv6,true},{local,true},"),
        "{reference}"
    );
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn opens_binds_and_closes_sockets_at_the_kernel_boot_as_pinned_erts() {
    // `kernel_boot_probe:socket/0` runs `socket:open/2,3,4`,
    // `socket:bind/2`, `socket:sockname/1`, `socket:close/1`, and
    // `socket:info/0,1` on the NIFs `nif_open/4`, `nif_bind/2`,
    // `nif_sockname/1`, `nif_close/1`, `nif_finalize_close/1`, and
    // `nif_info/0,1` (prim_socket_nif.c:4546-4743, 5733-5960, 13468-13545;
    // unix_socket_syncio.c:1689-1938, 4544-4853), the socket registry
    // messages of an open and a close (prim_socket_nif.c:17300-17338), the
    // global counters (prim_socket_nif.c:17127-17218), and the error
    // returns. The probe displays only the results that are the same on
    // Linux and macOS.
    let (reference, status, node) = run_kernel_boot("socket", false, &["{socket,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(
        reference.starts_with("{socket,[[#{num_cnt_bits=>64,"),
        "{reference}"
    );
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn sets_and_gets_socket_options_at_the_kernel_boot_as_pinned_erts() {
    // `kernel_boot_probe:sockopt/0` runs `socket:setopt/3` and
    // `socket:getopt/2` on the NIFs `nif_setopt/5` and `nif_getopt/3,5`
    // (prim_socket_nif.c:7801-7961, 8328-8405, 8473-8533, 11504-11710,
    // 12198-12254): boolean and integer options of the levels `socket`,
    // `tcp`, and `ip`, the `otp` options `domain`, `meta`, and
    // `use_registry`, the error returns, a closed socket, and
    // `socket:bind/2` with the atoms `any`, `loopback`, and `broadcast`,
    // which read `{otp, domain}` (socket.erl:3477). Some errors depend on
    // the host OS, so the test compares the two runs on the host.
    let (reference, status, node) = run_kernel_boot("sockopt", false, &["{sockopt,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(
        reference.starts_with("{sockopt,[[ok,{ok,true},ok,{ok,false},"),
        "{reference}"
    );
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn sets_and_gets_the_host_socket_options_as_pinned_erts() {
    // `kernel_boot_probe:sockopt_host/0` runs the handlers
    // `esock_setopt_bool_opt`, `esock_getopt_bool_opt`,
    // `esock_setopt_int_opt`, `esock_getopt_int_opt`, `esock_setopt_hops`,
    // `esock_setopt_multicast_if`, `esock_getopt_multicast_if`, and
    // `esock_setopt_in_update_membership` (prim_socket_nif.c:8728-8834,
    // 8941-8954, 11219-11259, 12638-12669, 13009-13048), the get of an
    // option without a get handler (prim_socket_nif.c:12234-12236),
    // `esock_setopt_tos` and `esock_getopt_tos` (prim_socket_nif.c:8757-8775,
    // 12679-12697, 16087-16344, 16637-16816), on Linux the string, domain,
    // protocol, and IPv6 membership handlers (prim_socket_nif.c:8538-8545,
    // 8961-9027, 11297-11338, 12259-12268, 12456-12476, 12569-12600,
    // 12741-12752, 13422-13451), and the options that the host does not
    // support. The host options differ
    // between Linux and macOS, so the test compares the two runs on the
    // host.
    let (reference, status, node) = run_kernel_boot("sockopt_host", false, &["{sockopt_host,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(
        reference.starts_with("{sockopt_host,[[ok,{ok,true},ok,{ok,false},"),
        "{reference}"
    );
    assert!(
        reference.contains("[ok,{ok,{127,0,0,1}},ok,{ok,{0,0,0,0}},"),
        "{reference}"
    );
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn sets_and_gets_the_raw_and_native_socket_options_as_pinned_erts() {
    // `kernel_boot_probe:sockopt_raw/0` runs `esock_setopt_bool_opt`,
    // `esock_setopt_int_opt`, and their getters for the options without a
    // typed call, `esock_setopt_ip_mtu_discover` and its IPv6 sibling with
    // their getters, `esock_setopt_in_update_source`,
    // `esock_setopt_addrform`, `esock_setopt_native`, and
    // `esock_getopt_native` with each value spec (prim_socket_nif.c:8414-8465,
    // 8704-8717, 8851-8934, 12117-12192, 12608-12629, 12709-12728,
    // 13009-13048, 13126-13216, 16364-16570). The host options differ
    // between Linux and macOS, so the test compares the two runs on the
    // host.
    let (reference, status, node) = run_kernel_boot("sockopt_raw", false, &["{sockopt_raw,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(
        reference.starts_with("{sockopt_raw,[[ok,{ok,true},"),
        "{reference}"
    );
    assert!(
        reference.contains("{error,{invalid,{value_spec,foo}}}"),
        "{reference}"
    );
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn sets_msfilter_and_gets_pktoptions_as_pinned_erts() {
    // `kernel_boot_probe:sockopt_pkt/0` runs `esock_setopt_msfilter` and
    // `esock_getopt_pktoptions` with the control message encoders of
    // `cmsgLevelIP` (prim_socket_nif.c:8602-8695, 13255-13405, 14176-14215,
    // 14335-14387, 14453-14490). Only Linux has the two options, so the test
    // compares the two runs on the host.
    let (reference, status, node) = run_kernel_boot("sockopt_pkt", false, &["{sockopt_pkt,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(
        reference.starts_with("{sockopt_pkt,[{error,"),
        "{reference}"
    );
    #[cfg(target_os = "linux")]
    assert!(reference.contains("{type,pktinfo}"), "{reference}");
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn sets_and_gets_the_otp_socket_options_and_linger_as_pinned_erts() {
    // `kernel_boot_probe:sockopt_otp/0` runs the handlers of
    // `esock_setopt_otp` and `esock_getopt_otp`
    // (prim_socket_nif.c:7873-8405, 11593-12010), the raise of an `otp`
    // option without a setter (prim_socket_nif.c:7942-7957), the
    // inheritance of an accepted socket (unix_socket_syncio.c:2843-2849),
    // the buffer size of a receive with length 0
    // (unix_socket_syncio.c:3900), and `esock_setopt_linger` and
    // `esock_getopt_linger` (prim_socket_nif.c:8551-8597, 12482-12530).
    let (reference, status, node) = run_kernel_boot("sockopt_otp", false, &["{sockopt_otp,"]);
    assert_eq!(status, Some(0), "ERTS");
    // ERTS writes a warning with a time stamp for a controlling process
    // that is not a local pid (prim_socket_nif.c:8082-8086). PBeam does not
    // write it (row "NIF stubs" of docs/erts-differences.md).
    let start = reference.find("{sockopt_otp,").unwrap();
    assert!(
        reference[..start].contains("Failed get pid of new controlling process"),
        "{reference}"
    );
    let reference = &reference[start..];
    assert!(
        reference.contains(r#"[{ok,<<"0123">>},{ok,<<"4567">>},{ok,<<"89">>}]"#),
        "{reference}"
    );
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn connects_listens_accepts_and_cancels_sockets_with_select_messages_as_pinned_erts() {
    // `kernel_boot_probe:socket_select/0` runs `socket:connect/2,3`,
    // `socket:listen/1,2`, `socket:accept/1,2`, and `socket:cancel/2` with
    // timeouts and `nowait` on the NIFs `nif_connect/1,3`, `nif_listen/2`,
    // `nif_accept/2`, and `nif_cancel/3` (prim_socket_nif.c:6027-6360,
    // 13895-14165; unix_socket_syncio.c:2008-2168, 2379-2878, 4967-5132),
    // the select messages of the poll thread (prim_socket_nif.c:17640-17760),
    // the abort messages of a second accept, a close, and an owner exit
    // (unix_socket_syncio.c:2643-2737, 4600-4730), and the exit of an
    // acceptor (unix_socket_syncio.c:10583-10608).
    let (reference, status, node) = run_kernel_boot("socket_select", false, &["{socket_select,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(
        reference.starts_with("{socket_select,[ok,[bound,listening],{error,timeout}],"),
        "{reference}"
    );
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn sends_and_receives_on_stream_sockets_as_pinned_erts() {
    // `kernel_boot_probe:socket_io/0` runs `socket:send/2,3,4`,
    // `socket:recv/1,2,3,4`, and `socket:cancel/2` with timeouts, `zero`,
    // and `nowait` on the NIFs `nif_send/4`, `nif_recv/4`, and
    // `nif_cancel/3` (prim_socket_nif.c:6440-6509, 7056-7124;
    // unix_socket_syncio.c:2996-3036, 3796-3878, 5144-5355, 6809-7196,
    // 9869-10520): a partial send and its continuation, the read select,
    // the end of a stream, the abort of a waiting reader at a close
    // (unix_socket_syncio.c:4637-4722), the counters of `socket:info/1`, and
    // the argument errors.
    let (reference, status, node) = run_kernel_boot("socket_io", false, &["{socket_io,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(
        reference.starts_with("{socket_io,[ok,{ok,<<\"hello\">>},"),
        "{reference}"
    );
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn sends_and_receives_on_datagram_and_stream_sockets_with_addresses_as_pinned_erts() {
    // `kernel_boot_probe:socket_dgram/0` runs `socket:sendto/3,4,5`,
    // `socket:recvfrom/1,2,3,4`, `socket:peername/1`, `socket:shutdown/2`,
    // and `socket:cancel/2` on inet, inet6, and local datagram sockets and inet
    // stream sockets, on the NIFs `nif_sendto/5`, `nif_recvfrom/4`,
    // `nif_peername/1`, `nif_shutdown/2`, and `nif_cancel/3`
    // (prim_socket_nif.c:6527-6600, 7153-7231, 7722-7779, 13626-13704,
    // 13955-14078; unix_socket_syncio.c:3042-3091, 3888-3977): the source
    // addresses, a truncated datagram, the read select, a partial send, the
    // counters of `socket:info/1`, and the argument errors.
    let (reference, status, node) = run_kernel_boot("socket_dgram", false, &["{socket_dgram,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(
        reference.starts_with("{socket_dgram,[{error,enotconn},ok,{ok,{true,<<\"ping\">>}},"),
        "{reference}"
    );
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn receives_with_flags_on_inet_sockets_as_pinned_erts() {
    // `kernel_boot_probe:socket_recvfrom_flags/0` runs `socket:recvfrom/4`
    // with the flag peek on inet and inet6 datagram sockets and an inet
    // stream socket. `essio_recvfrom` passes the flags to `recvfrom(2)`
    // (unix_socket_syncio.c:3925-3927); a stream socket gives the address
    // of length 0, `#{family => unspec, addr => <<>>}`.
    let (reference, status, node) =
        run_kernel_boot("socket_recvfrom_flags", false, &["{socket_recvfrom_flags,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(
        reference.starts_with(
            "{socket_recvfrom_flags,[{error,timeout},ok,{ok,{true,<<\"peek\">>}},{ok,{true,<<\"pe\">>}},"
        ),
        "{reference}"
    );
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn sends_io_vectors_as_pinned_erts() {
    // `kernel_boot_probe:socket_sendv/0` runs `socket:sendv/2,3` on the NIF
    // `nif_sendv/3` (prim_socket_nif.c:6777-6822; unix_socket_syncio.c:3361-3524):
    // an I/O vector of more than `iov_max` elements, which gives
    // `{iov, Written}` on a stream socket and `{invalid, iov}` on a datagram
    // socket, a partial send, the counters, and the argument errors. The
    // `writev(2)` of no entry gives `EINVAL` on macOS and 0 on Linux.
    let (reference, status, node) = run_kernel_boot("socket_sendv", false, &["{socket_sendv,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(
        reference.starts_with("{socket_sendv,[ok,{ok,<<\"abcd\">>},"),
        "{reference}"
    );
    assert_eq!(node, reference, "PBeam");
}

#[cfg(not(target_os = "linux"))]
#[test]
fn receives_with_the_flag_errqueue_as_pinned_erts() {
    // macOS has no `MSG_ERRQUEUE`, so the flag errqueue gives the flags 0.
    let (reference, status, node) = run_kernel_boot(
        "socket_recvfrom_errqueue",
        false,
        &["{socket_recvfrom_errqueue,"],
    );
    assert_eq!(status, Some(0), "ERTS");
    assert_eq!(reference, "{socket_recvfrom_errqueue,{error,timeout}}\r\n");
    assert_eq!(node, reference, "PBeam");
}

#[test]
fn sends_and_receives_with_gen_tcp_and_gen_udp_on_the_socket_backend_as_pinned_erts() {
    // `gen_tcp` and `gen_udp` with `{inet_backend, socket}` set the options
    // `{otp, iow}` and `{otp, meta}` (gen_tcp_socket.erl:1637-1642,
    // gen_udp_socket.erl:1311-1312) on `esock_setopt_otp`
    // (prim_socket_nif.c:7873-7961).
    for (probe, expected) in [
        (
            "socket_backend_tcp",
            "{socket_backend_tcp,[ok,{ok,<<\"ping\">>},ok,{ok,<<\"pong\">>},{error,timeout},ok,{error,closed},ok,ok,ok]}\r\n",
        ),
        (
            "socket_backend_udp",
            "{socket_backend_udp,[ok,true,ok,<<\"pong\">>,{error,timeout},ok,ok]}\r\n",
        ),
    ] {
        let prefix = format!("{{{probe},");
        let (reference, status, node) = run_kernel_boot(probe, false, &[&prefix]);
        assert_eq!(status, Some(0), "ERTS {probe}");
        assert_eq!(reference, expected);
        assert_eq!(node, reference, "PBeam {probe}");
    }
}

#[test]
fn stops_the_run_at_the_socket_calls_that_pbeam_does_not_support() {
    // PBeam has no acceptor queue (`essio_accept_accepting_other`,
    // unix_socket_syncio.c:2746-2757). ERTS connects with an
    // address that it did not decode (prim_socket_nif.c:6122-6143), and it
    // passes the flags of a recvfrom to `recvfrom(2)`
    // (unix_socket_syncio.c:3925-3927), which PBeam does only on inet and
    // inet6 sockets without `MSG_ERRQUEUE`. These calls stop the run with a
    // runtime error (row "NIF stubs").
    let mut probes = vec![
        (
            "socket_acceptors",
            "prim_socket:nif_accept/2: a second acceptor process is not supported",
        ),
        (
            "socket_connect_invalid",
            "prim_socket:nif_connect/3: an address that does not decode is not supported",
        ),
        (
            "socket_recvfrom_local_flags",
            "prim_socket:nif_recvfrom/4: the flags 2 on a socket of the domain 1 are not supported",
        ),
    ];
    if cfg!(target_os = "linux") {
        probes.push((
            "socket_recvfrom_errqueue",
            "prim_socket:nif_recvfrom/4: the flags 8192 on a socket of the domain 2 are not supported",
        ));
    }
    for (probe, text) in probes {
        let child = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "isolated_kernel_boot_error",
                "--include-ignored",
                "--quiet",
            ])
            .env(ISOLATED, probe)
            .env(ISOLATED_ERROR, text)
            .envs(KERNEL_BOOT_ENVIRONMENT)
            .env_remove(KERNEL_BOOT_INETRC.0)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{probe}: {}",
            String::from_utf8_lossy(&child.stdout)
        );
    }
}

/// The child side of
/// `stops_the_run_at_the_socket_calls_that_pbeam_does_not_support`: boot the
/// node with the `-s` argument of [`ISOLATED`], and expect the runtime
/// error that contains the text of [`ISOLATED_ERROR`].
#[test]
#[ignore = "the kernel boot tests run it in a child process"]
fn isolated_kernel_boot_error() {
    let arguments = env::var(ISOLATED).expect("run it through its parent test");
    let text = env::var(ISOLATED_ERROR).expect("run it through its parent test");
    let flags = kernel_boot_flags(&arguments);
    let flags = flags.iter().map(String::as_bytes).collect::<Vec<_>>();
    let esock = ESOCK_MODULES.map(pinned_beam);
    let esock = esock.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let error = boot_node(&esock, &flags).unwrap_err().to_string();
    assert!(error.contains(&text), "{error}");
}

/// The pinned modules of the socket NIF libraries, which the pinned ERTS
/// preloads (erts/emulator/Makefile.in:804-817), and which the kernel boot
/// loads as preloaded modules.
const ESOCK_MODULES: [&str; 3] = ["socket_registry", "prim_socket", "prim_net"];

/// The child side of `starts_kernel_and_stops_at_init_stop_as_pinned_erts`,
/// `looks_up_the_host_name_at_the_kernel_boot_as_pinned_erts`, and
/// `reports_the_socket_support_of_the_host_at_the_kernel_boot_as_pinned_erts`:
/// boot the node with [`ESOCK_MODULES`], `start_clean`, and the `-s`
/// arguments of the environment variable [`ISOLATED`], and expect the halt
/// of `init:stop/0,1` with the status of the second argument, or 0 without
/// it.
#[test]
#[ignore = "the kernel boot tests run it in a child process"]
fn isolated_kernel_boot() {
    let arguments = env::var(ISOLATED).expect("run it through its parent test");
    let flags = kernel_boot_flags(&arguments);
    let flags = flags.iter().map(String::as_bytes).collect::<Vec<_>>();
    let code = arguments
        .split_whitespace()
        .nth(1)
        .map_or(0, |status| status.parse().unwrap());
    let esock = ESOCK_MODULES.map(pinned_beam);
    let esock = esock.iter().map(Vec::as_slice).collect::<Vec<_>>();
    assert_eq!(
        boot_node(&esock, &flags).unwrap_err(),
        BeamError::Halt(code)
    );
}

#[test]
fn stops_the_run_at_the_exit_of_init_as_pinned_erts() {
    // The emulator starts `init` as a system process (erl_init.c:355-399),
    // and the exit of a system process stops it with `System process <Pid>
    // terminated: <Reason>` (erl_process.c:14186-14188). The boot script
    // of `init_kill_probe` kills `init` from the boot process. ERTS writes
    // the text and exits with status 1, with no crash dump with
    // `ERL_CRASH_DUMP_SECONDS=0`; PBeam stops the run with the text as its
    // error (row "Node boot").
    let text = "System process <0.0.0> terminated: killed";
    let reference = Command::new("erl")
        .args(["-noshell", "-boot", KILL_BOOT_FILE, "-pa", "fixtures/boot"])
        .env("ERL_CRASH_DUMP_SECONDS", "0")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        (
            String::from_utf8_lossy(&reference.stderr).lines().next(),
            reference.stdout.as_slice(),
            reference.status.code()
        ),
        (Some(text), &b""[..], Some(1)),
        "ERTS"
    );
    assert_eq!(
        boot_probe(KILL_PROBE, KILL_BOOT_FILE).unwrap_err(),
        BeamError::Runtime(text.into()),
        "PBeam"
    );
}

/// Return the standard error text of `system_process_probe:Function()` on
/// the pinned ERTS, which writes no crash dump with
/// `ERL_CRASH_DUMP_SECONDS=0`, with its exit status and the signal that
/// stopped it.
fn erts_stop(function: &str) -> (String, Option<i32>, Option<i32>) {
    let output = Command::new("erl")
        .args(["-noshell", "-pa", "fixtures/boot", "-eval"])
        .arg(format!("system_process_probe:{function}()."))
        .env("ERL_CRASH_DUMP_SECONDS", "0")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        String::from_utf8(output.stderr).unwrap(),
        output.status.code(),
        output.status.signal(),
    )
}

fn run_system_process_probe(function: &str) -> Result<pbeam_runtime::HostTerm, BeamError> {
    BeamProgram::load(&[SYSTEM_PROCESS_PROBE]).unwrap().execute(
        "system_process_probe",
        function,
        &[],
    )
}

#[test]
fn stops_the_run_at_the_exit_of_a_system_process_as_pinned_erts() {
    // The process of `erts_internal:spawn_system_process/3` is a system
    // process (erl_init.c:333-353, 436-456), and the exit of a system
    // process stops the emulator with the text `System process <Pid>
    // terminated: <Reason>` (erl_process.c:14186-14188), status 1 with no
    // crash dump. The pid of the ERTS process depends on the processes of
    // the boot, so only its form is compared.
    let (stderr, status, _) = erts_stop("exits");
    let text = stderr.lines().next().unwrap();
    let pid = text
        .strip_prefix("System process <0.")
        .and_then(|rest| rest.strip_suffix(".0> terminated: normal"))
        .unwrap_or_else(|| panic!("ERTS: {stderr}"));
    assert!(
        pid.bytes().all(|byte| byte.is_ascii_digit()),
        "ERTS: {stderr}"
    );
    assert_eq!(status, Some(1), "ERTS");
    assert_eq!(
        run_system_process_probe("exits").unwrap_err(),
        BeamError::Runtime("System process <0.1.0> terminated: normal".into())
    );
}

#[test]
fn stops_the_run_for_a_system_process_without_an_export_as_pinned_erts() {
    // `erl_spawn_system_process` looks up the export first and stops the
    // emulator with `No function M:F/A` when it finds none; the arity of an
    // improper list is -1 (erl_init.c:342-346). ERTS aborts with SIGABRT
    // after the text; PBeam stops the run with the text as its error.
    for (function, text) in [
        (
            "absent",
            "No function system_process_probe:absent_function/0",
        ),
        ("improper", "No function system_process_probe:exits/-1"),
    ] {
        let (stderr, _, signal) = erts_stop(function);
        assert_eq!(
            (stderr.lines().next(), signal),
            (Some(text), Some(6)),
            "ERTS {function}"
        );
        assert_eq!(
            run_system_process_probe(function).unwrap_err(),
            BeamError::Runtime(text.into()),
            "PBeam {function}"
        );
    }
}

#[test]
fn lists_the_loaded_modules_with_the_last_loaded_first() {
    // `erlang:loaded/0` gives the modules of the module table, the last
    // index first (beam_bif_load.c:762-791). The module table of ERTS holds
    // the preloaded modules first, so this test compares the order of the
    // PBeam start modules, which load in the order of the program.
    let program = BeamProgram::load(&[PROBE, SYSTEM_PROCESS_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let loaded = program
        .execute("system_process_probe", "loaded", &[])
        .unwrap();
    assert_eq!(
        loaded.term,
        Term::list_from(vec![atom("system_process_probe"), atom("init_boot_probe")])
    );
}

#[test]
fn keeps_the_module_table_order_of_loaded_after_a_delete_and_a_reload_as_pinned_erts() {
    // `erlang:finish_loading/1` puts the module entry at the first load
    // (beam_bif_load.c:425), and the entry keeps its index after
    // `erlang:delete_module/1`, so a module with only old code stays in
    // `erlang:loaded/0` and keeps its place after a later load
    // (beam_bif_load.c:762-791). `loaded_order_probe:main/0` loads
    // `loaded_order_b` before `loaded_order_a`, deletes `loaded_order_b`,
    // and loads it again. It keeps only its own two modules of each list,
    // because the other loaded modules of ERTS and PBeam differ: ERTS has
    // its preloaded and boot modules, and PBeam has the start modules.
    let expected = "{[loaded_order_a,loaded_order_b],[loaded_order_a,loaded_order_b],\
        [loaded_order_a,loaded_order_b]}\n";
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/boot",
            "-eval",
            "io:format(\"~w~n\", [loaded_order_probe:main()]), halt().",
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        (
            String::from_utf8_lossy(&reference.stdout).as_ref(),
            reference.status.code()
        ),
        (expected, Some(0)),
        "ERTS"
    );
    let erlang = pinned_beam("erlang");
    let prim_file = pinned_beam("prim_file");
    let program = BeamProgram::load(&[&erlang, &prim_file, LOADED_ORDER_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let order = || Term::list_from(vec![atom("loaded_order_a"), atom("loaded_order_b")]);
    let result = program.execute("loaded_order_probe", "main", &[]).unwrap();
    assert_eq!(
        result.term,
        Term::tuple(vec![order(), order(), order()]),
        "PBeam"
    );
}

#[test]
fn stops_the_boot_when_the_first_process_exits_normally() {
    // `erl_init:start/2` calls `init:boot/1` (erl_init.erl `run/3`). A boot
    // module whose `boot/1` returns makes the first process exit with the
    // reason `normal`. The first process is a system process, so its exit
    // stops the run (erl_process.c:14186-14188).
    let mut beams = BOOT_MODULES
        .iter()
        .filter(|name| **name != "init")
        .map(|name| pinned_beam(name))
        .collect::<Vec<_>>();
    beams.push(include_bytes!("../../fixtures/boot_return/init.beam").to_vec());
    let slices = beams.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let program = BeamProgram::load(&slices).unwrap();
    assert_eq!(
        program.boot(&[], Schedulers::host()),
        BeamError::Runtime("System process <0.0.0> terminated: normal".to_owned())
    );
}
