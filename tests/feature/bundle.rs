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
        .args([&release, Path::new("pbeam"), &directory.join("out")])
        .args(["m", "f"])
        .arg(&start));
    assert_eq!(status, Some(1));
    assert!(
        stderr.contains("is not a file of an OTP lib directory"),
        "{stderr}"
    );

    let start = start_list(&directory, &["/tmp/lib/app/ebin/missing.beam"]);
    let (status, stderr) = run(tool()
        .args([&release, Path::new("pbeam"), &directory.join("out")])
        .args(["m", "f"])
        .arg(&start));
    assert_eq!(status, Some(1));
    assert!(stderr.contains("cannot read"), "{stderr}");

    let (status, stderr) = run(tool()
        .arg(directory.join("missing"))
        .args([
            Path::new("pbeam"),
            &directory.join("out"),
            Path::new("m"),
            Path::new("f"),
        ])
        .arg(&start));
    assert_eq!(status, Some(1));
    assert!(stderr.contains("cannot read"), "{stderr}");
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
    let runtime = directory.join("pbeam");
    fs::write(&runtime, b"binary").unwrap();
    let start = start_list(&directory, &[ebin.join("init.beam").to_str().unwrap()]);
    let output = directory.join("out");
    let (status, stderr) = run(tool()
        .arg(&release)
        .arg(&runtime)
        .arg(&output)
        .args(["m", "f"])
        .arg(&start));
    assert_eq!(status, Some(0), "{stderr}");
    let bytes = fs::read(&output).unwrap();
    let end = bytes.len() - 16;
    let length = u64::from_le_bytes(bytes[end + 8..].try_into().unwrap()) as usize;
    let bundle = pbeam_runtime::Bundle::parse(&bytes[end - length..end]).unwrap();
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
fn reports_an_output_that_the_host_cannot_write() {
    let directory = directory("output");
    let release = directory.join("release");
    fs::create_dir(&release).unwrap();
    let module = directory.join("lib/app/ebin/m.beam");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, b"beam").unwrap();
    let runtime = directory.join("pbeam");
    fs::write(&runtime, b"binary").unwrap();
    let start = start_list(&directory, &[module.to_str().unwrap()]);
    let (status, stderr) = run(tool()
        .arg(&release)
        .arg(&runtime)
        .arg(directory.join("missing/out"))
        .args(["m", "f"])
        .arg(&start));
    assert_eq!(status, Some(1));
    assert!(stderr.contains("cannot write"), "{stderr}");
    fs::remove_dir_all(&directory).unwrap();
}
