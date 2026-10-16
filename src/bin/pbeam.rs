use std::{ffi::OsString, fs, os::unix::ffi::OsStrExt, path::PathBuf, process::ExitCode};

use clap::Parser;
use pbeam_runtime::{BeamError, BeamProgram, Term, install_start_dispositions};

#[derive(Parser)]
#[command(
    name = "pbeam",
    version,
    about = "Load BEAM files and run a function as the root process."
)]
struct Cli {
    /// Module name, such as `pbeam_boot`.
    #[arg(value_name = "MODULE")]
    module: String,
    /// Function name to run.
    #[arg(value_name = "FUNCTION")]
    function: String,
    /// Arguments of the function. Each argument is an atom that the
    /// loaded modules hold, such as a module or function name.
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
    /// BEAM files to load, after `--`.
    #[arg(value_name = "BEAM_FILE", required = true, num_args = 1.., last = true)]
    beam_files: Vec<PathBuf>,
}

/// Run the command line and exit with one of these statuses:
///
/// - 0: the root process returns. The binary writes nothing to stderr.
/// - 1: the root process exits. The binary writes the one stderr line
///   `pbeam: root process exited with <Reason>`, with `<Reason>` in the ERTS
///   term text (`print_term`, erl_printf_term.c:352-811). ERTS also exits
///   with status 1 when a boot process exit halts the runtime.
/// - 1: a runtime error, a file that cannot be read, a BEAM file that does
///   not load, or an argument or a boot flag name that is not an atom of the
///   loaded modules. The binary writes the one stderr line
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
    match run(Cli::parse()) {
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
/// A file that cannot be read, a BEAM file that does not load, an argument
/// or a boot flag name that is not an atom of the loaded modules, and the
/// errors of
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
                .atom(name)
                .map(|atom| program.host_term(Term::Atom(atom)))
                .ok_or_else(|| format!("argument {name} is not an atom of the loaded modules"))
        })
        .collect::<Result<Vec<_>, _>>()?;
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
    match result {
        Ok(_) => Ok(()),
        Err(BeamError::Halt(status)) => Err(Exit::Halt(status)),
        Err(execution_error) => Err(Exit::Error(execution_error.to_string())),
    }
}

/// Make the boot flag list of the `--root`, `--pa`, and `--mode` options:
/// `{root, [Root]}` first, then one `{pa, [Dir]}` tuple for each directory
/// in the command line order, then `{mode, [Mode]}`, with each value a
/// binary of its bytes. ERTS gives the same tuples for `erl -pa D1 -pa D2
/// -mode embedded`: `erlexec` gives `-root` before the user flags, the
/// emulator gives each boot argument to `init` as a binary of its bytes
/// (`erl_init.c:373-384`), and `init:parse_boot_args/1` makes one flag tuple
/// for each flag (`init.erl:1699-1702`, `1724`). A flag that the command
/// line does not have gives no tuple.
///
/// # Examples
///
/// ```rust,ignore
/// let flags = boot_flags(&program, Some(&OsString::from("/otp")), &[OsString::from("d1")], None)?;
/// // [{root, [<<"/otp">>]}, {pa, [<<"d1">>]}]
/// ```
///
/// # Errors
///
/// A flag name `root`, `pa`, or `mode` that is not an atom of the loaded
/// modules. The pinned `init` module has each of them.
fn boot_flags(
    program: &BeamProgram,
    root: Option<&OsString>,
    code_path: &[OsString],
    mode: Option<&String>,
) -> Result<Term, String> {
    let flags = root
        .map(|root| ("root", root.as_bytes()))
        .into_iter()
        .chain(
            code_path
                .iter()
                .map(|directory| ("pa", directory.as_bytes())),
        )
        .chain(mode.map(|mode| ("mode", mode.as_bytes())))
        .map(|(name, value)| {
            let flag = program
                .atom(name)
                .ok_or_else(|| format!("boot flag {name} is not an atom of the loaded modules"))?;
            let value = Term::binary(value.to_vec());
            Ok(Term::tuple(vec![
                Term::Atom(flag),
                Term::list_from(vec![value]),
            ]))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Term::list_from(flags))
}
