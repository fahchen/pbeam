//! Build the PCRE2 10.47 sources of OTP 29.1.1 in `vendor/pcre2`, which
//! `re:compile/2` uses.

/// The PCRE2 sources that the compile path links. ERTS builds them with
/// `-DERLANG_INTEGRATION` (`pcre.mk:60`), so `pcre2_internal.h` includes
/// `local_config.h`.
const SOURCES: [&str; 18] = [
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
