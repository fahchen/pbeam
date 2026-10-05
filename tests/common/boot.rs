//! The pinned OTP modules that the boot of the `pbeam` binary calls. A test
//! that runs the `pbeam` binary includes this file with
//! `#[path = "common/boot.rs"] mod boot;`. A constant in `common/mod.rs`
//! would be dead code in each other test crate.

/// The pinned OTP modules that `pbeam_boot` calls when it starts
/// `standard_error`, `erl_signal_server`, `user_sup`, and
/// `erl_signal_handler`. Without one of them, the `pbeam` binary stops with
/// `unresolved external MFA` of that module.
///
/// # Examples
///
/// ```rust,ignore
/// let beams = BOOT_MODULES.map(pinned_beam_path);
/// Command::new(env!("CARGO_BIN_EXE_pbeam")).args(["boot_probe", "main", BOOT_PROBE]).args(&beams);
/// ```
pub const BOOT_MODULES: [&str; 28] = [
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
];
