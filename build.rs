//! Build the PCRE2 10.47 sources of OTP 29.1.1 in `vendor/pcre2`, which
//! `re:compile/2` and `re:run/2,3` use.

use std::fmt::Write as _;
use std::path::PathBuf;

/// The PCRE2 sources that the compile path, the match path, and the `export`
/// option link, as the `PCRE_O` list of `pcre.mk:23-48` gives the other
/// sources of the library. ERTS builds them with `-DERLANG_INTEGRATION`
/// (`pcre.mk:60`), so `pcre2_internal.h` includes `local_config.h`.
const SOURCES: [&str; 23] = [
    "pcre2_auto_possess.c",
    "pcre2_chartables.c",
    "pcre2_chkdint.c",
    "pcre2_compile.c",
    "pcre2_compile_cgroup.c",
    "pcre2_compile_class.c",
    "pcre2_context.c",
    "pcre2_error.c",
    "pcre2_extuni.c",
    "pcre2_find_bracket.c",
    "pcre2_match.c",
    "pcre2_match_data.c",
    "pcre2_newline.c",
    "pcre2_ord2utf.c",
    "pcre2_pattern_info.c",
    "pcre2_script_run.c",
    "pcre2_serialize.c",
    "pcre2_string_utils.c",
    "pcre2_study.c",
    "pcre2_tables.c",
    "pcre2_ucd.c",
    "pcre2_valid_utf.c",
    "pcre2_xclass.c",
];

/// Compile and link the PCRE2 sources. The optimization level is 2 in each
/// profile: at level 0, each nesting level of a pattern uses about 3 times
/// more stack, and the compile stack guard then fails for patterns that
/// ERTS compiles. The sources are third-party code, so their warnings are
/// off, as `pcre.mk` removes `-Wimplicit-fallthrough`.
///
/// # Examples
///
/// ```text
/// cargo build  # runs main() and links libpcre2.a
/// ```
fn main() {
    println!("cargo:rerun-if-changed=vendor/pcre2");
    println!("cargo:rerun-if-changed=src/runtime/pcre2_serialize_guard.c");
    println!("cargo:rerun-if-changed=mise.toml");
    println!("cargo:rustc-env=PBEAM_OTP_RELEASE={}", otp_release());
    export_nif_api();
    let generated = generate_match_headers();
    cc::Build::new()
        .files(
            SOURCES
                .iter()
                .map(|source| format!("vendor/pcre2/{source}")),
        )
        .file("src/runtime/pcre2_serialize_guard.c")
        .include("vendor/pcre2")
        .include(&generated)
        .define("ERLANG_INTEGRATION", None)
        .opt_level(2)
        .warnings(false)
        .compile("pcre2");
}

/// Export the `enif_*` functions of `src/runtime/nif/abi/env.rs` from each
/// executable of the package, so the dynamic linker binds the undefined
/// `enif_*` symbols of a NIF shared library to them. A Linux executable
/// exports no symbol without a linker option; a macOS executable exports
/// each global symbol.
///
/// # Examples
///
/// ```text
/// cargo:rustc-link-arg=-Wl,--export-dynamic-symbol=enif_*  # on Linux
/// ```
fn export_nif_api() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg=-Wl,--export-dynamic-symbol=enif_*");
    }
}

/// Write the match headers of `pcre2_match.c` into `OUT_DIR` and return that
/// directory, as the three rules of `pcre.mk:74-94` make them from the line
/// numbers of the `COST_CHK` and `MEMCHR_ERLANG` calls of the source:
///
/// - `pcre2_match_loop_break_cases.gen.h` holds the `case` label of each
///   `COST_CHK` call, which the `LOOP_COUNT_BREAK` switch uses to resume a
///   match.
/// - `pcre2_match_memchr_break_cases.gen.h` holds the `case` label of each
///   `MEMCHR_ERLANG` call, which the restart switch of `pcre2_match` uses.
/// - `pcre2_match_yield_coverage.gen.h` numbers each yield point, which the
///   `DEBUG` build counts. The file is written for each build, as the
///   `PCRE_YIELD_COV` rule does.
///
/// Each rule skips a line that holds `define`, which is the `COST_CHK` or
/// `MEMCHR_ERLANG` macro itself or a `DBG_FAKE_` call.
///
/// # Examples
///
/// ```text
/// let headers = generate_match_headers();  // target/debug/build/pbeam-*/out
/// assert!(headers.join("pcre2_match_loop_break_cases.gen.h").exists());
/// ```
fn generate_match_headers() -> PathBuf {
    let source = std::fs::read_to_string("vendor/pcre2/pcre2_match.c").expect("read pcre2_match.c");
    let lines: Vec<(&str, usize)> = source
        .lines()
        .enumerate()
        .map(|(index, line)| (line, index + 1))
        .collect();
    let calls = |needle: &str| -> Vec<usize> {
        lines
            .iter()
            .filter(|(line, _)| {
                line.contains(needle) && !line.contains("define") && !line.contains("DBG_FAKE_")
            })
            .map(|&(_, number)| number)
            .collect()
    };
    let loop_break = calls("COST_CHK(");
    let memchr_break = calls("MEMCHR_ERLANG(");
    let mut yield_coverage = String::new();
    for (index, number) in loop_break.iter().chain(&memchr_break).enumerate() {
        let _ = writeln!(
            yield_coverage,
            "#define ERLANG_YIELD_POINT_{number} {index}"
        );
    }
    let _ = writeln!(
        yield_coverage,
        "#define ERLANG_YIELD_POINT_CNT {}",
        loop_break.len() + memchr_break.len()
    );
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let headers = [
        (
            "pcre2_match_loop_break_cases.gen.h",
            loop_break.iter().fold(String::new(), |mut cases, number| {
                let _ = writeln!(cases, "case {number}: goto L_LOOP_COUNT_{number};");
                cases
            }),
        ),
        (
            "pcre2_match_memchr_break_cases.gen.h",
            memchr_break
                .iter()
                .fold(String::new(), |mut cases, number| {
                    let _ = writeln!(
                        cases,
                        "case {number}: goto ERLANG_PCRE2_MATCH_YIELD_LINE_{number};"
                    );
                    cases
                }),
        ),
        ("pcre2_match_yield_coverage.gen.h", yield_coverage),
    ];
    for (name, contents) in headers {
        std::fs::write(out_dir.join(name), contents).expect("write match header");
    }
    out_dir
}

/// Return the OTP release of the pinned ERTS: the major part of the
/// `erlang` version in `mise.toml`. OTP takes `ERLANG_OTP_RELEASE` from
/// `OTP_VERSION` in the same way (`make/configure.ac:204`), so `29.1.1`
/// gives `29`.
///
/// ```text
/// erlang = "29.1.1"  # in mise.toml, gives 29
/// ```
fn otp_release() -> String {
    let pins = std::fs::read_to_string("mise.toml").expect("read mise.toml");
    let version = pins
        .lines()
        .find_map(|line| line.strip_prefix("erlang = \""))
        .expect("mise.toml pins erlang");
    let release = version.split(['.', '"']).next().unwrap_or_default();
    assert!(
        !release.is_empty() && release.bytes().all(|byte| byte.is_ascii_digit()),
        "unexpected erlang pin {version}"
    );
    release.to_owned()
}
