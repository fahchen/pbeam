//! The load-check table of the `public_key` NIF library
//! (`lib/public_key/c_src/public_key.c`), which ERTS registers under the
//! module `pubkey_os_cacerts` (`public_key.c:69`). The pinned module loads
//! it only on Windows (`pubkey_os_cacerts.erl:265-275`).
//!
//! PBeam has no binding of a `public_key` NIF. A call of the function of
//! the table stops the run with the runtime error of the static library
//! (`Library::unbound_fails`). The load callback of the library returns 0
//! for each load info (`public_key.c:44-52`).

/// The functions of the `public_key` NIF library, as `(name, arity)` in
/// the order of `nif_funcs` (`public_key.c:64-67`). Its flags are 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[0], ("os_cacerts", 0));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 1] = [("os_cacerts", 0)];
