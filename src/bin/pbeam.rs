use std::{env, ffi::OsString, fs, process::ExitCode};

use pbeam_runtime::BeamProgram;

fn main() -> ExitCode {
    match run(env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("pbeam: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    let Some(entry_module) = arguments.next() else {
        return Err(usage());
    };
    let Some(entry_function) = arguments.next() else {
        return Err(usage());
    };
    let beam_files = arguments.collect::<Vec<_>>();
    if beam_files.is_empty() {
        return Err(usage());
    }

    let mut modules = Vec::with_capacity(beam_files.len());
    for path in beam_files {
        let bytes = fs::read(&path).map_err(|read_error| {
            format!("cannot read {}: {read_error}", path.to_string_lossy())
        })?;
        modules.push(bytes);
    }
    let module_slices = modules.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let program = BeamProgram::load(&module_slices)
        .map_err(|load_error| format!("cannot load BEAM modules: {load_error}"))?;
    let entry_module = entry_module.to_string_lossy();
    let entry_function = entry_function.to_string_lossy();
    program
        .execute(&entry_module, &entry_function, &[])
        .map_err(|execution_error| execution_error.to_string())?;
    Ok(())
}

fn usage() -> String {
    "usage: pbeam <entry-module> <entry-function> <beam-file> [beam-file ...]".into()
}
