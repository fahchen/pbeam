//! The errors of the `pbeam_bundle` tool: the usage, the input files, and
//! the output file.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// A BEAM file that passes the NIF manifest gate.
const NO_GAPS: &[u8] = include_bytes!("../../fixtures/nif_gate/ok/prim_tty.beam");

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
    let directory = directory("mix-task");
    let output = directory.join("plug_hello.pbeamb");
    let start = directory.join("start.list");
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plug_hello");
    // The release needs no earlier output of the example runner. The test
    // builds its own release, so it does not read the release that another
    // test rebuilds.
    let release = directory.join("release");
    let mut mix_release = Command::new("mix");
    mix_release
        .args(["release", "--overwrite", "--path"])
        .arg(&release)
        .current_dir(&example)
        .env("MIX_ENV", "prod");
    let (status, mix_output) = run(&mut mix_release);
    assert_eq!(status, Some(0), "{mix_output}");
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
    // The task compiles no entry stub: the launcher boots the release with
    // its boot script.
    assert!(!release.join("pbeam").exists());
    let bytes = fs::read(&output).unwrap();
    let bundle = pbeam_runtime::Bundle::parse(&bytes).unwrap();
    let names: Vec<_> = bundle
        .resources()
        .iter()
        .map(|blob| blob.name().to_owned())
        .collect();
    // The start set is the preloaded modules, which the launcher gives to
    // `pbeam erl` from the ebin directory of erts.
    assert_eq!(
        bundle.resource("meta/start-modules"),
        Some(
            &b"persistent_term counters atomics erts_dirty_process_signal_handler \
               erts_trace_cleaner erts_literal_area_collector erl_tracer erts_internal erlang \
               erl_prim_loader prim_zip prim_net prim_socket socket_registry zlib prim_file \
               prim_inet prim_eval prim_buffer init erl_init erts_code_purger"[..]
        )
    );
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("lib/erts-") && name.ends_with("/ebin/init.beam"))
    );
    assert!(names.iter().any(|name| name == "releases/0.1.0/start.boot"));
    assert_eq!(
        (bundle.entry().module, bundle.entry().function),
        ("elixir", "start_cli")
    );
    // The payloads are the bytes of the release files.
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
    fs::write(ebin.join("init.beam"), NO_GAPS).unwrap();
    fs::write(ebin.join("extra.beam"), NO_GAPS).unwrap();
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
        vec!["bundle", "1", "2", "3", "4", "5", "6", "7"],
        vec!["fixture"],
        vec!["fixture", "only"],
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
    fs::write(&module, NO_GAPS).unwrap();
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

#[test]
fn writes_the_bundle_of_a_fixture_example() {
    // A fixture has no release directory. The tool writes the layout of a
    // release from the start set and the beam list of scripts/example.sh:
    // the example modules in `pbeam/`, the consolidated protocols in
    // `releases/pbeam/consolidated/`, and each application ebin directory
    // with all its files, because the run loads each other module on demand.
    let directory = directory("fixture");
    let example = directory.join("example");
    let consolidated = directory.join("consolidated");
    let ebin = directory.join("otp/lib/app-1/ebin");
    fs::create_dir_all(&example).unwrap();
    fs::create_dir_all(&consolidated).unwrap();
    fs::create_dir_all(&ebin).unwrap();
    fs::write(example.join("Elixir.M.beam"), b"entry").unwrap();
    fs::write(example.join("Elixir.Other.beam"), b"other").unwrap();
    fs::write(example.join("pbeam_boot.beam"), b"boot").unwrap();
    fs::write(consolidated.join("Elixir.Proto.beam"), b"proto").unwrap();
    fs::write(ebin.join("init.beam"), b"init").unwrap();
    fs::write(ebin.join("extra.beam"), b"extra").unwrap();
    let lines = [
        example.join("Elixir.M.beam"),
        example.join("Elixir.Other.beam"),
        consolidated.join("Elixir.Proto.beam"),
        example.join("pbeam_boot.beam"),
        ebin.join("init.beam"),
    ];
    let lines: Vec<_> = lines.iter().map(|path| path.to_str().unwrap()).collect();
    let start = directory.join("start.list");
    fs::write(&start, lines.join("\n")).unwrap();
    let beams = directory.join("beams.list");
    fs::write(&beams, lines.join("\n")).unwrap();
    let output = directory.join("out.pbeamb");
    let (status, stderr) = run(tool()
        .arg("fixture")
        .arg(&output)
        .args(["Elixir.M", "main"])
        .arg(&start)
        .arg(&beams)
        .arg(&consolidated));
    assert_eq!(status, Some(0), "{stderr}");
    let bytes = fs::read(&output).unwrap();
    let bundle = pbeam_runtime::Bundle::parse(&bytes).unwrap();
    assert_eq!(bundle.entry().module, "Elixir.M");
    assert_eq!(bundle.resource("pbeam/Elixir.M.beam").unwrap(), b"entry");
    assert_eq!(
        bundle.resource("pbeam/Elixir.Other.beam").unwrap(),
        b"other"
    );
    assert_eq!(
        bundle
            .resource("releases/pbeam/consolidated/Elixir.Proto.beam")
            .unwrap(),
        b"proto"
    );
    assert_eq!(
        bundle.resource("lib/app-1/ebin/init.beam").unwrap(),
        b"init"
    );
    // The start set does not name `extra.beam`, and the bundle carries it.
    assert_eq!(
        bundle.resource("lib/app-1/ebin/extra.beam").unwrap(),
        b"extra"
    );
    let modules = bundle.resource("meta/start-modules").unwrap();
    assert_eq!(
        String::from_utf8_lossy(modules)
            .split_ascii_whitespace()
            .collect::<Vec<_>>(),
        ["Elixir.Other", "Elixir.Proto", "init"]
    );
    assert_eq!(
        bundle.resource("releases/start_erl.data").unwrap(),
        format!("{} pbeam", env!("PBEAM_OTP_RELEASE")).as_bytes()
    );
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn reports_a_fixture_beam_that_the_layout_does_not_hold() {
    let directory = directory("fixture-error");
    let example = directory.join("example");
    let consolidated = directory.join("consolidated");
    let stray = directory.join("stray");
    fs::create_dir_all(&example).unwrap();
    fs::create_dir_all(&consolidated).unwrap();
    fs::create_dir_all(&stray).unwrap();
    fs::write(example.join("Elixir.M.beam"), b"entry").unwrap();
    fs::write(consolidated.join("Elixir.Proto.beam"), b"proto").unwrap();
    fs::write(stray.join("stray.beam"), b"stray").unwrap();
    let entry = example.join("Elixir.M.beam");
    let stray = stray.join("stray.beam");
    let start = start_list(
        &directory,
        &[entry.to_str().unwrap(), stray.to_str().unwrap()],
    );
    let beams = directory.join("beams.list");
    fs::write(
        &beams,
        [entry.to_str().unwrap(), stray.to_str().unwrap()].join("\n"),
    )
    .unwrap();
    let output = directory.join("out.pbeamb");
    let (status, stderr) = run(tool()
        .arg("fixture")
        .arg(&output)
        .args(["Elixir.M", "main"])
        .arg(&start)
        .arg(&beams)
        .arg(&consolidated));
    assert_eq!(status, Some(1));
    assert!(
        stderr.contains("is not in an OTP lib directory"),
        "{stderr}"
    );
    // The start list names the entry module.
    let start = start_list(&directory, &[stray.to_str().unwrap()]);
    let (status, stderr) = run(tool()
        .arg("fixture")
        .arg(&output)
        .args(["Elixir.M", "main"])
        .arg(&start)
        .arg(&beams)
        .arg(&consolidated));
    assert_eq!(status, Some(1));
    assert!(stderr.contains("has no Elixir.M.beam"), "{stderr}");
    // An input file or directory that the tool cannot read.
    let missing = directory.join("missing");
    let start = start_list(&directory, &[entry.to_str().unwrap()]);
    let (status, stderr) = run(tool()
        .arg("fixture")
        .arg(&output)
        .args(["Elixir.M", "main"])
        .arg(&missing)
        .arg(&beams)
        .arg(&consolidated));
    assert_eq!(status, Some(1));
    assert!(stderr.contains("cannot read"), "{stderr}");
    let (status, stderr) = run(tool()
        .arg("fixture")
        .arg(&output)
        .args(["Elixir.M", "main"])
        .arg(&start)
        .arg(&missing)
        .arg(&consolidated));
    assert_eq!(status, Some(1));
    assert!(stderr.contains("cannot read"), "{stderr}");
    let (status, stderr) = run(tool()
        .arg("fixture")
        .arg(&output)
        .args(["Elixir.M", "main"])
        .arg(&start)
        .arg(&beams)
        .arg(&missing));
    assert_eq!(status, Some(1));
    assert!(
        stderr.contains("cannot read the consolidated directory"),
        "{stderr}"
    );
    let missing_beam = example.join("Elixir.Missing.beam");
    let start = start_list(
        &directory,
        &[entry.to_str().unwrap(), missing_beam.to_str().unwrap()],
    );
    let (status, stderr) = run(tool()
        .arg("fixture")
        .arg(&output)
        .args(["Elixir.M", "main"])
        .arg(&start)
        .arg(&beams)
        .arg(&consolidated));
    assert_eq!(status, Some(1));
    assert!(stderr.contains("cannot read"), "{stderr}");
    fs::remove_dir_all(&directory).unwrap();
}

/// Return the path of the gate fixture directory `name` in `fixtures/nif_gate`.
fn nif_gate(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/nif_gate")
        .join(name)
}

#[test]
fn runs_the_nif_manifest_gate_on_each_beam_file_of_a_directory() {
    // The directory also holds the source, which the gate does not read.
    let (status, stderr) = run(tool().arg("check-nifs").arg(nif_gate("ok")));
    assert_eq!((status, stderr.as_str()), (Some(0), ""));

    // The items of each fixture, in the order of the paths.
    let (status, stderr) = run(tool().arg("check-nifs").arg(nif_gate("")));
    assert_eq!(status, Some(1));
    assert_eq!(
        stderr,
        "pbeam_bundle: the NIF manifest gate failed:\n\
         nif_gate_dynamic: the library name of erlang:load_nif/2 is not a literal string\n\
         nif_gate_dynamic: the library name of erlang:load_nif/2 is not a literal string\n\
         prim_tty:isprint/1: NIF function has no PBeam binding\n\
         prim_tty:wcwidth/1: NIF function has no PBeam binding\n\
         nif_gate_library: NIF library \"priv/nif_gate_library\" has no PBeam bindings\n\
         prim_tty:isprint/1: NIF function has no PBeam binding\n"
    );
}

#[test]
fn reports_the_usage_and_the_input_of_the_nif_manifest_gate() {
    let (status, stderr) = run(tool().arg("check-nifs"));
    assert_eq!(status, Some(1));
    assert!(stderr.starts_with("usage:"), "{stderr}");

    let directory = directory("check-nifs");
    let (status, stderr) = run(tool().arg("check-nifs").arg(directory.join("missing")));
    assert_eq!(status, Some(1));
    assert!(stderr.contains("cannot read"), "{stderr}");

    fs::write(directory.join("bad.beam"), b"not beam").unwrap();
    let (status, stderr) = run(tool().arg("check-nifs").arg(&directory));
    assert_eq!(status, Some(1));
    assert!(
        stderr.starts_with("pbeam_bundle: cannot load bad.beam:"),
        "{stderr}"
    );
    fs::remove_dir_all(&directory).unwrap();
}

/// Run `pbeam_bundle bundle` on `release` with an empty start list and the
/// arguments `extra`, and return the exit code, the stdout, the stderr, and
/// whether the output exists.
fn bundle_release(release: &Path, extra: &[&Path]) -> (Option<i32>, String, String, bool) {
    let directory = release.parent().unwrap();
    let start = start_list(directory, &[]);
    let output = directory.join("out");
    let _ = fs::remove_file(&output);
    let result = tool()
        .arg("bundle")
        .arg(release)
        .arg(&output)
        .args(["m", "f"])
        .arg(&start)
        .args(extra)
        .output()
        .expect("the tool runs");
    (
        result.status.code(),
        String::from_utf8_lossy(&result.stdout).into_owned(),
        String::from_utf8_lossy(&result.stderr).into_owned(),
        output.exists(),
    )
}

#[test]
fn runs_the_nif_manifest_gate_with_the_allowlist_before_the_bundle_build() {
    let directory = directory("nif-allowlist");
    let release = directory.join("release");
    let ebin = release.join("lib/app/ebin");
    fs::create_dir_all(&ebin).unwrap();
    fs::copy(
        nif_gate("missing_binding/prim_tty.beam"),
        ebin.join("prim_tty.beam"),
    )
    .unwrap();
    let allowlist = directory.join("nif-allowlist.txt");

    // Each accepted item is printed with the reason of its group.
    fs::write(
        &allowlist,
        "reason: not reached\nprim_tty:isprint/1\n\nreason:  not reached either \nprim_tty:wcwidth/1\n",
    )
    .unwrap();
    assert_eq!(
        bundle_release(&release, &[&allowlist]),
        (
            Some(0),
            "pbeam_bundle: the NIF manifest gate accepts prim_tty:isprint/1: not reached\n\
             pbeam_bundle: the NIF manifest gate accepts prim_tty:wcwidth/1: not reached either\n"
                .to_owned(),
            String::new(),
            true
        )
    );

    // Without an allowlist, each item fails the build before the output.
    let (status, _, stderr, written) = bundle_release(&release, &[]);
    assert_eq!((status, written), (Some(1), false));
    assert_eq!(
        stderr,
        "pbeam_bundle: the NIF manifest gate failed:\n\
         prim_tty:isprint/1: NIF function has no PBeam binding\n\
         prim_tty:wcwidth/1: NIF function has no PBeam binding\n"
    );

    // A referenced library without bindings cannot be accepted, and an item
    // of the list that the bundle does not have fails.
    fs::copy(
        nif_gate("missing_library/nif_gate_library.beam"),
        ebin.join("nif_gate_library.beam"),
    )
    .unwrap();
    fs::write(
        &allowlist,
        "reason: not reached\nprim_tty:isprint/1\nprim_tty:wcwidth/1\nnif_gate_library:load/0\n",
    )
    .unwrap();
    let (status, stdout, stderr, written) = bundle_release(&release, &[&allowlist]);
    assert_eq!((status, written), (Some(1), false));
    assert_eq!(stdout.lines().count(), 2, "{stdout}");
    assert_eq!(
        stderr,
        "pbeam_bundle: the NIF manifest gate failed:\n\
         nif_gate_library: NIF library \"priv/nif_gate_library\" has no PBeam bindings\n\
         nif_gate_library:load/0: the NIF allowlist accepts an item that is not a gap\n"
    );
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn reports_an_allowlist_item_without_a_reason_and_an_allowlist_that_does_not_read() {
    let directory = directory("nif-allowlist-errors");
    let release = directory.join("release");
    fs::create_dir_all(&release).unwrap();
    let allowlist = directory.join("nif-allowlist.txt");
    fs::write(&allowlist, "\nreason:\nprim_tty:isprint/1\n").unwrap();
    let (status, _, stderr, written) = bundle_release(&release, &[&allowlist]);
    assert_eq!((status, written), (Some(1), false));
    assert_eq!(
        stderr,
        format!(
            "pbeam_bundle: {}:3: the item prim_tty:isprint/1 has no reason\n",
            allowlist.display()
        )
    );

    let (status, _, stderr, written) = bundle_release(&release, &[&directory.join("missing")]);
    assert_eq!((status, written), (Some(1), false));
    assert!(stderr.contains("cannot read"), "{stderr}");
    fs::remove_dir_all(&directory).unwrap();
}
