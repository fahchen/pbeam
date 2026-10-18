//! Build one executable from the `pbeam` binary and a release directory.
//!
//! The tool writes each file of the release directory into a bundle, with
//! its path relative to the directory as the name. It adds the BEAM files of
//! a start list and the `meta/start-modules` resource that names them, and
//! it writes the binary, the bundle, and the trailer that names the bundle.
//! A run of the output without arguments extracts the bundle and starts the
//! release from it.

use std::{
    collections::BTreeMap,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

use pbeam_runtime::BundleWriter;

/// The magic of the trailer that follows the bundle at the end of the binary.
const MAGIC: &[u8; 8] = b"PBEAMEMB";

/// The name of the resource that lists the modules of the start set.
const START_MODULES: &str = "meta/start-modules";

/// The usage text of the tool.
const USAGE: &str =
    "usage: pbeam_bundle bundle <release-dir> <output> <entry-module> <entry-function> <start-list>
       pbeam_bundle executable <runtime-binary> <bundle> <output>";

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    let Some((command, rest)) = arguments.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let result = match command.to_str() {
        Some("bundle") => match rest {
            [directory, output, module, function, start] => build_bundle(
                Path::new(directory),
                Path::new(output),
                &module.to_string_lossy(),
                &function.to_string_lossy(),
                Path::new(start),
            ),
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        },
        Some("executable") => match rest {
            [runtime, bundle, output] => {
                build_executable(Path::new(runtime), Path::new(bundle), Path::new(output))
            }
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        },
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("pbeam_bundle: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Write the bundle of `directory`, with the start set of `start_list`, to
/// `output`.
fn build_bundle(
    directory: &Path,
    output: &Path,
    module: &str,
    function: &str,
    start_list: &Path,
) -> Result<(), String> {
    let mut resources = BTreeMap::new();
    collect(directory, directory, &mut resources)?;
    let start = start_modules(start_list, &mut resources)?;
    let mut writer = BundleWriter::new(
        env!("PBEAM_OTP_RELEASE"),
        "pbeam_bundle",
        module,
        function,
        0,
    )
    .map_err(|error| error.to_string())?;
    for (name, bytes) in &resources {
        writer
            .add_resource(name, bytes)
            .map_err(|error| error.to_string())?;
    }
    writer
        .add_resource(START_MODULES, start.as_bytes())
        .map_err(|error| error.to_string())?;
    write_output(output, &writer.encode(), 0o644)
}

/// Write the binary, the bundle of `bundle`, and the trailer to `output`.
fn build_executable(runtime: &Path, bundle: &Path, output: &Path) -> Result<(), String> {
    let mut file = fs::read(runtime)
        .map_err(|error| format!("cannot read the binary {}: {error}", runtime.display()))?;
    let bundle = fs::read(bundle)
        .map_err(|error| format!("cannot read the bundle {}: {error}", bundle.display()))?;
    file.extend_from_slice(&bundle);
    file.extend_from_slice(MAGIC);
    file.extend_from_slice(&u64::try_from(bundle.len()).unwrap().to_le_bytes());
    write_output(output, &file, 0o755)
}

/// Write `bytes` to `output` with `mode`.
fn write_output(output: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let mut file = fs::File::create(output)
        .map_err(|error| format!("cannot write {}: {error}", output.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("cannot write {}: {error}", output.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(output, fs::Permissions::from_mode(mode))
            .map_err(|error| format!("cannot set the mode of {}: {error}", output.display()))?;
    }
    Ok(())
}

/// Add each file under `directory` to `resources`, with its path relative to
/// `root` as the name.
fn collect(
    root: &Path,
    directory: &Path,
    resources: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
    let mut paths: Vec<PathBuf> = entries
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| format!("cannot read {}: {error}", directory.display()))
        })
        .collect::<Result<_, _>>()?;
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect(root, &path, resources)?;
        } else {
            let name = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            let bytes = fs::read(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            resources.insert(name, bytes);
        }
    }
    Ok(())
}

/// Add each BEAM file of `start_list` to `resources` under its path in the
/// OTP installation, and return the names of the modules.
fn start_modules(
    start_list: &Path,
    resources: &mut BTreeMap<String, Vec<u8>>,
) -> Result<String, String> {
    let list = fs::read_to_string(start_list)
        .map_err(|error| format!("cannot read {}: {error}", start_list.display()))?;
    let mut modules = Vec::new();
    let mut erts_directories = BTreeMap::new();
    for line in list.lines().filter(|line| !line.is_empty()) {
        let path = Path::new(line);
        let name = resource_name(path)?;
        let bytes =
            fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let file = name.rsplit('/').next().unwrap_or_default();
        modules.push(file.strip_suffix(".beam").unwrap_or(file).to_owned());
        if name.starts_with("lib/erts-")
            && let Some(parent) = path.parent()
        {
            erts_directories.insert(parent.to_owned(), ());
        }
        resources.insert(name, bytes);
    }
    // The release carries each application of OTP. The erts application is
    // the exception: copy all of it, because the boot loads modules that the
    // start set does not name.
    for directory in erts_directories.keys() {
        for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            let name = resource_name(&path)?;
            let bytes = fs::read(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            resources.insert(name, bytes);
        }
    }
    Ok(modules.join(" "))
}

/// Return the name of a start file in the OTP installation: its path from
/// the `lib` directory, for example `lib/erts-17.1/ebin/init.beam`.
fn resource_name(path: &Path) -> Result<String, String> {
    // The suffix is `<lib>/<application>/ebin/<file>`. An OTP installation
    // under a path such as `/usr/local/lib/erlang` has another `lib`
    // component before it.
    let components: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    let suffix = components.len().checked_sub(4);
    match suffix {
        Some(start) if components[start] == "lib" && components[start + 2] == "ebin" => {
            Ok(components[start..].join("/"))
        }
        _ => Err(format!(
            "{} is not a file of an OTP lib directory",
            path.display()
        )),
    }
}
