use std::{fs, path::PathBuf, process::ExitCode};

use clap::Parser;
use pbeam_runtime::BeamProgram;

#[derive(Parser)]
#[command(
    name = "pbeam",
    version,
    about = "Run a zero-argument function from BEAM files."
)]
struct Cli {
    /// Module name, such as `Elixir.Hello`.
    #[arg(value_name = "ENTRY_MODULE")]
    entry_module: String,
    /// Function name to run.
    #[arg(value_name = "ENTRY_FUNCTION")]
    entry_function: String,
    /// BEAM files to load.
    #[arg(value_name = "BEAM_FILE", required = true, num_args = 1..)]
    beam_files: Vec<PathBuf>,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("pbeam: {message}");
            ExitCode::FAILURE
        }
    }
}

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
    program
        .execute(&arguments.entry_module, &arguments.entry_function, &[])
        .map_err(|execution_error| execution_error.to_string())?;
    Ok(())
}
