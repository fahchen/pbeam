use std::{fs, path::PathBuf, process::ExitCode};

use clap::Parser;
use pbeam_runtime::{BeamProgram, Term};

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
    /// BEAM files to load, after `--`.
    #[arg(value_name = "BEAM_FILE", required = true, num_args = 1.., last = true)]
    beam_files: Vec<PathBuf>,
}

/// Run the command line. Print an error to stderr and exit with status 1
/// when loading or execution fails.
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
        Err(message) => {
            eprintln!("pbeam: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Read and load the BEAM files, and run `Module:Function(Atoms...)` as the
/// root process.
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
/// [`BeamProgram::execute`].
fn run(arguments: Cli) -> Result<(), String> {
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
    program
        .execute(&arguments.module, &arguments.function, &atoms)
        .map(|_| ())
        .map_err(|execution_error| execution_error.to_string())
}
