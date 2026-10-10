//! The boot module of `examples/hello_world` and the pinned OTP modules that
//! it calls. A test that runs the boot with the `pbeam` binary includes this
//! file with `#[path = "../common/boot.rs"] mod boot;`. A function in
//! `common/mod.rs` would be dead code in each other test crate.

use std::{
    fs,
    path::Path,
    process::{self, Command},
    sync::OnceLock,
};

/// The source of the boot module `pbeam_boot` of the Hello World example.
const BOOT_SOURCE: &str = "examples/hello_world/pbeam_boot.erl";

/// The directory of the compiled boot module.
const BOOT_DIRECTORY: &str = "target/examples/hello_world";

/// Compile `pbeam_boot.erl` with `erlc +deterministic` from `PATH` into
/// `target/examples/hello_world/` once for each test process, and return
/// the path of `pbeam_boot.beam`. `erlc` compiles into a directory of this
/// process, and a rename then replaces the shared file atomically. A test
/// can run in a child process, and `erlc` first deletes its output file
/// (compile.erl `remove_file/2`) and writes through the fixed temporary
/// file `pbeam_boot.bea#` (compile.erl `save_binary_1/2`). Thus a
/// compilation into the shared directory could remove the file while
/// another process reads it, or collide with another compilation.
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
        let directory = format!("{BOOT_DIRECTORY}/erlc-{}", process::id());
        fs::create_dir_all(&directory).unwrap();
        let output = Command::new("erlc")
            .args(["+deterministic", "-o", &directory, BOOT_SOURCE])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "erlc exited with {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let beam = Path::new("target/examples/hello_world/pbeam_boot.beam");
        fs::rename(Path::new(&directory).join("pbeam_boot.beam"), beam).unwrap();
        fs::remove_dir(&directory).unwrap();
        beam
    })
}

/// The pinned OTP modules that `pbeam_boot` calls when it starts the
/// `init` request process, `logger_server`, `application_controller`,
/// `standard_error`, `file_server_2`, `erl_signal_server`, `user_sup`,
/// `logger_sup`, `erl_signal_handler`, and the `default` logger handler. Without one of
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
pub const BOOT_MODULES: [&str; 42] = [
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
    "file_server",
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
