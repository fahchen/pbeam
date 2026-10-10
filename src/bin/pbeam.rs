use std::{fs, path::PathBuf, process::ExitCode};

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
    /// Without it, the root process gets all capabilities.
    #[arg(long, value_name = "MODULE")]
    policy: Option<String>,
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
///   not load, or an argument that is not an atom of the loaded modules. The
///   binary writes the one stderr line `pbeam: <message>`.
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
/// // $ pbeam pbeam_boot start boot_probe main -- pbeam_boot.beam boot_probe.beam erlang.beam ...
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
/// the BEAM files, and run `Module:Function(Atoms...)` as the root process.
/// With `--policy Policy`, the policy module `Policy` gives the
/// capabilities (see [`BeamProgram::execute_with_policy_module`]).
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
/// that is not an atom of the loaded modules, and the errors of
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
    let atoms = arguments
        .arguments
        .iter()
        .map(|name| {
            program
                .atom(name)
                .map(Term::Atom)
                .ok_or_else(|| format!("argument {name} is not an atom of the loaded modules"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (module, function) = (&arguments.module, &arguments.function);
    let result = match &arguments.policy {
        Some(policy) => program.execute_with_policy_module(module, function, &atoms, policy),
        None => program.execute(module, function, &atoms),
    };
    match result {
        Ok(_) => Ok(()),
        Err(BeamError::Halt(status)) => Err(Exit::Halt(status)),
        Err(execution_error) => Err(Exit::Error(execution_error.to_string())),
    }
}
