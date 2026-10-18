//! Unit tests of the embedded mode: the trailer, the extraction, and the
//! files of the extracted release.

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

#[test]
fn finds_the_bundle_that_the_trailer_names() {
    let mut bytes = b"binary".to_vec();
    bytes.extend_from_slice(b"bundle");
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&6_u64.to_le_bytes());
    assert_eq!(trailer_bundle(&bytes), Some(&b"bundle"[..]));
}

#[test]
fn a_binary_without_a_bundle_has_none() {
    // Shorter than the trailer, a trailer of another magic, and a length
    // beyond the start of the file.
    assert!(trailer_bundle(b"short").is_none());
    assert!(trailer_bundle(b"1234567890123456").is_none());
    let mut bytes = b"bundle".to_vec();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&100_u64.to_le_bytes());
    assert!(trailer_bundle(&bytes).is_none());
}

#[test]
fn a_run_of_a_binary_without_a_trailer_has_no_status() {
    // The unit test binary carries no bundle.
    assert!(run().is_none());
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
fn a_temporary_directory_skips_a_name_that_exists() {
    let base = directory("temp");
    let taken = base.join(format!("pbeam-embedded-{}-0", std::process::id()));
    fs::create_dir(&taken).unwrap();
    let created = temp_directory(&base).unwrap();
    assert_ne!(created, taken);
    assert!(created.is_dir());
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
fn a_bundle_without_start_modules_stops() {
    let directory = directory("no-modules");
    let bytes = bundle(&[]);
    let bundle = Bundle::parse(&bytes).unwrap();
    let error = start_in(Path::new("/bin/false"), &bundle, &directory).unwrap_err();
    assert!(error.contains("meta/start-modules"), "{error}");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_resource_that_leaves_the_directory_stops() {
    let directory = directory("bad-name");
    let bytes = bundle(&[("../x", b"x")]);
    let bundle = Bundle::parse(&bytes).unwrap();
    let error = start_in(Path::new("/bin/false"), &bundle, &directory).unwrap_err();
    assert!(error.contains("not a relative path"), "{error}");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_resource_below_a_file_stops() {
    let directory = directory("under-file");
    fs::write(directory.join("a"), b"x").unwrap();
    let bytes = bundle(&[("a/b", b"x")]);
    let bundle = Bundle::parse(&bytes).unwrap();
    let error = start_in(Path::new("/bin/false"), &bundle, &directory).unwrap_err();
    assert!(error.contains("cannot make"), "{error}");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_resource_on_a_directory_stops() {
    let directory = directory("on-directory");
    fs::create_dir(directory.join("x")).unwrap();
    let bytes = bundle(&[("x", b"x")]);
    let bundle = Bundle::parse(&bytes).unwrap();
    let error = start_in(Path::new("/bin/false"), &bundle, &directory).unwrap_err();
    assert!(error.contains("cannot write"), "{error}");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_start_module_that_the_release_lacks_stops() {
    let directory = directory("missing-module");
    let bytes = bundle(&[("meta/start-modules", b"missing")]);
    let bundle = Bundle::parse(&bytes).unwrap();
    let error = start_in(Path::new("/bin/false"), &bundle, &directory).unwrap_err();
    assert!(error.contains("has no module missing"), "{error}");
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_run_that_the_host_cannot_start_stops() {
    let directory = directory("no-executable");
    let bytes = bundle(&[
        ("meta/start-modules", b"init"),
        ("lib/kernel-1/ebin/init.beam", b"beam"),
        ("releases/start_erl.data", b"17.1 0.1.0"),
    ]);
    let bundle = Bundle::parse(&bytes).unwrap();
    let error = start_in(Path::new("/nonexistent/pbeam"), &bundle, &directory).unwrap_err();
    assert!(error.contains("cannot start the embedded run"), "{error}");
    fs::remove_dir_all(&directory).unwrap();
}
