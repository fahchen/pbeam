//! Build the PCRE2 10.47 sources of OTP 29.1.1 in `vendor/pcre2`, which
//! `re:compile/2` uses.

/// The PCRE2 sources that the compile path and the `export` option link.
/// ERTS builds them with `-DERLANG_INTEGRATION` (`pcre.mk:60`), so
/// `pcre2_internal.h` includes `local_config.h`.
const SOURCES: [&str; 19] = [
    "pcre2_auto_possess.c",
    "pcre2_chartables.c",
    "pcre2_chkdint.c",
    "pcre2_compile.c",
    "pcre2_compile_cgroup.c",
    "pcre2_compile_class.c",
    "pcre2_context.c",
    "pcre2_error.c",
    "pcre2_find_bracket.c",
    "pcre2_newline.c",
    "pcre2_ord2utf.c",
    "pcre2_pattern_info.c",
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
    println!("cargo:rerun-if-changed=mise.toml");
    println!("cargo:rustc-env=PBEAM_OTP_RELEASE={}", otp_release());
    cc::Build::new()
        .files(
            SOURCES
                .iter()
                .map(|source| format!("vendor/pcre2/{source}")),
        )
        .include("vendor/pcre2")
        .define("ERLANG_INTEGRATION", None)
        .opt_level(2)
        .warnings(false)
        .compile("pcre2");
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
