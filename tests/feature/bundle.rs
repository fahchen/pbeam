//! The errors of the `pbeam_bundle` tool: the usage, the input files, and
//! the output file.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Return a new directory for one test.
fn directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("pbeam-bundle-tool-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// Return the command of the tool.
fn tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pbeam_bundle"))
}

/// Run `command` and return its exit code and its stderr.
fn run(command: &mut Command) -> (Option<i32>, String) {
    let output = command.output().expect("the tool runs");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Return the build directory of `examples/plug_hello`.
fn release_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/examples/plug_hello")
}

/// Write a start list with `lines` and return its path.
fn start_list(directory: &Path, lines: &[&str]) -> PathBuf {
    let path = directory.join("start.list");
    fs::write(&path, lines.join("\n")).unwrap();
    path
}

#[test]
fn reports_the_usage_and_the_input_files() {
    let directory = directory("errors");
    let (status, stderr) = run(&mut tool());
    assert_eq!(status, Some(1));
    assert!(stderr.starts_with("usage:"), "{stderr}");

    let release = directory.join("release");
    fs::create_dir(&release).unwrap();
    let start = start_list(&directory, &["/etc/hosts"]);
    let (status, stderr) = run(tool()
        .arg("bundle")
        .arg(&release)
        .arg(directory.join("out"))
        .args(["m", "f"])
        .arg(&start));
    assert_eq!(status, Some(1));
    assert!(
        stderr.contains("is not a file of an OTP lib directory"),
        "{stderr}"
    );

    let start = start_list(&directory, &["/tmp/lib/app/ebin/missing.beam"]);
    let (status, stderr) = run(tool()
        .arg("bundle")
        .arg(&release)
        .arg(directory.join("out"))
        .args(["m", "f"])
        .arg(&start));
    assert_eq!(status, Some(1));
    assert!(stderr.contains("cannot read"), "{stderr}");

    let (status, stderr) = run(tool()
        .arg("bundle")
        .arg(directory.join("missing"))
        .arg(directory.join("out"))
        .args(["m", "f"])
        .arg(&start));
    assert_eq!(status, Some(1));
    assert!(stderr.contains("cannot read"), "{stderr}");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn rejects_a_release_directory_for_a_build() {
    let output = Command::new("bash")
        .arg("scripts/example.sh")
        .args(["plug_hello", "--release", "/tmp/other"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("scripts/example.sh runs");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--release needs --no-build"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn writes_the_bundle_from_a_mix_task() {
    // The task builds the tool, resolves the start set with
    // `scripts/example.sh --no-build --write-start`, and writes one bundle.
    // The check builds the release of examples/plug_hello before the Rust
    // tests.
    let directory = directory("mix-task");
    let output = directory.join("plug_hello.pbeamb");
    let start = directory.join("start.list");
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plug_hello");
    // The task compiles the entry stubs, so the release needs no earlier
    // output of the example runner.
    let release = directory.join("release");
    let status = Command::new("cp")
        .arg("-R")
        .arg(release_directory())
        .arg(&release)
        .status()
        .unwrap();
    assert!(status.success());
    // A release from `mix release` has no `pbeam` directory.
    let _ = fs::remove_dir_all(release.join("pbeam"));
    let mut command = Command::new("mix");
    command
        .args(["pbeam.bundle", "--output"])
        .arg(&output)
        .arg("--start-list")
        .arg(&start)
        .arg("--release")
        .arg(&release)
        .arg("--tool")
        .arg(env!("CARGO_BIN_EXE_pbeam_bundle"))
        .current_dir(&example)
        .env("MIX_ENV", "prod");
    let (status, stderr) = run(&mut command);
    assert_eq!(status, Some(0), "{stderr}");
    assert!(release.join("pbeam/plug_hello_run.beam").is_file());
    assert!(release.join("pbeam/pbeam_boot.beam").is_file());
    let bytes = fs::read(&output).unwrap();
    let bundle = pbeam_runtime::Bundle::parse(&bytes).unwrap();
    let names: Vec<_> = bundle
        .resources()
        .iter()
        .map(|blob| blob.name().to_owned())
        .collect();
    assert!(names.iter().any(|name| name == "meta/start-modules"));
    assert!(names.iter().any(|name| name == "pbeam/plug_hello_run.beam"));
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("lib/erts-") && name.ends_with("/ebin/init.beam"))
    );
    assert_eq!(bundle.entry().module, "plug_hello_run");
    // The payloads are the bytes of the release files.
    let entry = release.join("pbeam/plug_hello_run.beam");
    assert_eq!(
        bundle
            .resource("pbeam/plug_hello_run.beam")
            .expect("the entry is a resource"),
        fs::read(&entry).unwrap()
    );
    let application = fs::read_dir(release.join("lib"))
        .unwrap()
        .map(|app| app.unwrap().path().join("ebin"))
        .find_map(|ebin| {
            fs::read_dir(ebin).ok()?.find_map(|entry| {
                let path = entry.ok()?.path();
                (path
                    .extension()
                    .is_some_and(|extension| extension == "beam"))
                .then_some(path)
            })
        })
        .expect("the release holds a BEAM file of an application");
    let name = application
        .strip_prefix(&release)
        .unwrap()
        .to_str()
        .unwrap()
        .replace(std::path::MAIN_SEPARATOR, "/");
    assert_eq!(
        bundle.resource(&name).expect("bandit is a resource"),
        fs::read(&application).unwrap()
    );
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn keeps_the_last_lib_suffix_of_a_start_file() {
    // An OTP installation under `/usr/local/lib/erlang` has another `lib`
    // component before the suffix of its applications.
    let directory = directory("lib-suffix");
    let release = directory.join("release");
    fs::create_dir(&release).unwrap();
    let ebin = directory.join("lib/otp/lib/erts-17/ebin");
    fs::create_dir_all(&ebin).unwrap();
    fs::write(ebin.join("init.beam"), b"beam").unwrap();
    fs::write(ebin.join("extra.beam"), b"extra").unwrap();
    let start = start_list(&directory, &[ebin.join("init.beam").to_str().unwrap()]);
    let output = directory.join("out");
    let (status, stderr) = run(tool()
        .arg("bundle")
        .arg(&release)
        .arg(&output)
        .args(["m", "f"])
        .arg(&start));
    assert_eq!(status, Some(0), "{stderr}");
    let bytes = fs::read(&output).unwrap();
    let bundle = pbeam_runtime::Bundle::parse(&bytes).unwrap();
    assert!(bundle.resource("lib/erts-17/ebin/init.beam").is_some());
    assert!(bundle.resource("lib/erts-17/ebin/extra.beam").is_some());
    assert!(
        bundle
            .resource("lib/otp/lib/erts-17/ebin/init.beam")
            .is_none()
    );
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn reports_the_usage_of_each_command() {
    for arguments in [
        vec!["bundle"],
        vec!["bundle", "only"],
        vec!["executable"],
        vec!["unknown"],
    ] {
        let (status, stderr) = run(tool().args(&arguments));
        assert_eq!(status, Some(1), "{arguments:?}");
        assert!(stderr.starts_with("usage:"), "{arguments:?}: {stderr}");
    }
}

#[test]
fn reports_an_output_that_the_host_cannot_write() {
    let directory = directory("output");
    let release = directory.join("release");
    fs::create_dir(&release).unwrap();
    let module = directory.join("lib/app/ebin/m.beam");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, b"beam").unwrap();
    let start = start_list(&directory, &[module.to_str().unwrap()]);
    let (status, stderr) = run(tool()
        .arg("bundle")
        .arg(&release)
        .arg(directory.join("missing/out"))
        .args(["m", "f"])
        .arg(&start));
    assert_eq!(status, Some(1));
    assert!(stderr.contains("cannot write"), "{stderr}");
    fs::remove_dir_all(&directory).unwrap();
}
