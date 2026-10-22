//! The compiled regular expressions of the node, as the ERTS magic binaries
//! of `re:compile/2` and their magic references (`erl_binary.h`). Any
//! process of the node uses them, as any ERTS scheduler reads a magic
//! binary.
//!
//! A [`super::pcre2::Code`] owns a pointer that PCRE2 gives, so it stays on
//! the thread that made it. The node keeps the serialized bytes of each
//! expression in [`RegexSources`], and each scheduler decodes them once into
//! its own [`RegexCodes`].

use std::collections::HashMap;
use std::sync::Arc;

use super::pcre2::Code;
use crate::term::Reference;

/// The decoded code of each `re_pattern` that a process of one scheduler
/// used, by its reference: a cache of [`RegexSources`].
#[derive(Debug, Default)]
pub(super) struct RegexCodes(pub(super) HashMap<Reference, Code>);

/// The serialized code of each `re_pattern` of the node by its reference,
/// as the magic binaries of `build_compile_result` (`erl_bif_re.c:600-608`).
/// The bytes are the `pcre2_serialize_encode` form of `re:compile/2` with
/// `export` (`erl_bif_re.c:663-665`). ERTS frees a magic binary when no term
/// refers to it. PBeam keeps each code until the end of the run.
#[derive(Debug, Default)]
pub(super) struct RegexSources(pub(super) HashMap<Reference, Arc<[u8]>>);
