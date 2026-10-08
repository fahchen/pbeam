//! The boot module of `examples/hello_world` and the pinned OTP modules that
//! it calls. A test that runs the boot with the `pbeam` binary includes this
//! file with `#[path = "common/boot.rs"] mod boot;`. A constant in
//! `common/mod.rs` would be dead code in each other test crate.

/// The compiled boot module `pbeam_boot` of the Hello World example.
///
/// # Examples
///
/// ```rust,ignore
/// Command::new(env!("CARGO_BIN_EXE_pbeam")).args(["pbeam_boot", "start", "boot_probe", "main", "--", BOOT_BEAM]);
/// ```
pub const BOOT_BEAM: &str = "examples/hello_world/pbeam_boot.beam";

/// The pinned OTP modules that `pbeam_boot` calls when it starts the
/// `init` request process, `logger_server`, `application_controller`,
/// `standard_error`, `erl_signal_server`, `user_sup`, `logger_sup`,
/// `erl_signal_handler`, and the `default` logger handler. Without one of
/// them, the boot stops with `unresolved external MFA` of that module.
///
/// # Examples
///
/// ```rust,ignore
/// let beams = BOOT_MODULES.map(pinned_beam_path);
/// Command::new(env!("CARGO_BIN_EXE_pbeam"))
///     .args(["pbeam_boot", "start", "boot_probe", "main", "--", BOOT_BEAM, BOOT_PROBE])
///     .args(&beams);
/// ```
pub const BOOT_MODULES: [&str; 41] = [
    "erlang",
    "prim_tty",
    "proc_lib",
    "application",
    "application_controller",
    "lists",
    "os",
    "unicode",
    "gen_event",
    "gen",
    "prim_tty_sighandler",
    "erl_signal_handler",
    "standard_error",
    "supervisor_bridge",
    "user_sup",
    "user_drv",
    "group",
    "gen_statem",
    "io",
    "io_lib",
    "io_lib_format",
    "proplists",
    "queue",
    "gen_server",
    "logger",
    "logger_config",
    "edlin",
    "edlin_key",
    "logger_server",
    "logger_simple_h",
    "logger_sup",
    "supervisor",
    "logger_std_h",
    "logger_h_common",
    "logger_olp",
    "logger_proxy",
    "logger_handler_watcher",
    "logger_formatter",
    "maps",
    "init",
    "erts_internal",
];
