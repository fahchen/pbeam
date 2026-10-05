//! The compiled regular expressions of the node, as the ERTS magic binaries
//! of `re:compile/2` and their magic references (`erl_binary.h`). The
//! scheduler keeps them in its `State`, and all processes of a run share
//! them.

use std::collections::HashMap;

use super::pcre2::Code;
use crate::term::Reference;

/// The compiled code of each `re_pattern` by its reference, as the magic
/// binaries of `build_compile_result` (`erl_bif_re.c:600-608`). ERTS frees a
/// magic binary when no term refers to it. PBeam keeps each code until the
/// end of the run.
#[derive(Debug, Default)]
pub(super) struct RegexCodes(pub(super) HashMap<Reference, Code>);
