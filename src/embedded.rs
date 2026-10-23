//! The embedded mode of the `pbeam` binary: the `pbeam_bundle` tool appends
//! an application bundle to the binary, and a run without arguments starts
//! it. The mode extracts the bundle to a temporary directory and replaces
//! the process with the normal boot of the extracted release, so the
//! application needs no loose file of its own, and a signal reaches the boot
//! as it reaches a normal run. The bundle carries the OTP modules of the
//! start set, so the run needs no OTP installation.

use std::{
    env, fs,
    path::{Component, Path, PathBuf},
    process::{Command, ExitCode},
};

use crate::Bundle;

/// The magic of the trailer that follows the bundle at the end of the binary.
const MAGIC: &[u8; 8] = b"PBEAMEMB";

/// The size of the trailer: the magic and the length of the bundle, a `u64`.
const TRAILER: usize = 16;

/// What the end of `bytes` holds.
#[derive(Debug, PartialEq, Eq)]
enum Trailer<'a> {
    /// The bytes end without a trailer.
    Absent,
    /// The trailer names a bundle that does not fit before it.
    Invalid,
    /// The bundle that the trailer names.
    Bundle(&'a [u8]),
}

/// Run the bundle that is appended to the binary. Return `None` when the
/// binary holds no bundle.
///
/// # Examples
///
/// ```rust,ignore
/// // A plain binary holds no bundle.
/// assert!(run().is_none());
/// ```
pub fn run() -> Option<ExitCode> {
    let executable = env::current_exe().ok()?;
    let bytes = fs::read(&executable).ok()?;
    run_bundle(&executable, &bytes, &env::temp_dir())
}

/// Remove the extracted directory at the end of an embedded run. The boot
/// of an extracted release gets the directory with `--extracted-root`, so
/// the flag reaches no program that the run starts.
///
/// # Examples
///
/// ```rust,ignore
/// // A run that a launcher did not start has no directory.
/// remove_extraction(None);
/// ```
pub fn remove_extraction(directory: Option<PathBuf>) {
    if let Some(directory) = directory {
        let _ = fs::remove_dir_all(directory);
    }
}

/// Run the bundle of `bytes`, with `base` for the extraction. Return `None`
/// when the bytes end without a trailer.
fn run_bundle(executable: &Path, bytes: &[u8], base: &Path) -> Option<ExitCode> {
    let bundle_bytes = match trailer(bytes) {
        Trailer::Absent => return None,
        Trailer::Invalid => return Some(failure("the embedded trailer is invalid")),
        Trailer::Bundle(bundle) => bundle,
    };
    let Some(bundle) = Bundle::parse(bundle_bytes).ok() else {
        return Some(failure("the embedded bundle does not parse"));
    };
    let directory = match temp_directory(base) {
        Ok(directory) => directory,
        Err(message) => return Some(failure(&message)),
    };
    Some(start(executable, &bundle, &directory))
}

/// Return the bundle that the trailer at the end of `bytes` names.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(trailer(b"plain binary"), Trailer::Absent);
/// ```
fn trailer(bytes: &[u8]) -> Trailer<'_> {
    let Some(end) = bytes.len().checked_sub(TRAILER) else {
        return Trailer::Absent;
    };
    if &bytes[end..end + MAGIC.len()] != MAGIC {
        return Trailer::Absent;
    }
    let length = u64::from_le_bytes(bytes[end + MAGIC.len()..].try_into().unwrap()) as usize;
    match end.checked_sub(length) {
        Some(start) => Trailer::Bundle(&bytes[start..end]),
        None => Trailer::Invalid,
    }
}

/// Extract the bundle to `directory`, then replace this process with the
/// boot of its entry: the node boot of a release with a boot script, or the
/// `pbeam_boot` start of a fixture bundle. Return the exit status of a run
/// that fails before the boot.
fn start(executable: &Path, bundle: &Bundle<'_>, directory: &Path) -> ExitCode {
    let result = extract(directory, bundle).and_then(|()| match release_boot(directory) {
        Some((erts, version)) => Ok(boot_release(
            executable,
            bundle.entry(),
            directory,
            &erts,
            &version,
        )),
        None => start_modules(bundle)
            .and_then(|modules| boot(executable, bundle.entry(), &modules, directory)),
    });
    remove(directory);
    match result {
        Ok(error) => failure(&format!("cannot start the embedded run: {error}")),
        Err(message) => failure(&message),
    }
}

/// Write each resource of `bundle` below `directory`.
fn extract(directory: &Path, bundle: &Bundle<'_>) -> Result<(), String> {
    for blob in bundle.resources() {
        let path = bundle_path(directory, blob.name())?;
        let parent = path.parent().expect("a resource path has a parent");
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot make {}: {error}", parent.display()))?;
        fs::write(&path, blob.bytes())
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        // The bundle keeps no file mode, and the boot starts the port
        // programs of `erts-<Vsn>/bin`, such as `inet_gethost`.
        if is_erts_program(blob.name()) {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .map_err(|error| format!("cannot set the mode of {}: {error}", path.display()))?;
        }
    }
    Ok(())
}

/// Return whether the resource `name` is a file of the `bin` directory of
/// `erts`, such as `erts-17.1/bin/inet_gethost`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_erts_program("erts-17.1/bin/inet_gethost"));
/// assert!(!is_erts_program("lib/erts-17.1/ebin/init.beam"));
/// ```
fn is_erts_program(name: &str) -> bool {
    let mut components = name.split('/');
    components
        .next()
        .is_some_and(|first| first.starts_with("erts-"))
        && components.next() == Some("bin")
}

/// Return the modules of the start set of `bundle`, from the resource
/// `meta/start-modules`.
fn start_modules(bundle: &Bundle<'_>) -> Result<Vec<String>, String> {
    let modules = bundle
        .resource("meta/start-modules")
        .ok_or("the bundle has no meta/start-modules resource")?;
    Ok(String::from_utf8_lossy(modules)
        .split_ascii_whitespace()
        .map(ToOwned::to_owned)
        .collect())
}

/// Replace the process with the boot of `entry` in `directory`. Return the
/// error of a command that the host does not start.
fn boot(
    executable: &Path,
    entry: crate::Entry<'_>,
    modules: &[String],
    directory: &Path,
) -> Result<std::io::Error, String> {
    use std::os::unix::process::CommandExt;

    let mut beams = vec![
        directory
            .join("pbeam")
            .join(format!("{}.beam", entry.module)),
        directory.join("pbeam").join("pbeam_boot.beam"),
    ];
    for module in modules {
        beams.push(resolve_module(directory, module)?);
    }
    let version = release_version(directory)?;
    let mut command = Command::new(executable);
    command.args(["pbeam_boot", "start", entry.module, entry.function]);
    command.arg("--root").arg(directory);
    command.arg("--extracted-root").arg(directory);
    for path in code_paths(directory, &version)? {
        command.arg("--pa").arg(path);
    }
    command.arg("--").args(&beams);
    command.env("RELEASE_ROOT", directory);
    command.env(
        "RELEASE_SYS_CONFIG",
        directory.join("releases").join(&version).join("sys"),
    );
    Ok(command.exec())
}

/// Return the `erts` version and the release version of the release in
/// `directory` when the release has the boot script
/// `releases/<Vsn>/start.boot`, as `mix release` writes it. A fixture
/// bundle has no boot script.
fn release_boot(directory: &Path) -> Option<(String, String)> {
    let (erts, version) = start_erl_data(directory).ok()?;
    let boot = directory.join("releases").join(&version).join("start.boot");
    boot.is_file().then_some((erts, version))
}

/// Replace the process with the node boot of the release in `directory`,
/// with the `erl` arguments that `bin/<name> start` of a Mix release gives
/// when `RELEASE_DISTRIBUTION` is `none`: `-s <Module> <Function>` of the
/// entry, the `embedded` mode, `sys.config`, the boot script `start`, the
/// boot variable `RELEASE_LIB`, and the plain argument `--no-halt`. The
/// words of `ELIXIR_ERL_OPTIONS` come after `-noshell`, as the `elixir`
/// script of the release gives them unquoted (`elixir:229`), so
/// `ELIXIR_ERL_OPTIONS="+S 4"` starts 4 schedulers. The
/// run gets `RELEASE_ROOT` and `RELEASE_SYS_CONFIG`, which the release
/// boot reads. The preloaded modules come from `lib/erts-<Vsn>/ebin`,
/// where the bundle tool writes the start set. Return the error of a
/// command that the host does not start.
fn boot_release(
    executable: &Path,
    entry: crate::Entry<'_>,
    directory: &Path,
    erts: &str,
    version: &str,
) -> std::io::Error {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt, os::unix::process::CommandExt};

    let erts = format!("erts-{erts}");
    let release = directory.join("releases").join(version);
    let mut command = Command::new(executable);
    command.arg("erl").arg("--root").arg(directory);
    command
        .arg("--bindir")
        .arg(directory.join(&erts).join("bin"));
    command
        .arg("--preloaded")
        .arg(directory.join("lib").join(&erts).join("ebin"));
    command.arg("--extracted-root").arg(directory);
    command.args(["--", "-noshell"]);
    if let Some(options) = env::var_os("ELIXIR_ERL_OPTIONS") {
        command.args(
            options
                .as_bytes()
                .split(u8::is_ascii_whitespace)
                .filter(|word| !word.is_empty())
                .map(OsStr::from_bytes),
        );
    }
    command.args(["-s", entry.module, entry.function]);
    command.args(["-mode", "embedded", "-config"]);
    command.arg(release.join("sys"));
    command.arg("-boot").arg(release.join("start"));
    command.args(["-boot_var", "RELEASE_LIB"]);
    command.arg(directory.join("lib"));
    command.args(["-extra", "--no-halt"]);
    command.env("RELEASE_ROOT", directory);
    command.env("RELEASE_SYS_CONFIG", release.join("sys"));
    command.exec()
}

/// Remove the extracted `directory`, and leave a directory that stays.
fn remove(directory: &Path) {
    let _ = fs::remove_dir_all(directory);
}

/// Return [`ExitCode::FAILURE`] after a message on stderr.
fn failure(message: &str) -> ExitCode {
    eprintln!("pbeam: {message}");
    ExitCode::FAILURE
}

/// Return a new temporary directory for the extraction, as the first free
/// `pbeam-embedded-<pid>-<attempt>` directory under `base`.
fn temp_directory(base: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    for attempt in 0..100 {
        let directory = base.join(format!("pbeam-embedded-{}-{attempt}", std::process::id()));
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "cannot make {}: {error}",
                    directory.to_string_lossy()
                ));
            }
        }
    }
    Err("cannot make a temporary directory".to_owned())
}

/// Return the path of the resource `name` under `directory`, or an error
/// for a name that leaves `directory`.
fn bundle_path(directory: &Path, name: &str) -> Result<PathBuf, String> {
    let path = Path::new(name);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("the bundle resource {name} is not a relative path"));
    }
    Ok(directory.join(path))
}

/// Return the path of the boot module `module` under the extracted release
/// `root`, as `erl` finds it in `lib/*/ebin` and `erts-*/ebin`. The modules
/// of a fixture bundle are in `pbeam/`, and its consolidated protocols are
/// in `releases/*/consolidated/`; both come before the lib directories, so
/// a consolidated protocol comes before the unconsolidated protocol of the
/// same name.
fn resolve_module(root: &Path, module: &str) -> Result<PathBuf, String> {
    let file = format!("{module}.beam");
    let pbeam = root.join("pbeam").join(&file);
    if pbeam.is_file() {
        return Ok(pbeam);
    }
    let releases = root.join("releases");
    if let Ok(entries) = fs::read_dir(&releases) {
        for entry in entries {
            let consolidated = entry
                .map_err(|error| error.to_string())?
                .path()
                .join("consolidated")
                .join(&file);
            if consolidated.is_file() {
                return Ok(consolidated);
            }
        }
    }
    let mut candidates = Vec::new();
    for entry in
        fs::read_dir(root).map_err(|error| format!("cannot read {}: {error}", root.display()))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name.starts_with("erts-") {
            candidates.push(path.join("ebin").join(&file));
        } else if name == "lib" {
            for application in fs::read_dir(&path).map_err(|error| error.to_string())? {
                candidates.push(
                    application
                        .map_err(|error| error.to_string())?
                        .path()
                        .join("ebin")
                        .join(&file),
                );
            }
        }
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("the release {root:?} has no module {module}"))
}

/// Return the version of the release at `directory` from
/// `releases/start_erl.data`, as `bin/plug_hello` reads it.
fn release_version(directory: &Path) -> Result<String, String> {
    start_erl_data(directory).map(|(_, version)| version)
}

/// Return the `erts` version and the release version of the release at
/// `directory`: the two fields of `releases/start_erl.data`.
fn start_erl_data(directory: &Path) -> Result<(String, String), String> {
    let path = directory.join("releases").join("start_erl.data");
    let data = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut fields = data.split_ascii_whitespace().map(ToOwned::to_owned);
    fields
        .next()
        .zip(fields.next())
        .ok_or_else(|| format!("{} has no release version", path.display()))
}

/// Return the code path directories of the release at `directory`: the
/// compiled entry, each `lib/*/ebin`, and the consolidated protocols.
fn code_paths(directory: &Path, version: &str) -> Result<Vec<PathBuf>, String> {
    let mut paths = vec![directory.join("pbeam")];
    let lib = directory.join("lib");
    for application in
        fs::read_dir(&lib).map_err(|error| format!("cannot read {}: {error}", lib.display()))?
    {
        paths.push(
            application
                .map_err(|error| error.to_string())?
                .path()
                .join("ebin"),
        );
    }
    paths.push(
        directory
            .join("releases")
            .join(version)
            .join("consolidated"),
    );
    Ok(paths)
}

#[cfg(test)]
#[path = "../tests/embedded.rs"]
mod tests;
