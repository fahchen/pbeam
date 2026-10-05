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
/// `PCRE2_INFO_CAPTURECOUNT` (`pcre2.h:453`): the highest capture group
/// number, as a `uint32_t`.
const INFO_CAPTURECOUNT: u32 = 4;
/// `PCRE2_INFO_NEWLINE` (`pcre2.h:469`): the newline convention of the
/// compiled code, as a `uint32_t` `PCRE2_NEWLINE_*` value.
const INFO_NEWLINE: u32 = 20;

/// The size of the error message buffer of `build_compile_error`
/// (`erl_bif_re.c:809`). A longer message is cut to 119 bytes.
const ERROR_MESSAGE_SIZE: usize = 120;

/// The stack that the compile stack guard keeps free below the stack limit,
/// as `ERTS_PCRE_STACK_MARGIN` (`erl_bif_re.c:157`).
const STACK_MARGIN: usize = 10 * 1024;

// The PCRE2 functions that the compile path uses (`pcre2.h:650-751` and
// `pcre2.h:878`). `pcre2_code_8` and `pcre2_compile_context_8` are opaque C
// structs, so a `c_void` pointer stands for each.
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
    fn pcre2_code_free_8(code: *mut c_void);
}

/// One compiled regular expression: a `pcre2_code` that this value owns
/// and frees when it drops, as `regex_code_destructor`
/// (`erl_bif_re.c:117-125`) does.
#[derive(Debug)]
pub(super) struct Code(NonNull<c_void>);

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
mod tests {
    use super::*;

    #[test]
    fn gives_unknown_error_for_a_code_that_pcre2_does_not_know() {
        // ERTS: "Unknown error (%d)" for PCRE2_ERROR_BADDATA
        // (erl_bif_re.c:812-815).
        assert_eq!(error_message(99999), b"Unknown error (99999)");
        assert_eq!(error_message(114), b"missing closing parenthesis");
    }

    #[test]
    fn stops_at_the_stack_guard_on_a_small_stack() {
        // On a 64 KiB stack, 100 nested lookaheads reach the 10 KiB margin
        // of the guard (erl_bif_re.c:157-169). PCRE2 then returns error 133
        // at offset 0, and the thread does not crash.
        let pattern = [b"(?=".repeat(100), b")".repeat(100)].concat();
        let result = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || compile(&pattern, 0, None, None).map(|code| code.capture_count()))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(
            result,
            Err(CompileError {
                message: b"parentheses are too deeply nested (stack check)".to_vec(),
                offset: 0,
            })
        );
    }
}
