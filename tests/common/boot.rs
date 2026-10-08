//! The boot module of `examples/hello_world` and the pinned OTP modules that
//! it calls. A test that runs the boot with the `pbeam` binary includes this
//! file with `#[path = "common/boot.rs"] mod boot;`. A function in
//! `common/mod.rs` would be dead code in each other test crate.

use std::{
    fs::{self, File},
    path::Path,
    process::Command,
    sync::OnceLock,
};

/// The source of the boot module `pbeam_boot` of the Hello World example.
const BOOT_SOURCE: &str = "examples/hello_world/pbeam_boot.erl";

/// The directory of the compiled boot module.
const BOOT_DIRECTORY: &str = "target/examples/hello_world";

/// Compile `pbeam_boot.erl` with `erlc +deterministic` from `PATH` into
/// `target/examples/hello_world/` once for each test process, and return
/// the path of `pbeam_boot.beam`. A lock on `pbeam_boot.lock` in that
/// directory serializes the compilation across processes, because `erlc`
/// writes each output through the same temporary file `pbeam_boot.bea#`
/// (compile.erl `save_binary_1/2`) and a test can run in a child process.
///
/// # Examples
///
/// ```rust,ignore
/// Command::new(env!("CARGO_BIN_EXE_pbeam"))
///     .args(["pbeam_boot", "start", "boot_probe", "main", "--", BOOT_PROBE])
///     .arg(boot_beam());
/// assert_eq!(boot_beam(), Path::new("target/examples/hello_world/pbeam_boot.beam"));
/// ```
pub fn boot_beam() -> &'static Path {
    static BOOT_BEAM: OnceLock<&'static Path> = OnceLock::new();
    BOOT_BEAM.get_or_init(|| {
        fs::create_dir_all(BOOT_DIRECTORY).unwrap();
        let lock = File::create(Path::new(BOOT_DIRECTORY).join("pbeam_boot.lock")).unwrap();
        lock.lock().unwrap();
        let output = Command::new("erlc")
            .args(["+deterministic", "-o", BOOT_DIRECTORY, BOOT_SOURCE])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "erlc exited with {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Path::new("target/examples/hello_world/pbeam_boot.beam")
    })
}

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
///     .args(["pbeam_boot", "start", "boot_probe", "main", "--", BOOT_PROBE])
///     .args(&beams)
///     .arg(boot_beam());
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
