//! The release build and the ERTS version check of `mix pbeam.build`: the
//! task builds the release itself, and it stops on a release of another
//! ERTS version.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Return a new directory for one test.
fn directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("pbeam-build-task-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// Run `command` and return its exit code, its standard output, and its
/// standard error.
fn run(command: &mut Command) -> (Option<i32>, String, String) {
    let output = command.output().expect("the command runs");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Copy the sources and the dependencies of `examples/plug_hello` into
/// `directory` and return the copy. The copy leaves out `_build`, which
/// other tests write at the same time, so the copy compiles its own build.
/// The copy of `mix.exs` names the PBeam source of the test run and the
/// private release directory `directory/release`, so that the task writes
/// no file of the repository build that other tests read.
#[expect(
    clippy::unnecessary_debug_formatting,
    reason = "`{:?}` writes a path as a quoted string literal, which `mix.exs` reads as an Elixir string"
)]
fn copied_example(directory: &Path) -> PathBuf {
    let example = directory.join("plug_hello");
    fs::create_dir_all(&example).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plug_hello");
    let status = Command::new("cp")
        .arg("-a")
        .args(
            [
                "config",
                "deps",
                "lib",
                "mix.exs",
                "mix.lock",
                ".formatter.exs",
            ]
            .map(|entry| source.join(entry)),
        )
        .arg(&example)
        .status()
        .expect("cp runs");
    assert!(status.success());
    let mixfile = example.join("mix.exs");
    let config = fs::read_to_string(&mixfile).unwrap();
    let config = config
        .replace("\"../..\"", &format!("{:?}", env!("CARGO_MANIFEST_DIR")))
        .replace(
            "\"../../target/examples/plug_hello\"",
            &format!("{:?}", directory.join("release")),
        );
    fs::write(&mixfile, config).unwrap();
    example
}

/// Return the OTP pin of the PBeam source: the `erlang` version of its
/// `mise.toml`, which the ERTS version check of the task names.
fn pinned_otp() -> String {
    let pins = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("mise.toml")).unwrap();
    let pin = pins
        .lines()
        .find_map(|line| line.strip_prefix("erlang = \""))
        .expect("mise.toml pins erlang");
    pin.strip_suffix('"')
        .expect("the pin ends with a quote")
        .to_owned()
}

#[test]
fn builds_the_release_and_the_executable_of_the_example() {
    // The task builds the release itself: the test gives no `--release`
    // and runs no separate `mix release` step.
    let directory = directory("example");
    let example = copied_example(&directory);
    let release = directory.join("release");
    let output = directory.join("plug_hello.bundle");
    let (status, stdout, stderr) = run(Command::new("mix")
        .arg("pbeam.build")
        .arg("--output")
        .arg(&output)
        .arg("--runtime")
        .arg(env!("CARGO_BIN_EXE_pbeam"))
        .arg("--tool")
        .arg(env!("CARGO_BIN_EXE_pbeam_bundle"))
        .current_dir(&example)
        .env("MIX_ENV", "prod"));
    assert_eq!(status, Some(0), "{stdout}{stderr}");
    // The release task ran before the bundle: it prints this line for the
    // release of the project config.
    assert!(stdout.contains("Release created at"), "{stdout}");
    assert!(
        stdout.contains(&format!("Wrote {}", output.display())),
        "{stdout}"
    );
    // The release that the task built has an ERTS version, and the bundle
    // carries its `start_erl.data` at the path of the release. The task
    // writes the intermediate bundle next to the executable.
    let data = fs::read_to_string(release.join("releases/start_erl.data")).unwrap();
    assert!(data.split_ascii_whitespace().count() >= 2, "{data}");
    assert!(output.is_file());
    let bytes = fs::read(directory.join("plug_hello.pbeamb")).unwrap();
    let bundle = pbeam_runtime::Bundle::parse(&bytes).unwrap();
    assert_eq!(
        bundle.resource("releases/start_erl.data"),
        Some(data.as_bytes())
    );
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn stops_on_a_release_of_another_erts_version() {
    // A release of another ERTS version holds OTP modules that PBeam does
    // not run. The task stops before the bundle and names the ERTS version
    // of the release and the OTP pin of the PBeam source.
    let directory = directory("version");
    let example = copied_example(&directory);
    let release = directory.join("release");
    fs::create_dir_all(release.join("releases")).unwrap();
    fs::write(release.join("releases/start_erl.data"), "14.2.1 0.1.0").unwrap();
    let output = directory.join("plug_hello.bundle");
    let (status, stdout, stderr) = run(Command::new("mix")
        .arg("pbeam.build")
        .arg("--release")
        .arg(&release)
        .arg("--output")
        .arg(&output)
        .arg("--runtime")
        .arg(env!("CARGO_BIN_EXE_pbeam"))
        .arg("--tool")
        .arg(env!("CARGO_BIN_EXE_pbeam_bundle"))
        .current_dir(&example)
        .env("MIX_ENV", "prod"));
    assert_eq!(status, Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("14.2.1"), "{stderr}");
    assert!(stderr.contains(&pinned_otp()), "{stderr}");
    // The task wrote no executable.
    assert!(!output.exists());
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn stops_outside_the_prod_environment_and_on_a_release_without_an_erts_version() {
    // With no `--release`, the task builds the release of the prod
    // environment only. A release whose `releases/start_erl.data` names no
    // ERTS version stops the task before the bundle.
    let directory = directory("checks");
    let example = copied_example(&directory);
    let release = directory.join("release");
    fs::create_dir_all(release.join("releases")).unwrap();
    fs::write(release.join("releases/start_erl.data"), "").unwrap();
    let output = directory.join("plug_hello.bundle");
    let build = |release: Option<&Path>, environment: &str| {
        let mut command = Command::new("mix");
        command.arg("pbeam.build");
        if let Some(release) = release {
            command.arg("--release").arg(release);
        }
        run(command
            .arg("--output")
            .arg(&output)
            .arg("--runtime")
            .arg(env!("CARGO_BIN_EXE_pbeam"))
            .arg("--tool")
            .arg(env!("CARGO_BIN_EXE_pbeam_bundle"))
            .current_dir(&example)
            .env("MIX_ENV", environment))
    };
    let (status, stdout, stderr) = build(None, "dev");
    assert_eq!(status, Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("MIX_ENV=prod"), "{stderr}");
    let (status, stdout, stderr) = build(Some(&release), "prod");
    assert_eq!(status, Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("names no ERTS version"), "{stderr}");
    assert!(!output.exists());
    fs::remove_dir_all(&directory).unwrap();
}
