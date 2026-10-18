//! The compiled patterns of `binary:compile_pattern/1` and their references.
//! ERTS keeps the compiled data of each pattern in a magic binary and returns
//! the tuple `{bm, Ref}` or `{ac, Ref}` (`binary_compile_pattern_1`,
//! `erl_bif_binary.c:1103-1116`). PBeam keeps the source needles in this
//! store, by the reference of the magic binary that ERTS would make.

use std::collections::HashMap;

use crate::term::{Reference, Term};

/// The source needles of a compiled pattern.
#[derive(Debug)]
pub(super) enum CompiledPattern {
    /// The single needle of a `{bm, Ref}` pattern.
    Bm(Vec<u8>),
    /// The needles of an `{ac, Ref}` pattern, in the order of the list.
    Ac(Vec<Vec<u8>>),
}

/// The compiled patterns of the node, by the reference of the magic binary
/// that ERTS makes for each one (`erts_mk_magic_ref`,
/// `erl_bif_binary.c:1112-1114`).
#[derive(Debug, Default)]
pub(super) struct BinaryPatterns {
    /// Each pattern by its reference. ERTS frees a pattern when no term
    /// references it. PBeam keeps each pattern until the run ends: see
    /// "Compiled binary patterns" in `docs/erts-differences.md`.
    patterns: HashMap<Reference, CompiledPattern>,
}

impl BinaryPatterns {
    /// Store `pattern` for the reference `id` and return it, as
    /// `binary_compile_pattern_1` stores the compiled data in the magic
    /// binary before it makes the reference.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut patterns = BinaryPatterns::default();
    /// patterns.insert(Reference(0), CompiledPattern::Bm(b"a".to_vec()));
    /// assert!(patterns.pattern(&Term::Reference(Reference(0))).is_some());
    /// ```
    pub(super) fn insert(&mut self, id: Reference, pattern: CompiledPattern) {
        self.patterns.insert(id, pattern);
    }

    /// Return the pattern of `term`, or `None` when `term` is not the
    /// reference of a compiled pattern.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let patterns = BinaryPatterns::default();
    /// assert!(patterns.pattern(&Term::Reference(Reference(0))).is_none());
    /// ```
    pub(super) fn pattern(&self, term: &Term) -> Option<&CompiledPattern> {
        let Term::Reference(id) = term else {
            return None;
        };
        self.patterns.get(id)
    }
}
