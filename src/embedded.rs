//! The embedded mode of the `pbeam` binary: the `pbeam_bundle` tool appends
//! an application bundle to the binary, and a run without arguments starts
//! it. The mode extracts the bundle to a temporary directory and starts the
//! normal boot with the extracted files, so the application needs no loose
//! file of its own. The bundle carries the OTP modules of the start set, so
//! the run needs no OTP installation.

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
    let bundle_bytes = trailer_bundle(&bytes)?;
    let Some(bundle) = Bundle::parse(bundle_bytes).ok() else {
        eprintln!("pbeam: the embedded bundle does not parse");
        return Some(ExitCode::FAILURE);
    };
    Some(match start_bundle(&executable, &bundle) {
        Ok(status) => ExitCode::from(status),
        Err(message) => {
            eprintln!("pbeam: {message}");
            ExitCode::FAILURE
        }
    })
}

/// Return the bundle that the trailer at the end of `bytes` names, or
/// `None` when `bytes` ends without a trailer or with a short bundle.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(trailer_bundle(b"plain binary").is_none());
/// ```
fn trailer_bundle(bytes: &[u8]) -> Option<&[u8]> {
    let end = bytes.len().checked_sub(TRAILER)?;
    if &bytes[end..end + MAGIC.len()] != MAGIC {
        return None;
    }
    let length = u64::from_le_bytes(bytes[end + MAGIC.len()..].try_into().ok()?) as usize;
    let start = end.checked_sub(length)?;
    Some(&bytes[start..end])
}

/// Extract the bundle to a temporary directory and start the boot of the
/// entry with the extracted files as the OTP root. Return the exit status of
/// the run.
fn start_bundle(executable: &Path, bundle: &Bundle<'_>) -> Result<u8, String> {
    let directory = temp_directory(&env::temp_dir())?;
    let result = start_in(executable, bundle, &directory);
    // A temporary directory that stays behind changes nothing for the run.
    let _ = fs::remove_dir_all(&directory);
    result
}

/// Extract the bundle to `directory` and run the entry there.
fn start_in(executable: &Path, bundle: &Bundle<'_>, directory: &Path) -> Result<u8, String> {
    for blob in bundle.resources() {
        let path = bundle_path(directory, blob.name())?;
        let parent = path.parent().expect("a resource path has a parent");
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot make {}: {error}", parent.display()))?;
        fs::write(&path, blob.bytes())
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    let entry = bundle.entry();
    let mut beams = vec![
        directory
            .join("pbeam")
            .join(format!("{}.beam", entry.module)),
        directory.join("pbeam").join("pbeam_boot.beam"),
    ];
    let modules = bundle
        .resource("meta/start-modules")
        .ok_or("the bundle has no meta/start-modules resource")?;
    for module in String::from_utf8_lossy(modules).split_ascii_whitespace() {
        beams.push(resolve_module(directory, module)?);
    }
    let version = release_version(directory)?;
    let mut command = Command::new(executable);
    command.args(["pbeam_boot", "start", entry.module, entry.function]);
    command.arg("--root").arg(directory);
    for path in code_paths(directory, &version)? {
        command.arg("--pa").arg(path);
    }
    command.arg("--").args(&beams);
    command.env("RELEASE_ROOT", directory);
    command.env(
        "RELEASE_SYS_CONFIG",
        directory.join("releases").join(&version).join("sys"),
    );
    let status = command
        .status()
        .map_err(|error| format!("cannot start the embedded run: {error}"))?;
    Ok(status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1))
}

/// Return a new temporary directory for the extraction, as the first free
/// `pbeam-embedded-<pid>-<attempt>` directory under `base`.
fn temp_directory(base: &Path) -> Result<PathBuf, String> {
    for attempt in 0..100 {
        let directory = base.join(format!("pbeam-embedded-{}-{attempt}", std::process::id()));
        match fs::create_dir(&directory) {
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
/// `root`, as `erl` finds it in `lib/*/ebin` and `erts-*/ebin`.
fn resolve_module(root: &Path, module: &str) -> Result<PathBuf, String> {
    let file = format!("{module}.beam");
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
    let path = directory.join("releases").join("start_erl.data");
    let data = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    data.split_ascii_whitespace()
        .nth(1)
        .map(ToOwned::to_owned)
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
