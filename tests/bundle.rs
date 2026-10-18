//! Unit tests of [`Bundle`](super::Bundle) and
//! [`BundleWriter`](super::BundleWriter). They drive the real encoder and the
//! real parser with in-memory bytes; a malformed case builds the bytes by
//! hand, so it can pass a value that the writer rejects.

use super::{Bundle, BundleWriter, Cursor, Entry, MAX_MODULES, MAX_OTP_RELEASE_BYTES, OTP_RELEASE};
use crate::BeamProgram;

/// The compiler identity of the test bundles.
const COMPILER: &str = "elixir 1.20.4-otp-29";

/// A real compiled module, so the round-trip test shows that a module payload
/// feeds the loader.
const CONSTANT_BEAM: &[u8] = include_bytes!("../fixtures/beam/constant.beam");

/// Write a length-prefixed text, as the writer does.
fn push_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u16).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

/// Write a table, as the writer does.
fn push_table(out: &mut Vec<u8>, entries: &[(&str, &[u8])]) {
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (name, bytes) in entries {
        push_text(out, name);
        out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(bytes);
    }
}

/// Write the magic, the format version, the OTP release, and the compiler
/// identity.
fn push_header(out: &mut Vec<u8>, otp_release: &str, compiler: &str) {
    out.extend_from_slice(b"PBEAMBND");
    out.extend_from_slice(&1u16.to_le_bytes());
    push_text(out, otp_release);
    push_text(out, compiler);
}

/// Write an entry MFA.
fn push_entry(out: &mut Vec<u8>, module: &str, function: &str, arity: u32) {
    push_text(out, module);
    push_text(out, function);
    out.extend_from_slice(&arity.to_le_bytes());
}

/// Return a valid bundle with one module and no resource.
fn valid() -> Vec<u8> {
    let mut out = Vec::new();
    push_header(&mut out, OTP_RELEASE, COMPILER);
    push_entry(&mut out, "constant", "main", 0);
    push_table(&mut out, &[("constant", b"x")]);
    push_table(&mut out, &[]);
    out
}

/// Return the error message of a parse of `bytes`.
fn parse_error(bytes: &[u8]) -> String {
    Bundle::parse(bytes)
        .expect_err("the bundle must not parse")
        .to_string()
}

#[test]
fn round_trips_modules_and_a_resource() {
    let mut writer = BundleWriter::new(OTP_RELEASE, COMPILER, "constant", "main", 0).unwrap();
    writer.add_module("constant", CONSTANT_BEAM).unwrap();
    writer.add_module("other", b"\x00\x01\x02").unwrap();
    writer
        .add_resource("Elixir.Constant.app", b"{application, constant}.")
        .unwrap();
    assert_eq!(writer.bundle().entry().function, "main");
    let bytes = writer.encode();

    let bundle = Bundle::parse(&bytes).unwrap();
    assert_eq!(bundle.otp_release(), OTP_RELEASE);
    assert_eq!(bundle.compiler(), COMPILER);
    assert_eq!(
        bundle.entry(),
        Entry {
            module: "constant",
            function: "main",
            arity: 0,
        }
    );
    assert_eq!(
        bundle
            .modules()
            .iter()
            .map(|blob| (blob.name(), blob.bytes()))
            .collect::<Vec<_>>(),
        vec![
            ("constant", CONSTANT_BEAM),
            ("other", b"\x00\x01\x02".as_slice()),
        ]
    );
    assert_eq!(bundle.module("constant"), Some(CONSTANT_BEAM));
    assert_eq!(bundle.module("missing"), None);
    assert_eq!(bundle.resources().len(), 1);
    assert_eq!(bundle.resources()[0].name(), "Elixir.Constant.app");
    assert_eq!(
        bundle.resource("Elixir.Constant.app"),
        Some(b"{application, constant}.".as_slice())
    );
    assert_eq!(bundle.resource("missing"), None);
    // The format is canonical: a parsed bundle re-encodes to its own bytes.
    assert_eq!(bundle.encode(), bytes);

    // The module payload feeds the runtime loader without a copy.
    let program = BeamProgram::load(&[bundle.module("constant").unwrap()]).unwrap();
    assert!(program.atom("constant").is_some());
}

#[test]
fn accepts_an_empty_module_table_and_an_empty_resource_table() {
    let mut out = Vec::new();
    push_header(&mut out, OTP_RELEASE, COMPILER);
    push_entry(&mut out, "m", "f", 0);
    push_table(&mut out, &[]);
    push_table(&mut out, &[]);
    let bundle = Bundle::parse(&out).unwrap();
    assert!(bundle.modules().is_empty());
    assert!(bundle.resources().is_empty());
}

#[test]
fn rejects_a_bad_magic() {
    let mut bytes = valid();
    bytes[0] = b'X';
    assert_eq!(parse_error(&bytes), "bundle magic is not PBEAMBND");
}

#[test]
fn rejects_an_unsupported_format_version() {
    let mut bytes = valid();
    bytes[8..10].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(
        parse_error(&bytes),
        "unsupported bundle format version 2, expected 1"
    );
}

#[test]
fn rejects_an_incompatible_otp_release() {
    let mut out = Vec::new();
    push_header(&mut out, "28", COMPILER);
    push_entry(&mut out, "m", "f", 0);
    push_table(&mut out, &[]);
    push_table(&mut out, &[]);
    assert_eq!(
        parse_error(&out),
        format!(
            "bundle OTP release 28 is incompatible with this runtime, which supports {OTP_RELEASE}"
        )
    );
}

#[test]
fn rejects_an_otp_release_over_the_limit() {
    let mut out = Vec::new();
    out.extend_from_slice(b"PBEAMBND");
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(MAX_OTP_RELEASE_BYTES as u16 + 1).to_le_bytes());
    out.extend_from_slice(&[b'x'; MAX_OTP_RELEASE_BYTES + 1]);
    assert_eq!(
        parse_error(&out),
        format!(
            "bundle OTP release is {} bytes, over the limit {MAX_OTP_RELEASE_BYTES}",
            MAX_OTP_RELEASE_BYTES + 1
        )
    );
}

#[test]
fn rejects_a_truncated_format_version() {
    assert!(parse_error(b"PBEAMBND\x01").starts_with("bundle is truncated"));
}

#[test]
fn rejects_a_truncated_entry_function_length() {
    let mut out = Vec::new();
    push_header(&mut out, OTP_RELEASE, COMPILER);
    push_text(&mut out, "m");
    out.push(0);
    assert!(parse_error(&out).starts_with("bundle is truncated"));
}

#[test]
fn rejects_a_module_count_over_the_limit() {
    let mut out = Vec::new();
    push_header(&mut out, OTP_RELEASE, COMPILER);
    push_entry(&mut out, "m", "f", 0);
    out.extend_from_slice(&(MAX_MODULES + 1).to_le_bytes());
    assert_eq!(
        parse_error(&out),
        format!(
            "bundle module count {} is over the limit {MAX_MODULES}",
            MAX_MODULES + 1
        )
    );
}

#[test]
fn rejects_a_duplicate_module_name() {
    let mut out = Vec::new();
    push_header(&mut out, OTP_RELEASE, COMPILER);
    push_entry(&mut out, "m", "f", 0);
    push_table(&mut out, &[("m", b"a"), ("m", b"b")]);
    push_table(&mut out, &[]);
    assert_eq!(parse_error(&out), "duplicate bundle module name m");
}

#[test]
fn rejects_a_duplicate_resource_name() {
    let mut out = Vec::new();
    push_header(&mut out, OTP_RELEASE, COMPILER);
    push_entry(&mut out, "m", "f", 0);
    push_table(&mut out, &[]);
    push_table(&mut out, &[("r", b"a"), ("r", b"b")]);
    assert_eq!(parse_error(&out), "duplicate bundle resource name r");
}

#[test]
fn rejects_a_module_name_over_the_byte_limit() {
    let mut out = Vec::new();
    push_header(&mut out, OTP_RELEASE, COMPILER);
    push_entry(&mut out, "m", "f", 0);
    out.extend_from_slice(&1u32.to_le_bytes());
    push_text(&mut out, &"a".repeat(1021));
    assert_eq!(
        parse_error(&out),
        "bundle module 0 name is 1021 bytes, over the limit 1020"
    );
}

#[test]
fn rejects_a_module_name_over_the_character_limit() {
    let mut out = Vec::new();
    push_header(&mut out, OTP_RELEASE, COMPILER);
    push_entry(&mut out, "m", "f", 0);
    out.extend_from_slice(&1u32.to_le_bytes());
    push_text(&mut out, &"é".repeat(256));
    assert_eq!(
        parse_error(&out),
        "bundle module 0 name is 256 characters, over the limit 255"
    );
}

#[test]
fn accepts_an_empty_module_name() {
    let mut out = Vec::new();
    push_header(&mut out, OTP_RELEASE, COMPILER);
    push_entry(&mut out, "", "f", 0);
    push_table(&mut out, &[("", b"x")]);
    push_table(&mut out, &[]);
    let bundle = Bundle::parse(&out).unwrap();
    assert_eq!(bundle.module(""), Some(b"x".as_slice()));
}

#[test]
fn rejects_an_empty_resource_name() {
    let mut out = Vec::new();
    push_header(&mut out, OTP_RELEASE, COMPILER);
    push_entry(&mut out, "m", "f", 0);
    push_table(&mut out, &[]);
    out.extend_from_slice(&1u32.to_le_bytes());
    push_text(&mut out, "");
    assert_eq!(parse_error(&out), "bundle resource 0 name is empty");
}

#[test]
fn rejects_a_non_utf8_module_name() {
    let mut out = Vec::new();
    push_header(&mut out, OTP_RELEASE, COMPILER);
    push_entry(&mut out, "m", "f", 0);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.push(0xff);
    assert_eq!(parse_error(&out), "bundle module 0 name is not UTF-8");
}

#[test]
fn rejects_a_truncated_payload() {
    let mut out = Vec::new();
    push_header(&mut out, OTP_RELEASE, COMPILER);
    push_entry(&mut out, "m", "f", 0);
    out.extend_from_slice(&1u32.to_le_bytes());
    push_text(&mut out, "m");
    out.extend_from_slice(&5u64.to_le_bytes());
    out.extend_from_slice(b"ab");
    assert!(parse_error(&out).starts_with("bundle is truncated: module 0 payload needs 5 bytes"));
}

#[test]
fn rejects_trailing_bytes() {
    let mut bytes = valid();
    bytes.push(0);
    assert_eq!(parse_error(&bytes), "bundle has 1 trailing bytes");
}

#[test]
fn writer_rejects_an_empty_otp_release() {
    assert_eq!(
        BundleWriter::new("", COMPILER, "m", "f", 0)
            .unwrap_err()
            .to_string(),
        "bundle OTP release is empty"
    );
}

#[test]
fn writer_rejects_an_incompatible_otp_release() {
    assert_eq!(
        BundleWriter::new("28", COMPILER, "m", "f", 0)
            .unwrap_err()
            .to_string(),
        format!(
            "bundle OTP release 28 is incompatible with this runtime, which supports {OTP_RELEASE}"
        )
    );
}

#[test]
fn writer_rejects_a_duplicate_module_name() {
    let mut writer = BundleWriter::new(OTP_RELEASE, COMPILER, "m", "f", 0).unwrap();
    writer.add_module("m", b"a").unwrap();
    assert_eq!(
        writer.add_module("m", b"b").unwrap_err().to_string(),
        "duplicate bundle module name m"
    );
}

#[test]
fn writer_rejects_a_duplicate_resource_name() {
    let mut writer = BundleWriter::new(OTP_RELEASE, COMPILER, "m", "f", 0).unwrap();
    writer.add_resource("r", b"a").unwrap();
    assert_eq!(
        writer.add_resource("r", b"b").unwrap_err().to_string(),
        "duplicate bundle resource name r"
    );
}

#[test]
fn writer_accepts_an_empty_module_name() {
    let mut writer = BundleWriter::new(OTP_RELEASE, COMPILER, "m", "f", 0).unwrap();
    writer.add_module("", b"a").unwrap();
    assert_eq!(writer.bundle().module(""), Some(b"a".as_slice()));
}

#[test]
fn writer_rejects_a_module_name_over_the_character_limit() {
    let mut writer = BundleWriter::new(OTP_RELEASE, COMPILER, "m", "f", 0).unwrap();
    let name = "é".repeat(256);
    assert_eq!(
        writer.add_module(&name, b"a").unwrap_err().to_string(),
        "bundle module name is 256 characters, over the limit 255"
    );
}

#[test]
fn writer_rejects_a_module_name_over_the_byte_limit() {
    let mut writer = BundleWriter::new(OTP_RELEASE, COMPILER, "m", "f", 0).unwrap();
    let name = "a".repeat(1021);
    assert_eq!(
        writer.add_module(&name, b"a").unwrap_err().to_string(),
        "bundle module name is 1021 bytes, over the limit 1020"
    );
}

#[test]
fn writer_rejects_an_empty_resource_name() {
    let mut writer = BundleWriter::new(OTP_RELEASE, COMPILER, "m", "f", 0).unwrap();
    assert_eq!(
        writer.add_resource("", b"a").unwrap_err().to_string(),
        "bundle resource name is empty"
    );
}

#[test]
fn writer_rejects_a_resource_name_over_the_limit() {
    let mut writer = BundleWriter::new(OTP_RELEASE, COMPILER, "m", "f", 0).unwrap();
    let name = "a".repeat(1025);
    assert_eq!(
        writer.add_resource(&name, b"a").unwrap_err().to_string(),
        "bundle resource name is 1025 bytes, over the limit 1024"
    );
}

#[test]
fn writer_rejects_an_otp_release_over_the_limit() {
    let otp = "a".repeat(17);
    assert_eq!(
        BundleWriter::new(&otp, COMPILER, "m", "f", 0)
            .unwrap_err()
            .to_string(),
        format!("bundle OTP release is 17 bytes, over the limit {MAX_OTP_RELEASE_BYTES}")
    );
}

#[test]
fn writer_rejects_a_compiler_identity_over_the_limit() {
    let compiler = "a".repeat(65);
    assert_eq!(
        BundleWriter::new(OTP_RELEASE, &compiler, "m", "f", 0)
            .unwrap_err()
            .to_string(),
        "bundle compiler identity is 65 bytes, over the limit 64"
    );
}

#[test]
fn writer_rejects_a_full_module_table() {
    let names = (0..=MAX_MODULES)
        .map(|index| format!("m{index}"))
        .collect::<Vec<_>>();
    let mut writer = BundleWriter::new(OTP_RELEASE, COMPILER, "m", "f", 0).unwrap();
    for (index, name) in names.iter().enumerate() {
        let result = writer.add_module(name, b"");
        if index < MAX_MODULES as usize {
            result.unwrap();
        } else {
            assert_eq!(
                result.unwrap_err().to_string(),
                format!("bundle module table is full at {MAX_MODULES} entries")
            );
        }
    }
}

#[test]
fn cursor_reads_each_field() {
    let bytes = [
        0x01, 0x02, // u16
        0x03, 0x04, 0x05, 0x06, // u32
        0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, // u64
    ];
    let mut cursor = Cursor::new(&bytes);
    assert_eq!(cursor.u16("x").unwrap(), 0x0201);
    assert_eq!(cursor.u32("x").unwrap(), 0x0605_0403);
    assert_eq!(cursor.u64("x").unwrap(), 0x0e0d_0c0b_0a09_0807);
    assert!(cursor.is_empty());
    assert_eq!(cursor.remaining(), 0);
}

#[test]
fn cursor_reports_each_truncation() {
    let mut cursor = Cursor::new(&[]);
    assert!(
        cursor
            .u16("x")
            .unwrap_err()
            .to_string()
            .contains("truncated")
    );
    let mut cursor = Cursor::new(&[0; 2]);
    assert!(
        cursor
            .u32("x")
            .unwrap_err()
            .to_string()
            .contains("truncated")
    );
    let mut cursor = Cursor::new(&[0; 4]);
    assert!(
        cursor
            .u64("x")
            .unwrap_err()
            .to_string()
            .contains("truncated")
    );
    assert!(
        cursor
            .take(5, "x")
            .unwrap_err()
            .to_string()
            .contains("truncated")
    );
    assert!(
        cursor
            .take_u64(u64::MAX, "x")
            .unwrap_err()
            .to_string()
            .contains("truncated")
    );
}
