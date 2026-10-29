//! Unit tests of the embedded mode: the trailer, the extraction, the boot
//! command, and the files of the extracted release.

use super::*;
use crate::BundleWriter;

/// Return a new directory for one test.
fn directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("pbeam-embedded-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// Return the bundle of `resources` with the entry `m:f/0`.
fn bundle(resources: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = BundleWriter::new(env!("PBEAM_OTP_RELEASE"), "test", "m", "f", 0).unwrap();
    for (name, bytes) in resources {
        writer.add_resource(name, bytes).unwrap();
    }
    writer.encode()
}

/// Return the bundle with an entry that the extraction and the boot reach.
fn startable() -> Vec<u8> {
    bundle(&[
        ("meta/start-modules", b"init"),
        ("lib/kernel-1/ebin/init.beam", b"beam"),
        ("releases/start_erl.data", b"17.1 0.1.0"),
    ])
}

/// Return the image of a binary with `bytes` as its bundle.
fn image(bytes: &[u8]) -> Vec<u8> {
    let mut image = b"binary".to_vec();
    image.extend_from_slice(bytes);
    image.extend_from_slice(MAGIC);
    image.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    image
}

#[test]
fn finds_the_bundle_that_the_trailer_names() {
    let mut bytes = b"binary".to_vec();
    bytes.extend_from_slice(b"bundle");
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&6_u64.to_le_bytes());
    assert_eq!(trailer(&bytes), Trailer::Bundle(b"bundle"));
}

#[test]
fn a_binary_without_a_trailer_has_none() {
    // Shorter than the trailer, and a trailer of another magic.
    assert_eq!(trailer(b"short"), Trailer::Absent);
    assert_eq!(trailer(b"1234567890123456"), Trailer::Absent);
}

#[test]
fn a_trailer_whose_bundle_does_not_fit_is_invalid() {
    let mut bytes = b"bundle".to_vec();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&100_u64.to_le_bytes());
    assert_eq!(trailer(&bytes), Trailer::Invalid);
}

#[test]
fn a_run_of_a_binary_without_a_trailer_has_no_status() {
    // The unit test binary carries no bundle.
    assert!(run([]).is_none());
}

#[test]
fn the_word_of_the_boot_re_exec_marks_only_itself() {
    use std::ffi::OsStr;

    // Only the marker word of the re-exec keeps a run from the bundle; a
    // user word such as `erl` does not, so it reaches the release.
    assert!(is_boot_reexec(OsStr::new("--embedded-boot")));
    assert!(!is_boot_reexec(OsStr::new("erl")));
    assert!(!is_boot_reexec(OsStr::new("")));
}

#[test]
fn a_run_bundle_stops_at_an_invalid_trailer_or_bundle() {
    let directory = directory("run-bundle");
    let executable = Path::new("/nonexistent/pbeam");
    let mut invalid = b"x".to_vec();
    invalid.extend_from_slice(MAGIC);
    invalid.extend_from_slice(&100_u64.to_le_bytes());
    assert_eq!(
        run_bundle(executable, &invalid, &directory, Vec::new()),
        Some(ExitCode::FAILURE)
    );
    let mut garbage = b"garbage!".to_vec();
    garbage.extend_from_slice(MAGIC);
    garbage.extend_from_slice(&8_u64.to_le_bytes());
    assert_eq!(
        run_bundle(executable, &garbage, &directory, Vec::new()),
        Some(ExitCode::FAILURE)
    );
    assert_eq!(
        run_bundle(executable, b"plain", &directory, Vec::new()),
        None
    );
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_run_bundle_reports_a_base_that_fails() {
    let directory = directory("run-base");
    let file = directory.join("file");
    fs::write(&file, b"").unwrap();
    assert_eq!(
        run_bundle(
            Path::new("/nonexistent/pbeam"),
            &image(&startable()),
            &file,
            Vec::new()
        ),
        Some(ExitCode::FAILURE)
    );
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_run_bundle_that_cannot_start_removes_its_directory() {
    let base = directory("run-remove");
    assert_eq!(
        run_bundle(
            Path::new("/nonexistent/pbeam"),
            &image(&startable()),
            &base,
            Vec::new()
        ),
        Some(ExitCode::FAILURE)
    );
    assert!(fs::read_dir(&base).unwrap().next().is_none());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn removes_the_extracted_directory_of_a_run() {
    let directory = directory("remove");
    remove_extraction(Some(directory.clone()));
    assert!(!directory.exists());
    remove_extraction(None);
}

#[test]
fn a_resource_name_stays_below_the_directory() {
    let directory = Path::new("/tmp/root");
    assert_eq!(
        bundle_path(directory, "lib/app/ebin/x.beam").unwrap(),
        PathBuf::from("/tmp/root/lib/app/ebin/x.beam")
    );
    assert!(bundle_path(directory, "/etc/passwd").is_err());
    assert!(bundle_path(directory, "../x").is_err());
}

#[test]
fn a_resource_below_a_file_stops() {
    let directory = directory("under-file");
    fs::write(directory.join("a"), b"x").unwrap();
    let bytes = bundle(&[("a/b", b"x")]);
    let bundle = Bundle::parse(&bytes).unwrap();
    let error = extract(&directory, &bundle).unwrap_err();
    assert!(error.contains("cannot make"), "{error}");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_resource_on_a_directory_stops() {
    let directory = directory("on-directory");
    fs::create_dir(directory.join("x")).unwrap();
    let bytes = bundle(&[("x", b"x")]);
    let bundle = Bundle::parse(&bytes).unwrap();
    let error = extract(&directory, &bundle).unwrap_err();
    assert!(error.contains("cannot write"), "{error}");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_bundle_without_start_modules_stops() {
    let bytes = bundle(&[]);
    let bundle = Bundle::parse(&bytes).unwrap();
    let error = start_modules(&bundle).unwrap_err();
    assert!(error.contains("meta/start-modules"), "{error}");
}

#[test]
fn a_start_modules_resource_names_its_modules() {
    let bytes = bundle(&[("meta/start-modules", b"init kernel")]);
    let bundle = Bundle::parse(&bytes).unwrap();
    assert_eq!(start_modules(&bundle).unwrap(), vec!["init", "kernel"]);
}

#[test]
fn a_start_module_resolves_from_lib_or_from_erts() {
    let directory = directory("resolve");
    let ebin = directory.join("lib/kernel-1/ebin");
    fs::create_dir_all(&ebin).unwrap();
    fs::write(ebin.join("init.beam"), b"beam").unwrap();
    assert_eq!(
        resolve_module(&directory, "init").unwrap(),
        ebin.join("init.beam")
    );
    let erts = directory.join("erts-17/ebin");
    fs::create_dir_all(&erts).unwrap();
    fs::write(erts.join("erlang.beam"), b"beam").unwrap();
    assert_eq!(
        resolve_module(&directory, "erlang").unwrap(),
        erts.join("erlang.beam")
    );
    // The modules of a fixture bundle are in `pbeam/`, and its consolidated
    // protocols are in `releases/*/consolidated/`. The consolidated
    // protocol comes before the unconsolidated protocol of the lib
    // directory, and a module of `pbeam/` comes before both.
    let pbeam = directory.join("pbeam");
    fs::create_dir_all(&pbeam).unwrap();
    fs::write(pbeam.join("example.beam"), b"beam").unwrap();
    assert_eq!(
        resolve_module(&directory, "example").unwrap(),
        pbeam.join("example.beam")
    );
    let consolidated = directory.join("releases/pbeam/consolidated");
    fs::create_dir_all(&consolidated).unwrap();
    fs::write(consolidated.join("proto.beam"), b"beam").unwrap();
    fs::write(ebin.join("proto.beam"), b"unconsolidated").unwrap();
    assert_eq!(
        resolve_module(&directory, "proto").unwrap(),
        consolidated.join("proto.beam")
    );
    assert!(resolve_module(&directory, "missing").is_err());
    assert!(resolve_module(&directory.join("missing"), "init").is_err());
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn the_release_version_is_the_second_field_of_start_erl_data() {
    let directory = directory("version");
    let releases = directory.join("releases");
    fs::create_dir_all(&releases).unwrap();
    fs::write(releases.join("start_erl.data"), "17.1 0.1.0").unwrap();
    assert_eq!(release_version(&directory).unwrap(), "0.1.0");
    fs::write(releases.join("start_erl.data"), "17.1").unwrap();
    assert!(release_version(&directory).is_err());
    fs::remove_file(releases.join("start_erl.data")).unwrap();
    assert!(release_version(&directory).is_err());
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn the_code_paths_hold_the_entry_the_apps_and_the_protocols() {
    let directory = directory("paths");
    fs::create_dir_all(directory.join("lib/kernel-1/ebin")).unwrap();
    assert_eq!(
        code_paths(&directory, "0.1.0").unwrap(),
        vec![
            directory.join("pbeam"),
            directory.join("lib/kernel-1/ebin"),
            directory.join("releases/0.1.0/consolidated")
        ]
    );
    assert!(code_paths(&directory.join("missing"), "0.1.0").is_err());
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_boot_with_a_start_module_that_the_release_lacks_stops() {
    let directory = directory("missing-module");
    let entry = crate::Entry {
        module: "m",
        function: "f",
        arity: 0,
    };
    let error = boot(
        Path::new("/nonexistent/pbeam"),
        entry,
        &["missing".to_owned()],
        &directory,
    )
    .unwrap_err();
    assert!(error.contains("has no module missing"), "{error}");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_boot_without_a_release_version_stops() {
    let directory = directory("no-version");
    fs::create_dir_all(directory.join("lib")).unwrap();
    let entry = crate::Entry {
        module: "m",
        function: "f",
        arity: 0,
    };
    let error = boot(Path::new("/nonexistent/pbeam"), entry, &[], &directory).unwrap_err();
    assert!(error.contains("start_erl.data"), "{error}");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_boot_that_the_host_cannot_start_returns_the_error() {
    let directory = directory("no-executable");
    let releases = directory.join("releases");
    fs::create_dir_all(directory.join("lib")).unwrap();
    fs::create_dir_all(&releases).unwrap();
    fs::write(releases.join("start_erl.data"), "17.1 0.1.0").unwrap();
    let entry = crate::Entry {
        module: "m",
        function: "f",
        arity: 0,
    };
    let error = boot(Path::new("/nonexistent/pbeam"), entry, &[], &directory).unwrap();
    assert!(error.to_string().contains("No such file"), "{error}");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_start_that_fails_reports_it_and_removes_the_directory() {
    let directory = directory("start-error");
    let bytes = bundle(&[]);
    let bundle = Bundle::parse(&bytes).unwrap();
    assert_eq!(
        start(Path::new("/nonexistent/pbeam"), &bundle, &directory, &[]),
        ExitCode::FAILURE
    );
    assert!(!directory.exists());
}

#[test]
fn a_temporary_directory_skips_a_name_that_exists() {
    let base = directory("temp");
    let taken = base.join(format!("pbeam-embedded-{}-0", std::process::id()));
    fs::create_dir(&taken).unwrap();
    let created = temp_directory(&base).unwrap();
    assert_ne!(created, taken);
    assert!(created.is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&created).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    for attempt in 0..100 {
        let path = base.join(format!("pbeam-embedded-{}-{attempt}", std::process::id()));
        let _ = fs::create_dir(&path);
    }
    assert!(temp_directory(&base).is_err());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_temporary_directory_reports_a_base_that_fails() {
    let base = directory("temp-error");
    let file = base.join("file");
    fs::write(&file, b"").unwrap();
    assert!(temp_directory(&file).is_err());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_release_with_a_boot_script_boots_with_pbeam_erl() {
    let directory = directory("release-boot");
    let release = directory.join("releases/0.1.0");
    fs::create_dir_all(&release).unwrap();
    assert_eq!(release_boot(&directory), None);
    fs::write(directory.join("releases/start_erl.data"), "17.1 0.1.0").unwrap();
    assert_eq!(release_boot(&directory), None);
    fs::write(release.join("start.boot"), b"boot").unwrap();
    assert_eq!(
        release_boot(&directory),
        Some(("17.1".to_owned(), "0.1.0".to_owned()))
    );
    let entry = crate::Entry {
        module: "elixir",
        function: "start_cli",
        arity: 0,
    };
    let error = boot_release(
        Path::new("/nonexistent/pbeam"),
        entry,
        &directory,
        "17.1",
        "0.1.0",
        Some(std::ffi::OsStr::new(" +S  4\t")),
        &[std::ffi::OsString::from("a")],
    );
    assert!(error.to_string().contains("No such file"), "{error}");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn splits_the_erl_options_at_space_tab_and_newline_only() {
    // The default `IFS` of sh is space, tab, and newline, so `\r` and
    // `\x0C` stay in a word (`releases/<vsn>/elixir:229`).
    let words = option_words(std::ffi::OsStr::new(" +S\t4\n-a\rb\x0Cc  "));
    assert_eq!(words, ["+S", "4", "-a\rb\x0Cc"]);
}

#[test]
fn a_start_of_a_release_that_fails_reports_it_and_removes_the_directory() {
    let directory = directory("release-start-error");
    let bytes = bundle(&[
        ("releases/start_erl.data", b"17.1 0.1.0"),
        ("releases/0.1.0/start.boot", b"boot"),
    ]);
    let bundle = Bundle::parse(&bytes).unwrap();
    assert_eq!(
        start(Path::new("/nonexistent/pbeam"), &bundle, &directory, &[]),
        ExitCode::FAILURE
    );
    assert!(!directory.exists());
}

#[test]
fn the_extraction_makes_each_file_of_the_erts_bin_directory_executable() {
    use std::os::unix::fs::PermissionsExt;
    let directory = directory("erts-mode");
    let bytes = bundle(&[
        ("erts-17.1/bin/inet_gethost", b"program"),
        ("lib/erts-17.1/ebin/init.beam", b"beam"),
    ]);
    extract(&directory, &Bundle::parse(&bytes).unwrap()).unwrap();
    let mode = |name: &str| {
        fs::metadata(directory.join(name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode("erts-17.1/bin/inet_gethost"), 0o755);
    assert_eq!(mode("lib/erts-17.1/ebin/init.beam") & 0o111, 0);
    assert!(!is_erts_program("erts-17.1"));
    assert!(!is_erts_program("lib/erts-17.1/bin/x"));
    fs::remove_dir_all(&directory).unwrap();
}
