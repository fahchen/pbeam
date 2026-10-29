//! Build one executable from the `pbeam` binary and a release directory or
//! a fixture example.
//!
//! The tool writes each file of the release directory into a bundle, with
//! its path relative to the directory as the name. It adds the BEAM files of
//! a start list and the `meta/start-modules` resource that names them, and
//! it writes the binary, the bundle, and the trailer that names the bundle.
//! A fixture example has no release directory, so the `fixture` command
//! writes the start set of the example into the layout of a release: the
//! example modules and `pbeam_boot.beam` in `pbeam/`, the consolidated
//! protocols in `releases/pbeam/consolidated/`, and each other module under
//! its lib directory. A run of the output without arguments extracts the
//! bundle and starts the entry from it. The `bundle` command runs the NIF
//! manifest gate on each BEAM file of the bundle before it writes the bundle,
//! and the `check-nifs` command runs the gate on each BEAM file of a
//! directory.

use std::{
    collections::BTreeMap,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

use pbeam_runtime::{BeamModule, BundleWriter, NifGap, nif_manifest_gaps};

/// The magic of the trailer that follows the bundle at the end of the binary.
const MAGIC: &[u8; 8] = b"PBEAMEMB";

/// The name of the resource that lists the modules of the start set.
const START_MODULES: &str = "meta/start-modules";

/// The release version of a fixture bundle. A fixture has no release
/// version, so the launcher reads this version from the synthetic
/// `releases/start_erl.data` and finds the consolidated protocols in
/// `releases/pbeam/consolidated/`.
const FIXTURE_VERSION: &str = "pbeam";

/// The usage text of the tool.
const USAGE: &str =
    "usage: pbeam_bundle bundle <release-dir> <output> <entry-module> <entry-function> <start-list>
       pbeam_bundle fixture <output> <entry-module> <entry-function> <start-list> <beams-list> <consolidated-dir>
       pbeam_bundle executable <runtime-binary> <bundle> <output>
       pbeam_bundle check-nifs <directory>";

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    let Some((command, rest)) = arguments.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let result = match command.to_str() {
        Some("bundle") => {
            if let [directory, output, module, function, start] = rest {
                build_bundle(
                    Path::new(directory),
                    Path::new(output),
                    &module.to_string_lossy(),
                    &function.to_string_lossy(),
                    Path::new(start),
                )
            } else {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
        Some("fixture") => {
            if let [output, module, function, start, beams, consolidated] = rest {
                build_fixture(
                    Path::new(output),
                    &module.to_string_lossy(),
                    &function.to_string_lossy(),
                    Path::new(start),
                    Path::new(beams),
                    Path::new(consolidated),
                )
            } else {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
        Some("executable") => {
            if let [runtime, bundle, output] = rest {
                build_executable(Path::new(runtime), Path::new(bundle), Path::new(output))
            } else {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
        Some("check-nifs") => {
            if let [directory] = rest {
                check_nifs(Path::new(directory))
            } else {
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
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
/// `output`. The bundle holds each file of the directory, so it ships each
/// NIF shared library of `lib/<app>/priv/` at its path. The NIF manifest
/// gate runs on each BEAM file of the bundle first, so a module that PBeam
/// has no library for stops the build before the output exists.
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
    nif_gate(directory, &resources)?;
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

/// Write the bundle of a fixture example, which has no release directory, to
/// `output`.
///
/// `start_list` is the start set of the run and `beams_list` is each module
/// that the run loads, as `scripts/example.sh` writes them. The example
/// modules and `pbeam_boot.beam` become `pbeam/`, the consolidated
/// protocols of `consolidated` become `releases/pbeam/consolidated/`, and
/// each other module keeps its path from the lib directory, as a release
/// has it. The synthetic `releases/start_erl.data` names the version
/// `pbeam`, so the launcher reads the same layout as the layout of a
/// release. The resource `meta/start-modules` names each start module except
/// the entry module and `pbeam_boot`, which the launcher adds itself.
fn build_fixture(
    output: &Path,
    module: &str,
    function: &str,
    start_list: &Path,
    beams_list: &Path,
    consolidated: &Path,
) -> Result<(), String> {
    let start = read_list(start_list)?;
    let beams = read_list(beams_list)?;
    let consolidated = fs::canonicalize(consolidated).map_err(|error| {
        format!(
            "cannot read the consolidated directory {}: {error}",
            consolidated.display()
        )
    })?;
    let entry_file = format!("{module}.beam");
    let example = start
        .iter()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name == std::ffi::OsStr::new(&entry_file))
        })
        .and_then(|path| fs::canonicalize(path).ok())
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .ok_or_else(|| {
            format!(
                "the start list {} has no {entry_file}",
                start_list.display()
            )
        })?;
    let mut resources = BTreeMap::new();
    let mut ebins = BTreeMap::new();
    for path in start.iter().chain(&beams) {
        add_fixture_beam(path, &example, &consolidated, &mut ebins, &mut resources)?;
    }
    // The run loads each other module of an application on demand, so the
    // bundle carries the whole ebin directory of each application that the
    // build names, as a release carries its applications.
    for (directory, prefix) in &ebins {
        add_ebin(directory, prefix, &mut resources)?;
    }
    let mut modules = Vec::new();
    for path in &start {
        let name = module_name(path);
        if name != module && name != "pbeam_boot" {
            modules.push(name);
        }
    }
    resources.insert(
        "releases/start_erl.data".to_owned(),
        format!("{} {FIXTURE_VERSION}", env!("PBEAM_OTP_RELEASE")).into_bytes(),
    );
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
    let modules = modules.join(" ");
    writer
        .add_resource(START_MODULES, modules.as_bytes())
        .map_err(|error| error.to_string())?;
    write_output(output, &writer.encode(), 0o644)
}

/// Add the BEAM file `path` of a fixture build to `resources` under the name
/// of its layout: a module of an OTP or Elixir lib directory keeps its path
/// from the lib directory, a module of the example directory `example`
/// becomes `pbeam/<file>`, and a module of `consolidated` becomes
/// `releases/pbeam/consolidated/<file>`. An OTP or Elixir module also
/// records its ebin directory in `ebins`, under the name of the directory.
fn add_fixture_beam(
    path: &Path,
    example: &Path,
    consolidated: &Path,
    ebins: &mut BTreeMap<PathBuf, String>,
    resources: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let name = if let Ok(name) = resource_name(&canonical) {
        let ebin = canonical
            .parent()
            .expect("a BEAM file of a lib directory has a parent");
        let prefix = name.rsplit_once('/').map_or("", |(prefix, _)| prefix);
        ebins.insert(ebin.to_path_buf(), prefix.to_owned());
        return Ok(());
    } else {
        let file = canonical
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if canonical.parent() == Some(example) {
            format!("pbeam/{file}")
        } else if canonical.parent() == Some(consolidated) {
            format!("releases/{FIXTURE_VERSION}/consolidated/{file}")
        } else {
            return Err(format!(
                "{} is not in an OTP lib directory, the example directory {}, or the consolidated directory {}",
                path.display(),
                example.display(),
                consolidated.display()
            ));
        }
    };
    let bytes =
        fs::read(&canonical).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    resources.insert(name, bytes);
    Ok(())
}

/// Add each file of the ebin directory `directory` to `resources` under
/// `prefix`.
fn add_ebin(
    directory: &Path,
    prefix: &str,
    resources: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let mut files = BTreeMap::new();
    collect(directory, directory, &mut files)?;
    for (name, bytes) in files {
        resources.insert(format!("{prefix}/{name}"), bytes);
    }
    Ok(())
}

/// Return each non-empty line of the list `path` as a path.
fn read_list(path: &Path) -> Result<Vec<PathBuf>, String> {
    let list = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Ok(list
        .lines()
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect())
}

/// Return the name of the module of the BEAM file `path`.
fn module_name(path: &Path) -> String {
    let file = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    file.strip_suffix(".beam").unwrap_or(&file).to_owned()
}

/// Run the NIF manifest gate on each BEAM file under `directory`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(check_nifs(Path::new("fixtures/nif_gate/missing_binding")).is_ok());
/// // The error starts "the NIF manifest gate failed:" and names
/// // "nif_gate_library: NIF library \"priv/nif_gate_library\" has no PBeam bindings".
/// assert!(check_nifs(Path::new("fixtures/nif_gate/missing_library")).is_err());
/// ```
fn check_nifs(directory: &Path) -> Result<(), String> {
    let mut files = BTreeMap::new();
    collect(directory, directory, &mut files)?;
    nif_gate(directory, &files)
}

/// Run the NIF manifest gate on each BEAM file of `files`, the files under
/// `root` by their relative names, in the order of the names. The shipped
/// libraries of a BEAM file `<dir>/ebin/<module>.beam` are the `.so` files
/// under `<dir>/priv/`, which `code:priv_dir/1` gives for the application
/// `<dir>`. The gate prints each unbound function of a bound library as a
/// report line, with the result of a call, and does not fail for it. The
/// result is an error that lists each module that PBeam has no library for
/// and each shipped library that does not load, one on each line, or the
/// error of a BEAM file that does not load.
///
/// # Examples
///
/// ```rust,ignore
/// // Prints "pbeam_bundle: the NIF manifest gate reports prim_tty:isprint/1:
/// // NIF function has no PBeam binding; a call runs the BEAM stub body".
/// assert!(nif_gate(root, &missing_binding_files).is_ok());
/// ```
fn nif_gate(root: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let mut failures = Vec::new();
    for (name, bytes) in files.iter().filter(|(name, _)| name.ends_with(".beam")) {
        let module =
            BeamModule::load(bytes).map_err(|error| format!("cannot load {name}: {error}"))?;
        let libraries = shipped_libraries(root, name, files);
        for gap in nif_manifest_gaps(&module, &libraries) {
            if matches!(gap, NifGap::Unbound { .. }) {
                println!("pbeam_bundle: the NIF manifest gate reports {gap}");
            } else {
                failures.push(gap.to_string());
            }
        }
    }
    if failures.is_empty() {
        return Ok(());
    }
    Err(format!(
        "the NIF manifest gate failed:\n{}",
        failures.join("\n")
    ))
}

/// Return the paths under `root` of the `.so` files of `files` in the
/// `priv` directory next to the `ebin` directory of the BEAM file `beam`,
/// in the order of the names. A BEAM file outside an `ebin` directory has
/// none.
///
/// # Examples
///
/// ```rust,ignore
/// // files: lib/a-1/ebin/a.beam, lib/a-1/priv/a_nif.so, lib/b-1/priv/b.so
/// assert_eq!(
///     shipped_libraries(Path::new("/r"), "lib/a-1/ebin/a.beam", &files),
///     [PathBuf::from("/r/lib/a-1/priv/a_nif.so")]
/// );
/// ```
fn shipped_libraries(root: &Path, beam: &str, files: &BTreeMap<String, Vec<u8>>) -> Vec<PathBuf> {
    let ebin = Path::new(beam).parent().unwrap_or(Path::new(""));
    if ebin.file_name() != Some(std::ffi::OsStr::new("ebin")) {
        return Vec::new();
    }
    let private = ebin.with_file_name("priv");
    files
        .keys()
        .filter(|name| name.ends_with(".so") && Path::new(name).starts_with(&private))
        .map(|name| root.join(name))
        .collect()
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
        modules.push(module_name(path));
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
