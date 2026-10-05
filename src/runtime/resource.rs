//! NIF resources of the node: native objects that a term refers to through
//! a reference, as ERTS magic binaries and their magic references
//! (`erl_binary.h`). All processes of a run share one store.

use std::collections::HashMap;

use super::pcre2::Code;
use crate::term::Reference;

/// The resources of the node. ERTS frees a magic binary when no term refers
/// to it. PBeam keeps each resource until the end of the run.
#[derive(Debug, Default)]
pub(super) struct Resources {
    /// The compiled code of each `re_pattern` by its reference, as the
    /// magic binaries of `build_compile_result` (`erl_bif_re.c:600-608`).
    regex_codes: HashMap<Reference, Code>,
    /// Each TTY resource by its reference, as the `TTYResource` objects of
    /// `tty_create_nif` (`prim_tty_nif.c:988-1072`).
    ttys: HashMap<Reference, TtyResource>,
}

/// One terminal of `prim_tty:tty_create/1`, as the ERTS `TTYResource`
/// (`prim_tty_nif.c:98-118`) on Unix. Its input fd is always 0
/// (`prim_tty_nif.c:1001`).
#[derive(Debug)]
pub(super) struct TtyResource {
    /// The output fd: 1 for `stdout` or 2 for `stderr`.
    pub(super) ofd: libc::c_int,
}

impl Resources {
    /// Keep the compiled code `code` under the reference `reference`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut resources = Resources::default();
    /// resources.insert_regex_code(Reference(0), pcre2::compile(b"a", 0, None, None)?);
    /// ```
    pub(super) fn insert_regex_code(&mut self, reference: Reference, code: Code) {
        self.regex_codes.insert(reference, code);
    }

    /// Keep the TTY resource `tty` under the reference `reference`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut resources = Resources::default();
    /// resources.insert_tty(Reference(0), TtyResource { ofd: libc::STDOUT_FILENO });
    /// ```
    pub(super) fn insert_tty(&mut self, reference: Reference, tty: TtyResource) {
        self.ttys.insert(reference, tty);
    }

    /// Return the TTY resource of `reference`, as `enif_get_resource` with
    /// the TTY resource type does. A reference of another resource type or
    /// of `make_ref/0` gives `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// resources.insert_tty(Reference(0), TtyResource { ofd: libc::STDOUT_FILENO });
    /// assert_eq!(resources.tty(Reference(0)).map(|tty| tty.ofd), Some(1));
    /// assert!(resources.tty(Reference(1)).is_none());
    /// ```
    pub(super) fn tty(&self, reference: Reference) -> Option<&TtyResource> {
        self.ttys.get(&reference)
    }
}
