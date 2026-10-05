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
}
