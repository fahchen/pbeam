//! The `pbeam` command-line binary. It loads BEAM files, runs a function as the root process, and
//! boots a node as `erl` does.

use std::{
    ffi::{OsStr, OsString},
    fs,
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    process::ExitCode,
};

use clap::Parser;
use pbeam_runtime::{
    BeamError, BeamProgram, HostTerm, Schedulers, Term, add_erlexec_path,
    install_start_dispositions,
};

#[derive(Parser)]
#[command(
    name = "pbeam",
    version,
    about = "Load BEAM files and run a function as the root process.",
    after_help = "`pbeam erl --root DIR --bindir DIR --preloaded DIR -- ERL_ARGUMENT...` boots the node as `erl` does (see `pbeam erl --help`)."
)]
struct Cli {
    /// Module name, such as `pbeam_boot`.
    #[arg(value_name = "MODULE")]
    module: String,
    /// Function name to run.
    #[arg(value_name = "FUNCTION")]
    function: String,
    /// Arguments of the function. Each argument is an atom, such as a
    /// module or function name.
    #[arg(value_name = "ATOM")]
    arguments: Vec<String>,
    /// A loaded policy module, which exports `initial/1` and `grant/3`.
    /// Each effect entry then checks the capabilities of its caller.
    /// Without it, the run skips each capability check.
    #[arg(long, value_name = "MODULE")]
    policy: Option<String>,
    /// Skip each capability check, as a run without `--policy` does. It
    /// cannot be used with `--policy`.
    // The run reads no value: a run without `--policy` skips the checks,
    // and `clap` rejects the flag with `--policy`.
    #[arg(long = "no-capability-checks", conflicts_with = "policy")]
    _no_capability_checks: bool,
    /// Run in boot mode, only with `--policy`: the root process starts with
    /// the boot authority, which the policy does not check, and
    /// `pbeam_cap:enter_runtime(Module, Function)` gives it the
    /// capabilities of `initial/1` of the policy module, one way.
    #[arg(long, requires = "policy")]
    boot: bool,
    /// A code path directory of the boot, as `erl -pa DIR` gives. Repeat it
    /// for more directories. With one or more `--pa`, or with `--root`, the
    /// function gets one more argument after the atoms: the boot flag list,
    /// with one `{pa, [Dir]}` tuple for each `--pa` in the command line
    /// order, and each value as a binary of its bytes, as
    /// `init:parse_boot_args/1` gives the flags. The `pbeam_boot` module of
    /// `examples/hello_world` takes this list as the third argument of
    /// `start/3`.
    #[arg(long = "pa", value_name = "DIR")]
    code_path: Vec<OsString>,
    /// The root directory of the OTP installation, as `erl` gives it to
    /// `init` with `-root DIR`. It has no default. With it, the boot flag
    /// list starts with `{root, [Dir]}`, and the `pbeam_boot` module sets
    /// the code path of `start_clean.boot` and starts the `kernel`
    /// application, whose `code_server` reads the root with
    /// `init:get_argument(root)`. `pbeam_boot` needs it, as `init` needs
    /// `-root`: without it, the boot exits with
    /// `no_or_multiple_root_variables`.
    #[arg(long, value_name = "DIR")]
    root: Option<OsString>,
    /// The code loading mode, as `erl -mode MODE` gives it. Without it, the
    /// mode is `interactive`, as for `erl`. With it, the boot flag list ends
    /// with `{mode, [Mode]}`. It needs `--root`: only `code_server` reads
    /// it.
    #[arg(long, value_name = "MODE", value_parser = ["interactive", "embedded"], requires = "root")]
    mode: Option<String>,
    /// The extracted release of an embedded run, which the boot removes at
    /// its exit. The launcher of an appended bundle sets it; a command line
    /// does not set it.
    #[arg(long = "extracted-root", value_name = "DIR", hide = true)]
    extracted_root: Option<PathBuf>,
    /// BEAM files to load, after `--`.
    #[arg(value_name = "BEAM_FILE", required = true, num_args = 1.., last = true)]
    beam_files: Vec<PathBuf>,
}

/// The command form `pbeam erl`, which boots the node with the arguments of
/// `erl`, for example the arguments that `bin/<app> start` of a release
/// gives `erl`. The first argument `erl` selects this form, so the other
/// form cannot run a module with the name `erl`; the pinned OTP has no such
/// module.
#[derive(Parser)]
#[command(
    name = "pbeam erl",
    bin_name = "pbeam erl",
    about = "Boot the node with the preloaded modules and the arguments of erl."
)]
struct ErlCli {
    /// The root directory of the OTP installation or of the release, as the
    /// `erl` script finds it and `erlexec` gives it to the emulator with
    /// `-root DIR`. A boot script reads the `$ROOT` directories below it.
    #[arg(long, value_name = "DIR")]
    root: OsString,
    /// The `bin` directory of `erts`, as the `erl` script sets it, for
    /// example `$ROOT/erts-17.1/bin`. `erlexec` gives it to the emulator
    /// with `-bindir DIR`, and the port programs of `erts`, such as
    /// `inet_gethost`, start from it.
    #[arg(long, value_name = "DIR")]
    bindir: OsString,
    /// The directory of the BEAM files of the preloaded modules, which
    /// `erlang:pre_loaded/0` names, for example the `ebin` directory of
    /// `erts`. ERTS builds these modules into the emulator.
    #[arg(long, value_name = "DIR")]
    preloaded: PathBuf,
    /// The extracted release of an embedded run, which the boot removes at
    /// its exit. The launcher of an appended release bundle sets it; a
    /// command line does not set it.
    #[arg(long = "extracted-root", value_name = "DIR", hide = true)]
    extracted_root: Option<PathBuf>,
    /// The arguments of `erl`, after `--`.
    #[arg(value_name = "ERL_ARGUMENT", last = true)]
    arguments: Vec<OsString>,
}

/// The options that `erlexec` reads before it starts the emulator, so that
/// they change the start or the arguments of the emulator
/// (`erts/etc/common/erlexec.c:661-861`, `2122-2140`). PBeam has no
/// `erlexec` for them, and `-name` and `-sname` start the distribution,
/// which PBeam does not have, as `-proto_dist` and `-start_epmd` configure
/// it. `-emu_type` and `-emu_flavor` select the emulator, the `-smp`
/// options are checked by `erlexec` (`-smp disable` and `-smpdisable` stop
/// it), `-no_epmd` needs `-proto_dist` (`erlexec.c:499-534`, `1118-1123`),
/// and `-S` makes `erlexec` rewrite the later arguments and add `-noshell`
/// (`erlexec.c:1084-1090`, `2222-2238`). `pbeam erl` rejects each of them
/// before `-extra`. Each `erlexec` option with an operand is in the list,
/// so the first `-extra` is the one that `erlexec` reads as an option
/// (`erlexec.c:708-712`).
const ERLEXEC_OPTIONS: [&str; 27] = [
    "-S",
    "-args_file",
    "-compile",
    "-configfd",
    "-detached",
    "-emu_args",
    "-emu_args_exit",
    "-emu_flavor",
    "-emu_name_exit",
    "-emu_qouted_cmd_exit",
    "-emu_type",
    "-env",
    "-epmd",
    "-keep_window",
    "-make",
    "-man",
    "-name",
    "-no_epmd",
    "-noinput",
    "-proto_dist",
    "-smp",
    "-smpauto",
    "-smpdisable",
    "-smpenable",
    "-sname",
    "-start_epmd",
    "-version",
];

/// The environment variables of the flags that `erlexec` adds before and
/// after the arguments of the command line, and whose `-extra` arguments it
/// moves to the end (`erts/etc/common/erlexec.c:2177-2260`). `ERL_OTP29_FLAGS`
/// is the variable of the OTP release 29 (`"ERL_OTP" OTP_SYSTEM_VERSION
/// "_FLAGS"`). For `ERL_EPMD_PORT`, `erlexec` gives `-epmd_port Port` to the
/// emulator (`erlexec.c:633`, `1754-1760`), which only the distribution
/// reads.
const ERLEXEC_VARIABLES: [&str; 5] = [
    "ERL_OTP29_FLAGS",
    "ERL_AFLAGS",
    "ERL_FLAGS",
    "ERL_ZFLAGS",
    "ERL_EPMD_PORT",
];

/// Run the command line and exit with one of these statuses:
///
/// - 0: the root process returns. The binary writes nothing to stderr.
/// - 1: the root process exits with a reason other than `normal`, or it
///   exits `normal` and later no process can wake. The binary writes the one stderr line
///   `pbeam: root process exited with <Reason>`, with `<Reason>` in the ERTS
///   term text (`print_term`, erl_printf_term.c:352-811). ERTS also exits
///   with status 1 when a boot process exit halts the runtime. After a root
///   exit `normal`, a halt or SIGTERM ends the run with its own status.
/// - 1: a runtime error, a file that cannot be read, a BEAM file that does
///   not load, or a boot flag name that is not an atom of the loaded
///   modules. The binary writes the one stderr line
///   `pbeam: <message>`.
/// - the low 8 bits of the status code of `erlang:halt/2`, as the C `exit`
///   of ERTS gives (`bif.c:4346-4362`). The binary writes nothing to
///   stderr.
/// - 2: a usage error. `clap` writes the usage text to stderr.
///
/// A Rust panic (status 101) is a PBeam defect, not a BEAM result.
///
/// # Examples
///
/// ```rust,ignore
/// // $ root=$(erl -noshell -eval 'io:put_chars(code:root_dir()), halt().')
/// // $ pbeam pbeam_boot start boot_probe main --root "$root" -- pbeam_boot.beam boot_probe.beam erlang.beam ...
/// // Hello, world!
/// ```
fn main() -> ExitCode {
    // A binary with an appended bundle starts it with the arguments of the
    // command line, so a user argument such as `erl` reaches the release and
    // does not start the `erl` command below. The re-exec of the boot of an
    // extracted release marks itself with `--embedded-boot`, so it reaches
    // the commands below instead of the bundle again.
    let mut words: Vec<OsString> = std::env::args_os().collect();
    if words.len() > 1 && pbeam_runtime::embedded::is_boot_reexec(words[1].as_os_str()) {
        words.remove(1);
    } else if let Some(status) = pbeam_runtime::embedded::run(words.iter().skip(1).cloned()) {
        return status;
    }
    if words
        .get(1)
        .is_some_and(|word| word.as_os_str() == OsStr::new("erl"))
    {
        let arguments = ErlCli::parse_from(words.into_iter().skip(1));
        let extracted = arguments.extracted_root.clone();
        let status = exit_status(boot_erl(arguments));
        pbeam_runtime::embedded::remove_extraction(extracted);
        return status;
    }
    // `parse_from` reads `words`, which no longer holds the marker of a
    // re-exec; `parse` would read the environment again and see it. Its
    // first word is the program name, as `parse` takes it.
    let arguments = Cli::parse_from(words);
    let extracted = arguments.extracted_root.clone();
    let status = exit_status(run(arguments));
    // A boot of an extracted release removes its directory at the exit. The
    // flag reaches only this process, not the programs that it starts.
    pbeam_runtime::embedded::remove_extraction(extracted);
    status
}

/// Return the exit status of the end of a command line, and write the
/// message of an error to stderr.
fn exit_status(end: Result<(), Exit>) -> ExitCode {
    match end {
        Ok(()) => ExitCode::SUCCESS,
        Err(Exit::Halt(status)) => ExitCode::from(status.to_le_bytes()[0]),
        Err(Exit::Error(message)) => {
            eprintln!("pbeam: {message}");
            ExitCode::FAILURE
        }
    }
}

/// How a command line ends without a root result.
enum Exit {
    /// `erlang:halt/2` stopped the run with this status code.
    Halt(i32),
    /// An error, with the message for stderr.
    Error(String),
}

impl From<String> for Exit {
    /// Make the error of a message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(matches!(Exit::from(String::from("bad")), Exit::Error(_)));
    /// ```
    fn from(message: String) -> Self {
        Self::Error(message)
    }
}

impl From<BeamError> for Exit {
    /// Make the end of a run error: a halt gives its status code, and each
    /// other error gives its message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(matches!(Exit::from(BeamError::Halt(3)), Exit::Halt(3)));
    /// ```
    fn from(run_error: BeamError) -> Self {
        match run_error {
            BeamError::Halt(status) => Self::Halt(status),
            run_error @ BeamError::Runtime(_) => Self::Error(run_error.to_string()),
        }
    }
}

/// Set the signal dispositions of the ERTS emulator start, read and load
/// the BEAM files, and run `Module:Function(Atoms...)` as the root process,
/// or `Module:Function(Atoms..., Flags)` with the boot flag list of
/// `--root`, `--pa`, and `--mode` (see [`boot_flags`]).
/// With `--policy Policy`, the policy module `Policy` gives the
/// capabilities (see [`BeamProgram::execute_with_policy_module`]), in boot
/// mode with `--boot`. Without
/// it, and with `--no-capability-checks`, the run skips each capability
/// check (see [`BeamProgram::execute`]).
///
/// # Examples
///
/// ```rust,ignore
/// let arguments = Cli::parse_from(["pbeam", "hello", "main", "--", "hello.beam"]);
/// run(arguments)?;
/// ```
///
/// # Errors
///
/// A file that cannot be read, a BEAM file that does not load, a boot flag
/// name that is not an atom of the loaded modules, and the errors of
/// [`BeamProgram::execute`] and [`BeamProgram::execute_with_policy_module`].
/// A halt of the run gives [`Exit::Halt`].
fn run(arguments: Cli) -> Result<(), Exit> {
    install_start_dispositions();
    let mut modules = Vec::with_capacity(arguments.beam_files.len());
    for path in arguments.beam_files {
        let bytes = fs::read(&path).map_err(|read_error| {
            format!("cannot read {}: {read_error}", path.to_string_lossy())
        })?;
        modules.push(bytes);
    }
    let module_slices = modules.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let program = BeamProgram::load(&module_slices)
        .map_err(|load_error| format!("cannot load BEAM modules: {load_error}"))?;
    let mut atoms = arguments
        .arguments
        .iter()
        .map(|name| {
            program
                .put_atom(name)
                .map(|atom| program.host_term(Term::Atom(atom)))
        })
        .collect::<Result<Vec<_>, BeamError>>()?;
    if arguments.root.is_some() || !arguments.code_path.is_empty() {
        let flags = boot_flags(
            &program,
            arguments.root.as_ref(),
            &arguments.code_path,
            arguments.mode.as_ref(),
        )?;
        atoms.push(program.host_term(flags));
    }
    let (module, function) = (&arguments.module, &arguments.function);
    let result = match &arguments.policy {
        Some(policy) => {
            program.execute_with_policy_module(module, function, &atoms, policy, arguments.boot)
        }
        None => program.execute(module, function, &atoms),
    };
    run_end(result)
}

/// Return the end of a command line for the result of its run.
fn run_end(result: Result<HostTerm, BeamError>) -> Result<(), Exit> {
    result?;
    Ok(())
}

/// Boot the node as `erl` does with the arguments of `pbeam erl`: set the
/// signal dispositions of the emulator start, load the preloaded modules
/// (see [`BeamProgram::load_preloaded`]), and run [`BeamProgram::boot`]
/// with the arguments that `erlexec` gives the emulator for these `erl`
/// arguments: `-root Root -bindir Bindir -progname erl -- -home Home --`
/// and then the `erl` arguments unchanged
/// (`erts/etc/common/erlexec.c:625-640`, `1151-1161`). Without the
/// variable `HOME`, the arguments have no `-home`, as in `erlexec`.
///
/// # Examples
///
/// ```rust,ignore
/// let arguments = ErlCli::parse_from(["pbeam erl", "--root", root, "--bindir", bindir, "--preloaded", ebin, "--", "-boot", "start_clean"]);
/// boot_erl(arguments)?;
/// ```
///
/// # Errors
///
/// An `erl` option of [`ERLEXEC_OPTIONS`] or an emulator flag (an
/// argument that starts with `+`) before `-extra`, a variable of
/// [`ERLEXEC_VARIABLES`] that is set and not empty, `ERL_EPMD_PORT` also
/// when it is empty, the errors of
/// [`BeamProgram::load_preloaded`], and the errors of
/// [`BeamProgram::boot`]. A halt of the run gives [`Exit::Halt`].
fn boot_erl(arguments: ErlCli) -> Result<(), Exit> {
    // An empty flag variable gives no flag (`erlexec.c:1762-1768`), but
    // `erlexec` gives `-epmd_port` for each value of `ERL_EPMD_PORT`, also
    // an empty one (`erlexec.c:1754-1760`).
    if let Some(variable) = ERLEXEC_VARIABLES.iter().find(|variable| {
        std::env::var_os(variable)
            .is_some_and(|value| !value.is_empty() || **variable == "ERL_EPMD_PORT")
    }) {
        return Err(format!(
            "the environment variable {variable} of erl is not supported: \
             see \"Release boot\" in docs/erts-differences.md"
        )
        .into());
    }
    let (schedulers, erl_arguments) = scheduler_flags(&arguments.arguments)?;
    install_start_dispositions();
    // `erlexec` puts `bindir` and `rootdir/bin` first in `PATH` before it
    // starts the emulator (`erlexec.c:556-620`).
    add_erlexec_path(arguments.bindir.as_bytes(), arguments.root.as_bytes());
    let program = BeamProgram::load_preloaded(&arguments.preloaded)
        .map_err(|load_error| format!("cannot load the preloaded modules: {load_error}"))?;
    let home = std::env::var_os("HOME");
    let mut boot: Vec<&[u8]> = vec![
        b"-root",
        arguments.root.as_bytes(),
        b"-bindir",
        arguments.bindir.as_bytes(),
        b"-progname",
        b"erl",
        b"--",
    ];
    if let Some(home) = &home {
        boot.extend([&b"-home"[..], home.as_bytes()]);
    }
    boot.push(b"--");
    boot.extend(erl_arguments);
    run_end(Err(program.boot(&boot, schedulers)))
}

/// Take the `+S` flags out of the `erl` arguments `arguments` before
/// `-extra`, as `erlexec` gives them to the emulator and not to `init`
/// (`erlexec.c:943-966`, `1097-1100`). Return the scheduler counts that the
/// emulator sets for them (see [`Schedulers::apply_flag`]) and the other
/// arguments. The value of a flag is the rest of `+S<Value>`, or the next
/// argument of `+S`.
///
/// # Examples
///
/// ```rust,ignore
/// let (schedulers, rest) = scheduler_flags(&["+S".into(), "4:2".into(), "-noshell".into()])?;
/// assert_eq!((schedulers.total(), schedulers.online(), rest), (4, 2, vec![&b"-noshell"[..]]));
/// ```
///
/// # Errors
///
/// An `erl` option of [`ERLEXEC_OPTIONS`], an emulator flag (an argument
/// that starts with `+`) other than `+S`, `+S` without a value, and the
/// errors of [`Schedulers::apply_flag`].
fn scheduler_flags(arguments: &[OsString]) -> Result<(Schedulers, Vec<&[u8]>), Exit> {
    let mut schedulers = Schedulers::host();
    let mut rest = Vec::with_capacity(arguments.len());
    let mut options = arguments.iter().map(|argument| argument.as_bytes());
    while let Some(option) = options.next() {
        if option == b"-extra" {
            rest.push(option);
            rest.extend(options);
            break;
        }
        let value = match option.strip_prefix(b"+S") {
            Some(b"") => options.next().filter(|value| *value != b"--"),
            Some(value) if !value.starts_with(b"P") && !value.starts_with(b"D") => Some(value),
            _ => None,
        };
        if let Some(value) = value {
            schedulers.apply_flag(value)?;
        } else if option.starts_with(b"+")
            || ERLEXEC_OPTIONS.iter().any(|name| name.as_bytes() == option)
        {
            return Err(format!(
                "the erl option {} is not supported: see \"Release boot\" in \
                 docs/erts-differences.md",
                String::from_utf8_lossy(option)
            )
            .into());
        } else {
            rest.push(option);
        }
    }
    Ok((schedulers, rest))
}

/// The version of the pinned `erts` (erts/vsn.mk:23), which the `erl`
/// script puts in `BINDIR` as `%VSN%` (erl.src.src:52).
const ERTS_VSN: &str = "17.1";

/// Make the boot flag list of the `--root`, `--pa`, and `--mode` options:
/// `{root, [Root]}` first, then `{bindir, [Root/erts-Vsn/bin]}`, then one `{pa, [Dir]}` tuple for each directory
/// in the command line order, then `{mode, [Mode]}`, with each value a
/// binary of its bytes. ERTS gives the same tuples for `erl -pa D1 -pa D2
/// -mode embedded`: the `erl` script sets `BINDIR` to
/// `$ROOTDIR/erts-%VSN%/bin` (erl.src.src:52), and `erlexec` gives `-root`
/// and `-bindir` before the user flags (erlexec.c:1152-1155), the
/// emulator gives each boot argument to `init` as a binary of its bytes
/// (`erl_init.c:373-384`), and `init:parse_boot_args/1` makes one flag tuple
/// for each flag (`init.erl:1699-1702`, `1724`). A flag that the command
/// line does not have gives no tuple.
///
/// # Examples
///
/// ```rust,ignore
/// let flags = boot_flags(&program, Some(&OsString::from("/otp")), &[OsString::from("d1")], None)?;
/// // [{root, [<<"/otp">>]}, {bindir, [<<"/otp/erts-17.1/bin">>]}, {pa, [<<"d1">>]}]
/// ```
///
/// # Errors
///
/// A flag name `root`, `bindir`, `pa`, or `mode` that is not an atom of the loaded
/// modules. The pinned `init` module has each of them.
fn boot_flags(
    program: &BeamProgram,
    root: Option<&OsString>,
    code_path: &[OsString],
    mode: Option<&String>,
) -> Result<Term, String> {
    // The `--root` option gives the `bindir` flag too, as the `erl` script
    // and `erlexec` always give both.
    let bindir =
        root.map(|root| [root.as_bytes(), b"/erts-", ERTS_VSN.as_bytes(), b"/bin"].concat());
    let flags = root
        .map(|root| ("root", root.as_bytes()))
        .into_iter()
        .chain(bindir.as_deref().map(|bindir| ("bindir", bindir)))
        .chain(
            code_path
                .iter()
                .map(|directory| ("pa", directory.as_bytes())),
        )
        .chain(mode.map(|mode| ("mode", mode.as_bytes())))
        .map(|(name, value)| {
            let flag = if name == "bindir" {
                program.put_atom(name).map_err(|error| error.to_string())?
            } else {
                program.atom(name).ok_or_else(|| {
                    format!("boot flag {name} is not an atom of the loaded modules")
                })?
            };
            let value = Term::binary(value.to_vec());
            Ok(Term::tuple(vec![
                Term::Atom(flag),
                Term::list_from(vec![value]),
            ]))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Term::list_from(flags))
}
