//! The PCRE2 10.47 library of OTP 29.1.1, which `build.rs` compiles from
//! `vendor/pcre2` (`erts/emulator/pcre` at the `OTP-29.1.1` tag). ERTS
//! compiles each regular expression of `re:compile/2` with it
//! (`erl_bif_re.c`), so PBeam gets the same messages, offsets, and capture
//! counts. The library uses 8-bit code units (`PCRE2_CODE_UNIT_WIDTH 8` in
//! `local_config.h`), so each function name has the suffix `_8`.

use std::ffi::{c_int, c_void};
use std::ptr::NonNull;

/// `PCRE2_ANCHORED` (`pcre2.h:103`): the `anchored` option.
pub(super) const ANCHORED: u32 = 0x8000_0000;
/// `PCRE2_CASELESS` (`pcre2.h:120`): the `caseless` option.
pub(super) const CASELESS: u32 = 0x0000_0008;
/// `PCRE2_DOLLAR_ENDONLY` (`pcre2.h:121`): the `dollar_endonly` option.
pub(super) const DOLLAR_ENDONLY: u32 = 0x0000_0010;
/// `PCRE2_DOTALL` (`pcre2.h:122`): the `dotall` option.
pub(super) const DOTALL: u32 = 0x0000_0020;
/// `PCRE2_DUPNAMES` (`pcre2.h:123`): the `dupnames` option.
pub(super) const DUPNAMES: u32 = 0x0000_0040;
/// `PCRE2_EXTENDED` (`pcre2.h:124`): the `extended` option.
pub(super) const EXTENDED: u32 = 0x0000_0080;
/// `PCRE2_FIRSTLINE` (`pcre2.h:125`): the `firstline` option.
pub(super) const FIRSTLINE: u32 = 0x0000_0100;
/// `PCRE2_MULTILINE` (`pcre2.h:127`): the `multiline` option.
pub(super) const MULTILINE: u32 = 0x0000_0400;
/// `PCRE2_NEVER_UTF` (`pcre2.h:129`): the `never_utf` option.
pub(super) const NEVER_UTF: u32 = 0x0000_1000;
/// `PCRE2_NO_AUTO_CAPTURE` (`pcre2.h:130`): the `no_auto_capture` option.
pub(super) const NO_AUTO_CAPTURE: u32 = 0x0000_2000;
/// `PCRE2_NO_START_OPTIMIZE` (`pcre2.h:133`): the `no_start_optimize`
/// option.
pub(super) const NO_START_OPTIMIZE: u32 = 0x0001_0000;
/// `PCRE2_UCP` (`pcre2.h:134`): the `ucp` option.
pub(super) const UCP: u32 = 0x0002_0000;
/// `PCRE2_UNGREEDY` (`pcre2.h:135`): the `ungreedy` option.
pub(super) const UNGREEDY: u32 = 0x0004_0000;
/// `PCRE2_UTF` (`pcre2.h:136`): the `unicode` option.
pub(super) const UTF: u32 = 0x0008_0000;
/// `PCRE2_NOTBOL` (`pcre2.h:181`): the `notbol` option.
pub(super) const NOTBOL: u32 = 0x0000_0001;
/// `PCRE2_NOTEOL` (`pcre2.h:182`): the `noteol` option.
pub(super) const NOTEOL: u32 = 0x0000_0002;
/// `PCRE2_NOTEMPTY` (`pcre2.h:183`): the `notempty` option.
pub(super) const NOTEMPTY: u32 = 0x0000_0004;
/// `PCRE2_NOTEMPTY_ATSTART` (`pcre2.h:184`): the `notempty_atstart` option.
pub(super) const NOTEMPTY_ATSTART: u32 = 0x0000_0008;
/// `PCRE2_NO_UTF_CHECK` (`pcre2.h:104`): the match does not check the UTF-8
/// form of the subject. `re:internal_run/4` sets it with `first` false,
/// because `re:grun/3` checked the subject already
/// (`erl_bif_re.c:1505-1512`).
pub(super) const NO_UTF_CHECK: u32 = 0x4000_0000;

/// `PCRE2_NEWLINE_CR` (`pcre2.h:215`): `{newline, cr}`.
pub(super) const NEWLINE_CR: u32 = 1;
/// `PCRE2_NEWLINE_LF` (`pcre2.h:216`): `{newline, lf}`, the default
/// (`NEWLINE_DEFAULT` in `local_config.h`).
pub(super) const NEWLINE_LF: u32 = 2;
/// `PCRE2_NEWLINE_CRLF` (`pcre2.h:217`): `{newline, crlf}`.
pub(super) const NEWLINE_CRLF: u32 = 3;
/// `PCRE2_NEWLINE_ANY` (`pcre2.h:218`): `{newline, any}`.
pub(super) const NEWLINE_ANY: u32 = 4;
/// `PCRE2_NEWLINE_ANYCRLF` (`pcre2.h:219`): `{newline, anycrlf}`.
pub(super) const NEWLINE_ANYCRLF: u32 = 5;
/// `PCRE2_NEWLINE_NUL` (`pcre2.h:220`): `{newline, nul}`.
pub(super) const NEWLINE_NUL: u32 = 6;

/// `PCRE2_BSR_UNICODE` (`pcre2.h:222`): the `bsr_unicode` option.
pub(super) const BSR_UNICODE: u32 = 1;
/// `PCRE2_BSR_ANYCRLF` (`pcre2.h:223`): the `bsr_anycrlf` option.
pub(super) const BSR_ANYCRLF: u32 = 2;

/// `PCRE2_ERROR_BADDATA` (`pcre2.h:395`): `pcre2_get_error_message` does
/// not know the error code.
const ERROR_BADDATA: c_int = -29;
/// `PCRE2_ERROR_BADMAGIC` (`pcre2.h:397`).
pub(super) const ERROR_BADMAGIC: c_int = -31;
/// `PCRE2_ERROR_NOMATCH` (`pcre2.h:352`): the subject does not match.
pub(super) const ERROR_NOMATCH: c_int = -1;
/// `PCRE2_ERROR_BADMODE` (`pcre2.h:398`).
pub(super) const ERROR_BADMODE: c_int = -32;
/// `PCRE2_ERROR_BADOFFSET` (`pcre2.h:399`): the start offset is past the
/// end of the subject.
pub(super) const ERROR_BADOFFSET: c_int = -33;
/// `PCRE2_ERROR_BADUTFOFFSET` (`pcre2.h:402`): the start offset is not on a
/// UTF-8 character boundary.
pub(super) const ERROR_BADUTFOFFSET: c_int = -36;
/// `PCRE2_ERROR_RECURSELOOP` (`pcre2.h:418`): the pattern recursed without
/// a match, which `re_run` charges as all reductions of one turn
/// (`erl_bif_re.c:1751-1754`).
pub(super) const ERROR_RECURSELOOP: c_int = -52;
/// `PCRE2_ERROR_MATCHLIMIT` (`pcre2.h:413`).
pub(super) const ERROR_MATCHLIMIT: c_int = -47;
/// `PCRE2_ERROR_DEPTHLIMIT` (`pcre2.h:419`), which the obsolete name
/// `PCRE2_ERROR_RECURSIONLIMIT` also gives.
pub(super) const ERROR_DEPTHLIMIT: c_int = -53;
/// `PCRE2_ERROR_HEAPLIMIT` (`pcre2.h:430`): the match used the heap limit.
pub(super) const ERROR_HEAPLIMIT: c_int = -63;
/// `PCRE2_ERROR_LOOP_LIMIT` (`pcre2.h:444`), the ERTS addition that a match
/// returns when it uses the loops that remain.
pub(super) const ERROR_LOOP_LIMIT: c_int = -76;
/// `PCRE2_INFO_CAPTURECOUNT` (`pcre2.h:453`): the highest capture group
/// number, as a `uint32_t`.
const INFO_CAPTURECOUNT: u32 = 4;
/// `PCRE2_INFO_BSR` (`pcre2.h:452`): the `\R` convention of the compiled
/// code, as a `uint32_t` `PCRE2_BSR_*` value.
const INFO_BSR: u32 = 3;
/// `PCRE2_INFO_ALLOPTIONS` (`pcre2.h:449`): the options that the compile
/// set, as a `uint32_t` `PCRE2_*` value.
const INFO_ALLOPTIONS: u32 = 0;
/// `PCRE2_INFO_NAMECOUNT` (`pcre2.h:466`): the number of named capture
/// groups, as a `uint32_t`.
const INFO_NAMECOUNT: u32 = 17;
/// `PCRE2_INFO_NAMEENTRYSIZE` (`pcre2.h:467`): the size of one entry of
/// the name table, as a `uint32_t`.
const INFO_NAMEENTRYSIZE: u32 = 18;
/// `PCRE2_INFO_NEWLINE` (`pcre2.h:469`): the newline convention of the
/// compiled code, as a `uint32_t` `PCRE2_NEWLINE_*` value.
const INFO_NEWLINE: u32 = 20;
/// `PCRE2_UNSET` (`pcre2.h:314`): an ovector slot of a capture group that
/// did not take part in the match.
pub(super) const UNSET: usize = usize::MAX;

/// The size of the error message buffer of `build_compile_error`
/// (`erl_bif_re.c:809`). A longer message is cut to 119 bytes.
const ERROR_MESSAGE_SIZE: usize = 120;

/// The stack that the compile stack guard keeps free below the stack limit,
/// as `ERTS_PCRE_STACK_MARGIN` (`erl_bif_re.c:157`).
const STACK_MARGIN: usize = 10 * 1024;

// The PCRE2 functions that the compile path, the match path, and the
// `export` option use (`pcre2.h:650-790`, `pcre2.h:826-834`, and
// `pcre2.h:878`). `pcre2_code_8`, `pcre2_compile_context_8`,
// `pcre2_general_context_8`, `pcre2_match_context_8`, and
// `pcre2_match_data_8` are opaque C structs, so a `c_void` pointer stands
// for each.
unsafe extern "C" {
    fn pcre2_compile_context_create_8(general: *mut c_void) -> *mut c_void;
    fn pcre2_compile_context_free_8(context: *mut c_void);
    fn pcre2_set_newline_8(context: *mut c_void, newline: u32) -> c_int;
    fn pcre2_set_bsr_8(context: *mut c_void, bsr: u32) -> c_int;
    fn pcre2_set_compile_recursion_guard_8(
        context: *mut c_void,
        guard: unsafe extern "C" fn(u32, *mut c_void) -> c_int,
        data: *mut c_void,
    ) -> c_int;
    fn pcre2_compile_8(
        pattern: *const u8,
        length: usize,
        options: u32,
        error_code: *mut c_int,
        error_offset: *mut usize,
        context: *mut c_void,
    ) -> *mut c_void;
    fn pcre2_pattern_info_8(code: *const c_void, what: u32, place: *mut c_void) -> c_int;
    fn pcre2_get_error_message_8(code: c_int, buffer: *mut u8, size: usize) -> c_int;
    fn pcre2_match_data_create_8(ovec_count: u32, general: *mut c_void) -> *mut c_void;
    fn pcre2_match_data_free_8(data: *mut c_void);
    fn pcre2_get_ovector_pointer_8(data: *mut c_void) -> *mut usize;
    fn pcre2_set_loops_left_8(data: *mut c_void, budget: c_int);
    fn pcre2_get_loops_left_8(data: *mut c_void) -> c_int;
    fn pcre2_set_restart_data_8(data: *mut c_void, restart: *mut *mut c_void);
    fn pcre2_set_restart_flags_8(data: *mut c_void, flags: u32);
    fn pcre2_match_context_create_8(general: *mut c_void) -> *mut c_void;
    fn pcre2_match_context_free_8(context: *mut c_void);
    fn pcre2_set_match_limit_8(context: *mut c_void, limit: u32) -> c_int;
    fn pcre2_set_depth_limit_8(context: *mut c_void, limit: u32) -> c_int;
    fn pcre2_free_restart_data_8(data: *mut c_void);
    fn pcre2_match_8(
        code: *const c_void,
        subject: *const u8,
        length: usize,
        start_offset: usize,
        options: u32,
        data: *mut c_void,
        context: *mut c_void,
    ) -> c_int;
    fn pcre2_serialize_encode_8(
        codes: *const *const c_void,
        number_of_codes: i32,
        serialized_bytes: *mut *mut u8,
        serialized_size: *mut usize,
        general: *mut c_void,
    ) -> i32;
    fn pcre2_serialize_free_8(bytes: *mut u8);
    fn pcre2_serialize_get_number_of_codes_8(bytes: *const u8) -> i32;
    fn pcre2_serialize_decode_8(
        codes: *mut *mut c_void,
        number_of_codes: i32,
        bytes: *const u8,
        general: *mut c_void,
    ) -> i32;
    fn pbeam_pcre2_name_table(
        code: *const c_void,
        buffer: *mut u8,
        size: usize,
        count: *mut u32,
        entry_size: *mut u32,
    ) -> c_int;
    fn pbeam_pcre2_serialized_header_size() -> usize;
    fn pbeam_pcre2_serialized_tables_size() -> usize;
    fn pbeam_pcre2_serialized_blocksize_offset() -> usize;
    fn pbeam_pcre2_serialized_code_size() -> usize;
    #[cfg(test)]
    fn pbeam_pcre2_serialized_magic_offset() -> usize;
    #[cfg(test)]
    fn pbeam_pcre2_serialized_code_start_offset() -> usize;
    fn pbeam_pcre2_serialized_number_of_codes_offset() -> usize;
    fn pbeam_pcre2_serialized_code_is_valid(bytes: *const u8, blocksize: usize) -> c_int;
    fn pcre2_code_free_8(code: *mut c_void);
}

/// One compiled regular expression: a `pcre2_code` that this value owns
/// and frees when it drops, as `regex_code_destructor`
/// (`erl_bif_re.c:117-125`) does.
#[derive(Debug)]
pub(super) struct Code(
    /// The owned pointer to the live `pcre2_code` that
    /// `pcre2_compile_8` returned. The value is opaque to Rust, and only
    /// `pcre2_code_free_8` in `Drop` frees it.
    NonNull<c_void>,
);

/// A compile error of PCRE2: the message of `pcre2_get_error_message` and
/// the offset in the pattern where the error was found.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct CompileError {
    /// The message bytes, without the final NUL byte.
    pub(super) message: Vec<u8>,
    /// The byte offset in the pattern.
    pub(super) offset: usize,
}

impl Code {
    /// Return the highest capture group number (`PCRE2_INFO_CAPTURECOUNT`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(compile(b"(a)(?:b)(c)", 0, None, None)?.capture_count(), 2);
    /// ```
    pub(super) fn capture_count(&self) -> u32 {
        self.info(INFO_CAPTURECOUNT)
    }

    /// Return the newline convention (`PCRE2_INFO_NEWLINE`): the
    /// `{newline, X}` option, a `(*CRLF)` item of the pattern, or
    /// `NEWLINE_LF`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(compile(b"a", 0, None, None)?.newline(), NEWLINE_LF);
    /// assert_eq!(compile(b"(*CRLF)a", 0, None, None)?.newline(), NEWLINE_CRLF);
    /// ```
    pub(super) fn newline(&self) -> u32 {
        self.info(INFO_NEWLINE)
    }

    /// Return the `\R` convention (`PCRE2_INFO_BSR`): the `bsr_anycrlf` or
    /// `bsr_unicode` option of the compile, a `(*BSR_ANYCRLF)` item of the
    /// pattern, or `BSR_UNICODE`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(compile(b"a", 0, None, None)?.bsr(), BSR_UNICODE);
    /// assert_eq!(compile(b"a", 0, None, Some(BSR_ANYCRLF))?.bsr(), BSR_ANYCRLF);
    /// ```
    pub(super) fn bsr(&self) -> u32 {
        self.info(INFO_BSR)
    }

    /// Return the number of named capture groups (`PCRE2_INFO_NAMECOUNT`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(compile(b"(?<a>x)(?<b>y)", 0, None, None)?.name_count(), 2);
    /// assert_eq!(compile(b"(a)", 0, None, None)?.name_count(), 0);
    /// ```
    pub(super) fn name_count(&self) -> u32 {
        self.info(INFO_NAMECOUNT)
    }

    /// Return whether the compile set `PCRE2_DUPNAMES`, as
    /// `build_capture` reads `PCRE2_INFO_ALLOPTIONS`
    /// (`erl_bif_re.c:1379`). The inline item `(?J)` allows duplicate
    /// names but does not set the bit.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(compile(b"(?<a>x)(?<a>y)", DUPNAMES, None, None)?.has_dupnames());
    /// assert!(!compile(b"(?J)(?<a>x)(?<a>y)", 0, None, None)?.has_dupnames());
    /// ```
    pub(super) fn has_dupnames(&self) -> bool {
        self.info(INFO_ALLOPTIONS) & DUPNAMES != 0
    }

    /// Return the entries of the name table of the code, as
    /// `pcre2_substring_nametable_scan` reads them for `build_one_capture`
    /// (`erl_bif_re.c:1269-1307`): the number of one named group and the
    /// bytes of its name without the final NUL byte. PCRE2 sorts the
    /// entries by name, and the entries of one name are adjacent.
    ///
    /// The C shim `pbeam_pcre2_name_table` copies the table into a buffer
    /// that this value owns, as `pbeam_pcre2_serialized_code_is_valid`
    /// checks serialized bytes. Rust then reads the copy.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let code = compile(b"(?<z>a)(?<m>b)", 0, None, None)?;
    /// assert_eq!(code.names(), vec![(2, b"m".to_vec()), (1, b"z".to_vec())]);
    /// assert!(compile(b"(a)", 0, None, None)?.names().is_empty());
    /// ```
    pub(super) fn names(&self) -> Vec<(u32, Vec<u8>)> {
        let count = self.name_count() as usize;
        if count == 0 {
            return Vec::new();
        }
        let entry_size = self.info(INFO_NAMEENTRYSIZE) as usize;
        let mut table = vec![0u8; count * entry_size];
        let mut copied_count = 0u32;
        let mut copied_entry_size = 0u32;
        // SAFETY: `self.0` is a live `pcre2_code` that only this value
        // owns. `table` has `count * entry_size` writable bytes, and the
        // shim copies at most that many bytes into it.
        let result = unsafe {
            pbeam_pcre2_name_table(
                self.0.as_ptr(),
                table.as_mut_ptr(),
                table.len(),
                &raw mut copied_count,
                &raw mut copied_entry_size,
            )
        };
        assert_eq!(result, 0, "PCRE2 name table copy failed");
        assert_eq!(
            (copied_count, copied_entry_size),
            (count as u32, entry_size as u32)
        );
        table
            .chunks_exact(entry_size)
            .map(|entry| {
                let number = u32::from(u16::from_be_bytes([entry[0], entry[1]]));
                let end = entry[2..]
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(entry_size - 2)
                    + 2;
                (number, entry[2..end].to_vec())
            })
            .collect()
    }

    /// Return the serialized form of this code, as `build_compile_export`
    /// (`erl_bif_re.c:663-665`) makes it with `pcre2_serialize_encode`: a
    /// header with the magic number, the version, and the configuration of
    /// PCRE2, the character tables, and a copy of the code in which the
    /// memory control, table, and JIT fields are zero
    /// (`pcre2_serialize.c:70-154`). The bytes are equal for equal patterns
    /// and options.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let encoded = compile(b"a", 0, None, None)?.serialize();
    /// assert_eq!(encoded.len(), 1265); // aarch64 macOS, as ERTS
    /// // SERIALIZED_DATA_MAGIC (pcre2_serialize.c:53), in host byte order
    /// assert_eq!(&encoded[..4], &0x5052_3253u32.to_ne_bytes());
    /// ```
    pub(super) fn serialize(&self) -> Vec<u8> {
        let code = self.0.as_ptr().cast_const();
        let mut bytes = std::ptr::null_mut();
        let mut size = 0;
        // SAFETY: `code` is a live `pcre2_code`, and the array of one
        // pointer is valid for the call. With no general context, PCRE2
        // allocates the buffer with `malloc` and writes its address and size
        // into `bytes` and `size`. The slice copies `size` bytes from it
        // before `pcre2_serialize_free_8` frees it once.
        unsafe {
            let count = pcre2_serialize_encode_8(
                &raw const code,
                1,
                &raw mut bytes,
                &raw mut size,
                std::ptr::null_mut(),
            );
            assert_eq!(count, 1, "PCRE2 serialize failed with {count}");
            let encoded = std::slice::from_raw_parts(bytes, size).to_vec();
            pcre2_serialize_free_8(bytes);
            encoded
        }
    }

    /// Return one `uint32_t` item of `pcre2_pattern_info`. ERTS ignores the
    /// return code (`erl_bif_re.c:595-596`), because each item it asks for
    /// exists.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(compile(b"(a)", 0, None, None)?.info(INFO_CAPTURECOUNT), 1);
    /// ```
    fn info(&self, what: u32) -> u32 {
        let mut value = 0u32;
        // SAFETY: `self.0` is a live `pcre2_code` that only this value owns.
        // Each item that this module asks for is a `uint32_t`
        // (`pcre2_pattern_info.c`), so PCRE2 writes 4 bytes into `value`.
        unsafe { pcre2_pattern_info_8(self.0.as_ptr(), what, (&raw mut value).cast()) };
        value
    }
}

impl Drop for Code {
    /// Free the compiled code.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// drop(compile(b"a", 0, None, None)?);
    /// ```
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `pcre2_compile_8`, and only this value
        // owns it, so PCRE2 frees it once.
        unsafe { pcre2_code_free_8(self.0.as_ptr()) };
    }
}

/// What `pcre2_serialize_decode` reports for encoded pattern bytes.
#[derive(Debug)]
pub(super) enum Deserialized {
    /// The compiled code.
    Code(Code),
    /// `PCRE2_ERROR_BADMAGIC` or `PCRE2_ERROR_BADMODE`: the bytes came from
    /// another architecture or another PCRE2 version, so ERTS re-compiles
    /// the source (`erl_bif_re.c:783-788`).
    Incompatible,
    /// Another error, which ERTS reports as `badarg`
    /// (`erl_bif_re.c:790-795`).
    Invalid,
}

/// Decode the bytes of one exported pattern, as `re_import_1`
/// (`erl_bif_re.c:761-789`) does with `pcre2_serialize_decode`.
///
/// ERTS gives PCRE2 no length, so truncated or forged bytes make PCRE2 read
/// past the binary. PBeam checks the fixed header, the character tables,
/// and the size of the first code block before the call. The block size
/// check uses the layout constants that the C shim
/// `src/runtime/pcre2_serialize_guard.c` reports.
///
/// # Examples
///
/// ```rust,ignore
/// let code = compile(b"a", 0, None, None)?;
/// let encoded = code.serialize();
/// assert!(matches!(deserialize(&encoded), Deserialized::Code(_)));
/// // The first four bytes are the magic number (pcre2_serialize.c:53).
/// let mut other = encoded;
/// other[..4].fill(0);
/// assert!(matches!(deserialize(&other), Deserialized::Incompatible));
/// assert!(matches!(deserialize(&[]), Deserialized::Invalid));
/// ```
pub(super) fn deserialize(encoded: &[u8]) -> Deserialized {
    // SAFETY: the layout functions read no memory. They return constants of
    // the PCRE2 build that this crate links.
    let (header, tables, blocksize_offset, code_size, count_offset) = unsafe {
        (
            pbeam_pcre2_serialized_header_size(),
            pbeam_pcre2_serialized_tables_size(),
            pbeam_pcre2_serialized_blocksize_offset(),
            pbeam_pcre2_serialized_code_size(),
            pbeam_pcre2_serialized_number_of_codes_offset(),
        )
    };
    if encoded.len() < header {
        return Deserialized::Invalid;
    }
    // `pcre2_serialize_decode` reads this field before the magic
    // (`pcre2_serialize.c:177-180`), so the check keeps the same order.
    let count = i32::from_ne_bytes(
        encoded[count_offset..count_offset + 4]
            .try_into()
            .expect("the header holds the number of codes"),
    );
    if count <= 0 {
        return Deserialized::Invalid;
    }
    // SAFETY: `encoded` is at least as long as the fixed header, which is
    // all that this function reads (`pcre2_serialize.c:249-259`).
    let count = unsafe { pcre2_serialize_get_number_of_codes_8(encoded.as_ptr()) };
    if count == ERROR_BADMAGIC || count == ERROR_BADMODE {
        return Deserialized::Incompatible;
    }
    debug_assert!(count > 0, "a valid header holds a positive code count");
    let tables_end = header + tables;
    let blocksize_at = tables_end + blocksize_offset;
    let word = std::mem::size_of::<usize>();
    if encoded.len() < blocksize_at + word {
        return Deserialized::Invalid;
    }
    let blocksize = usize::from_ne_bytes(
        encoded[blocksize_at..blocksize_at + word]
            .try_into()
            .expect("the word holds the block size"),
    );
    if blocksize <= code_size || blocksize > encoded.len() - tables_end {
        return Deserialized::Invalid;
    }
    // SAFETY: the code block holds at least `code_size` bytes, which is all
    // that this function reads.
    let valid =
        unsafe { pbeam_pcre2_serialized_code_is_valid(encoded[tables_end..].as_ptr(), blocksize) };
    if valid == 0 {
        return Deserialized::Invalid;
    }
    let mut code = std::ptr::null_mut();
    // SAFETY: `encoded` holds the table block and a code block of
    // `blocksize` bytes, which is all that the decode reads. The block
    // passes the field checks of the decode and the code range check, so
    // the call allocates no tables that it leaves allocated on an error.
    // With no general context, PCRE2 allocates the code with the default
    // allocator, which `pcre2_code_free_8` in `Drop` uses.
    let decoded = unsafe {
        pcre2_serialize_decode_8(&raw mut code, 1, encoded.as_ptr(), std::ptr::null_mut())
    };
    assert_eq!(decoded, 1, "PCRE2 decode of a checked block");
    Deserialized::Code(Code(
        NonNull::new(code).expect("a decoded code is a live pointer"),
    ))
}

/// One match state: the `pcre2_match_data` of one `re:run/2,3` call, which
/// this value owns and frees when it drops, as `cleanup_restart_context`
/// (`erl_bif_re.c:1013-1017`) frees it. The ovector of the match data gives
/// the offsets of each capture group after a match.
pub(super) struct MatchData {
    /// The owned pointer to the live `pcre2_match_data` that
    /// `pcre2_match_data_create_8` returned. The value is opaque to Rust,
    /// and only `pcre2_match_data_free_8` in `Drop` frees it.
    data: NonNull<c_void>,
    /// The number of offset pairs of the ovector, which is the capture
    /// count of the code plus one for the whole match
    /// (`erl_bif_re.c:1575`, `1607`).
    pairs: usize,
    /// The `restart_data` field of the match data. PCRE2 reads and writes
    /// the field through this pointer, so the field has a stable address
    /// for the life of the match data. ERTS keeps the field in the restart
    /// context of the call and sets it to NULL before a match
    /// (`erl_bif_re.c:1641-1642`). A match that stops at its loop limit
    /// makes PCRE2 write the position of the match into the field
    /// (`pcre2_match.c:8586-8587`), and `Drop` frees that state through
    /// `pcre2_free_restart_data`, as `cleanup_restart_context`
    /// (`erl_bif_re.c:1009-1012`) does.
    restart: Box<*mut c_void>,
}

impl MatchData {
    /// Make the match data of a match with `pairs` offset pairs.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let data = MatchData::new(2);
    /// assert_eq!(data.ovector(2), [UNSET, UNSET, UNSET, UNSET]);
    /// ```
    ///
    /// # Panics
    ///
    /// Panic when PCRE2 cannot allocate the match data.
    pub(super) fn new(pairs: u32) -> Self {
        // PCRE2 keeps at least one pair and at most `UINT16_MAX`
        // (`pcre2_match_data.c:58-59`), and the ovector of the match data
        // has the clamped size.
        let pairs = pairs.clamp(1, u32::from(u16::MAX));
        // SAFETY: `pcre2_match_data_create_8` with no general context uses
        // `malloc`, and the new match data is valid until
        // `pcre2_match_data_free_8` in `Drop`.
        let data = unsafe { pcre2_match_data_create_8(pairs, std::ptr::null_mut()) };
        let data = NonNull::new(data).expect("PCRE2 match data allocation failed");
        let mut restart: Box<*mut c_void> = Box::new(std::ptr::null_mut());
        // SAFETY: `data` is a live match data. The pointer to the boxed
        // field stays valid until `Drop` frees the match data first, and
        // PCRE2 writes no other value into the field without a restart.
        unsafe { pcre2_set_restart_data_8(data.as_ptr(), &raw mut *restart) };
        Self {
            data,
            pairs: pairs as usize,
            restart,
        }
    }

    /// Return the `pairs` offset pairs that PCRE2 wrote into the ovector of
    /// this match data: two offsets, the start and the end of the match, for
    /// each pair. A slot of a capture group that did not take part in the
    /// match is [`UNSET`]. The match returns this count, and PCRE2 leaves
    /// the other pairs of the allocation uninitialized, so the slice stops
    /// at the count.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let data = MatchData::new(1);
    /// assert_eq!(data.ovector(1).len(), 2);
    /// ```
    pub(super) fn ovector(&self, pairs: usize) -> &[usize] {
        let pairs = pairs.min(self.pairs);
        // SAFETY: `self.data` is a live match data with `self.pairs` pairs,
        // and PCRE2 owns its ovector, which stays valid until `Drop`.
        // PCRE2 initialized the first `pairs` pairs of the ovector in the
        // match that returned `pairs`. No other reference to the ovector
        // exists while this slice borrows the match data.
        unsafe {
            std::slice::from_raw_parts(pcre2_get_ovector_pointer_8(self.data.as_ptr()), pairs * 2)
        }
    }

    /// Set the loops that the next match may use, as `re_run`
    /// (`erl_bif_re.c:1640`) does with `reds_initial * LOOP_FACTOR`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut data = MatchData::new(1);
    /// data.set_loops_left(64000);
    /// assert_eq!(data.loops_left(), 64000);
    /// ```
    pub(super) fn set_loops_left(&mut self, loops: i32) {
        // SAFETY: `self.data` is a live match data that this value owns.
        unsafe { pcre2_set_loops_left_8(self.data.as_ptr(), loops) };
    }

    /// Return the loops that the match did not use, as
    /// `pcre2_get_loops_left` gives them to `re_run`
    /// (`erl_bif_re.c:1718`) for the reduction charge.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut data = MatchData::new(1);
    /// data.set_loops_left(100);
    /// assert_eq!(data.loops_left(), 100);
    /// ```
    pub(super) fn loops_left(&self) -> i32 {
        // SAFETY: `self.data` is a live match data that this value owns.
        unsafe { pcre2_get_loops_left_8(self.data.as_ptr()) }
    }

    /// Clear the restart state of a match that a previous call left, as
    /// `re_run` does before each match with `pcre2_set_restart_flags`
    /// (`erl_bif_re.c:1643`). PBeam runs a match in one step, so the field
    /// is NULL already, and this call keeps the invariant of the field.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut data = MatchData::new(1);
    /// data.clear_restart();
    /// ```
    pub(super) fn clear_restart(&mut self) {
        // SAFETY: the box behind `self.restart` gave PCRE2 a pointer to a
        // `*mut c_void` field, and this value owns it. A match that never
        // traps leaves the field NULL.
        unsafe { pcre2_set_restart_flags_8(self.data.as_ptr(), 0) };
    }

    /// Continue a match that used its loop limit, as the trap to
    /// `erlang:re_run_trap/3` does with `pcre2_match(NULL, NULL, 0, 0, 0,
    /// match_data, NULL)` (`erl_bif_re.c:1856`) after it sets a fresh loop
    /// limit. The restart data of the match data holds the position of the
    /// match, so the call continues from there, and the return code is the
    /// same one that a fresh match gives.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let code = compile(b"a+", 0, None, None)?;
    /// let mut data = MatchData::new(2);
    /// data.set_loops_left(1);
    /// assert_eq!(execute(&code, &data, None, b"aa", 0, 0), ERROR_LOOP_LIMIT);
    /// data.set_loops_left(i32::MAX);
    /// data.clear_restart();
    /// assert_eq!(data.resume(), 1);
    /// ```
    pub(super) fn resume(&self) -> c_int {
        // SAFETY: the match data holds the restart state of the interrupted
        // match, and PCRE2 reads NULL code, subject, and context as the
        // restart call of `re_match_trap` passes them (`erl_bif_re.c:1856`).
        unsafe {
            pcre2_match_8(
                std::ptr::null(),
                std::ptr::null(),
                0,
                0,
                0,
                self.data.as_ptr(),
                std::ptr::null_mut(),
            )
        }
    }
}

impl Drop for MatchData {
    /// Free the match data, as `cleanup_restart_context`
    /// (`erl_bif_re.c:1013-1017`) does after a match.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// drop(MatchData::new(1));
    /// ```
    fn drop(&mut self) {
        // SAFETY: `self.data` came from `pcre2_match_data_create_8`, and
        // only this value owns it. The match data holds the pointer to the
        // boxed restart field, which this value owns too, so PCRE2 frees
        // the state of an interrupted match through it once. PCRE2 then
        // frees the match data itself.
        unsafe {
            if !(*self.restart).is_null() {
                pcre2_free_restart_data_8(self.data.as_ptr());
            }
            pcre2_match_data_free_8(self.data.as_ptr());
        }
    }
}

/// The `pcre2_match_context` of one `re:run/2,3` call that sets a match
/// limit or a recursion limit, as `re_run` makes it
/// (`erl_bif_re.c:1654-1665`). The value owns the context and frees it when
/// it drops.
pub(super) struct MatchContext(
    /// The owned pointer to the live `pcre2_match_context` that
    /// `pcre2_match_context_create_8` returned. The value is opaque to
    /// Rust, and only `pcre2_match_context_free_8` in `Drop` frees it.
    NonNull<c_void>,
);

impl MatchContext {
    /// Make the match context of a match that gives a match limit, a
    /// recursion limit, or both. PCRE2 uses its own limit for each one that
    /// is `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let context = MatchContext::new(Some(1), None);
    /// ```
    ///
    /// # Panics
    ///
    /// Panic when PCRE2 cannot allocate the match context.
    pub(super) fn new(match_limit: Option<u32>, depth_limit: Option<u32>) -> Self {
        // SAFETY: `pcre2_match_context_create_8` with no general context
        // uses `malloc`. The new context is valid until
        // `pcre2_match_context_free_8` in `Drop`, and the set functions
        // only write a limit field of the context.
        let context = unsafe { pcre2_match_context_create_8(std::ptr::null_mut()) };
        let context = NonNull::new(context).expect("PCRE2 match context allocation failed");
        unsafe {
            if let Some(limit) = match_limit {
                pcre2_set_match_limit_8(context.as_ptr(), limit);
            }
            if let Some(limit) = depth_limit {
                pcre2_set_depth_limit_8(context.as_ptr(), limit);
            }
        }
        Self(context)
    }
}

impl Drop for MatchContext {
    /// Free the match context, as `cleanup_restart_context`
    /// (`erl_bif_re.c:1018-1021`) does after a match.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// drop(MatchContext::new(None, Some(7)));
    /// ```
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `pcre2_match_context_create_8`, and
        // only this value owns it, so PCRE2 frees it once.
        unsafe { pcre2_match_context_free_8(self.0.as_ptr()) };
    }
}

/// Match `subject` from `offset` with `code`, as `re_run`
/// (`erl_bif_re.c:1712-1716`) does with `pcre2_match`, and return its return
/// code: the number of capture pairs that the match set, 0 for a match
/// whose ovector is too small, or a negative `PCRE2_ERROR_*` value. The
/// offsets of each pair are in `data.ovector(rc)`. The options are the
/// `PCRE2_*` match bits.
///
/// # Examples
///
/// ```rust,ignore
/// let code = compile(b"(a)", 0, None, None)?;
/// let mut data = MatchData::new(code.capture_count() as u32 + 1);
/// data.set_loops_left(i32::MAX);
/// assert_eq!(execute(&code, &data, None, b"ba", 0, 0), 2);
/// assert_eq!(data.ovector(2), [1, 2, 1, 2]);
/// assert_eq!(execute(&code, &data, None, b"ba", 2, 0), ERROR_NOMATCH);
/// ```
pub(super) fn execute(
    code: &Code,
    data: &MatchData,
    context: Option<&MatchContext>,
    subject: &[u8],
    offset: usize,
    options: u32,
) -> c_int {
    // SAFETY: `code` is a live code, `data` is a live match data that
    // holds the restart field that PCRE2 reads and writes, and `subject`
    // and its length come from one slice that outlives the call. The
    // context is a live match context when it is present, and PCRE2 reads
    // it only during the call.
    unsafe {
        pcre2_match_8(
            code.0.as_ptr(),
            subject.as_ptr(),
            subject.len(),
            offset,
            options,
            data.data.as_ptr(),
            context.map_or(std::ptr::null_mut(), |context| context.0.as_ptr()),
        )
    }
}

/// Return whether `rc` is a UTF-8 error of the subject, which `re_run`
/// (`erl_bif_re.c:1756-1777`) turns into `badarg` after it uses all
/// reductions.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_utf8_error(-3));
/// assert!(is_utf8_error(-23));
/// assert!(!is_utf8_error(-24));
/// assert!(!is_utf8_error(ERROR_NOMATCH));
/// ```
pub(super) fn is_utf8_error(rc: c_int) -> bool {
    // `PCRE2_ERROR_UTF8_ERR1` to `PCRE2_ERROR_UTF8_ERR21` (`pcre2.h:357-377`).
    (-23..=-3).contains(&rc)
}

/// Compile `pattern` with the `PCRE2_*` option bits `options`, as
/// `compile` (`erl_bif_re.c:548-578`) does. A `newline` or `bsr` value
/// replaces the default of the compile context. The context has the stack
/// guard of `erts_init_bif_re` (`erl_bif_re.c:183-211`): PCRE2 stops with
/// the error "parentheses are too deeply nested (stack check)" when less
/// than 10 KiB of the thread stack is free.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(compile(b"(a)", UTF, None, None)?.capture_count(), 1);
/// assert_eq!(compile(b"a", 0, Some(NEWLINE_CRLF), Some(BSR_ANYCRLF))?.newline(), NEWLINE_CRLF);
/// let error = compile(b"(a)(b", 0, None, None).unwrap_err();
/// assert_eq!(error, CompileError { message: b"missing closing parenthesis".to_vec(), offset: 5 });
/// ```
///
/// # Errors
///
/// Return the PCRE2 message and offset of a pattern that does not compile.
pub(super) fn compile(
    pattern: &[u8],
    options: u32,
    newline: Option<u32>,
    bsr: Option<u32>,
) -> Result<Code, CompileError> {
    let limit = stack_limit();
    let mut error_code = 0;
    let mut error_offset = 0;
    // SAFETY: `pcre2_compile_context_create_8` with no general context uses
    // `malloc`. The new context is valid until `pcre2_compile_context_free_8`
    // at the end of this block. The set functions only write fields of the
    // context; PCRE2 accepts each `NEWLINE_*` and `BSR_*` value. The guard
    // gets `limit` as its data and reads no memory through it. The pattern
    // pointer and length come from one slice. PCRE2 copies what it needs
    // into the compiled code, so the context and the pattern can go after
    // the compile.
    let code = unsafe {
        let context = pcre2_compile_context_create_8(std::ptr::null_mut());
        assert!(
            !context.is_null(),
            "PCRE2 compile context allocation failed"
        );
        pcre2_set_compile_recursion_guard_8(
            context,
            stack_guard,
            std::ptr::without_provenance_mut(limit),
        );
        if let Some(newline) = newline {
            pcre2_set_newline_8(context, newline);
        }
        if let Some(bsr) = bsr {
            pcre2_set_bsr_8(context, bsr);
        }
        let code = pcre2_compile_8(
            pattern.as_ptr(),
            pattern.len(),
            options,
            &raw mut error_code,
            &raw mut error_offset,
            context,
        );
        pcre2_compile_context_free_8(context);
        code
    };
    NonNull::new(code).map(Code).ok_or_else(|| CompileError {
        message: error_message(error_code),
        offset: error_offset,
    })
}

/// Return the message of a PCRE2 error code, as `build_compile_error`
/// (`erl_bif_re.c:809-818`) makes it: the text of
/// `pcre2_get_error_message` in a 120-byte buffer, or `Unknown error (N)`
/// for a code that PCRE2 does not know.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(error_message(114), b"missing closing parenthesis");
/// assert_eq!(error_message(99999), b"Unknown error (99999)");
/// ```
fn error_message(code: c_int) -> Vec<u8> {
    let mut buffer = [0u8; ERROR_MESSAGE_SIZE];
    // SAFETY: PCRE2 writes at most `buffer.len()` bytes into `buffer`, and
    // the text ends with a NUL byte, also when it is cut
    // (`pcre2_error.c:332-375`).
    let result = unsafe { pcre2_get_error_message_8(code, buffer.as_mut_ptr(), buffer.len()) };
    if result == ERROR_BADDATA {
        return format!("Unknown error ({code})").into_bytes();
    }
    let length = buffer
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(buffer.len());
    buffer[..length].to_vec()
}

/// The compile stack guard: return non-zero when the stack of the calling
/// thread has less than `STACK_MARGIN` bytes above `limit`, as
/// `stack_guard_downwards` (`erl_bif_re.c:160-169`) does. The stack of each
/// supported host grows down. PCRE2 calls it at each nesting level of the
/// pattern.
///
/// # Examples
///
/// ```rust,ignore
/// // SAFETY: the guard reads no memory through its data.
/// assert_eq!(unsafe { stack_guard(0, std::ptr::without_provenance_mut(stack_limit())) }, 0);
/// assert_eq!(unsafe { stack_guard(0, std::ptr::without_provenance_mut(usize::MAX - STACK_MARGIN)) }, 1);
/// ```
unsafe extern "C" fn stack_guard(_depth: u32, limit: *mut c_void) -> c_int {
    let marker = 0u8;
    let address = std::hint::black_box(&raw const marker).addr();
    c_int::from(address < limit.addr().saturating_add(STACK_MARGIN))
}

/// Return the lowest address of the stack of the calling thread, as
/// `erts_get_stacklimit` gives it for a scheduler thread. macOS gives the
/// highest address and the size of the stack.
///
/// # Examples
///
/// ```rust,ignore
/// let marker = 0u8;
/// assert!(stack_limit() < (&raw const marker).addr());
/// ```
#[cfg(target_os = "macos")]
fn stack_limit() -> usize {
    // SAFETY: `pthread_self` is the calling thread, which is alive. The two
    // functions only read the stack fields of its thread struct.
    unsafe {
        let thread = libc::pthread_self();
        libc::pthread_get_stackaddr_np(thread).addr() - libc::pthread_get_stacksize_np(thread)
    }
}

/// Return the lowest address of the stack of the calling thread, as
/// `erts_get_stacklimit` gives it for a scheduler thread.
///
/// # Examples
///
/// ```rust,ignore
/// let marker = 0u8;
/// assert!(stack_limit() < (&raw const marker).addr());
/// ```
#[cfg(target_os = "linux")]
fn stack_limit() -> usize {
    // SAFETY: `pthread_getattr_np` fills `attributes` for the calling
    // thread, which is alive, and `pthread_attr_getstack` reads the lowest
    // stack address and the size from it. `pthread_attr_destroy` then frees
    // what `pthread_getattr_np` allocated.
    unsafe {
        let mut attributes: libc::pthread_attr_t = std::mem::zeroed();
        let result = libc::pthread_getattr_np(libc::pthread_self(), &raw mut attributes);
        assert_eq!(result, 0, "pthread_getattr_np failed");
        let mut address = std::ptr::null_mut();
        let mut size = 0;
        libc::pthread_attr_getstack(&raw const attributes, &raw mut address, &raw mut size);
        libc::pthread_attr_destroy(&raw mut attributes);
        address.addr()
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/pcre2.rs"]
mod tests;
