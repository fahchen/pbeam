//! The NIF manifest gate: a build-time check of the NIF references of a
//! BEAM module against the static NIF libraries of PBeam.
//!
//! A bundle build runs the gate on each module before a run, so a module
//! that references a NIF library or a NIF function that PBeam does not bind
//! fails the build and not the run. The runtime does not call the gate.
//!
//! The gate keys a library by the module that calls `erlang:load_nif/2`, as
//! the runtime does (`load_nif` in the parent module). ERTS selects a static
//! NIF library by the module of the caller and does not use the path for it
//! (`beam/erl_nif.c:4743-4757`, `5324-5331`), so a module with a library
//! needs no literal path. A module without a library must have a literal
//! path, so that the gate can name the library that the module references.

use std::fmt;

use super::Linked;
use crate::beam::{BeamModule, ImportKind, code::Instruction, code::Operand};
use crate::term::Term;

/// One NIF reference of a module that PBeam does not bind. The text of an
/// item is its message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NifGap {
    /// `M: the library name of erlang:load_nif/2 is not a literal string`.
    DynamicPath {
        /// The module that calls `erlang:load_nif/2`.
        module: String,
    },
    /// `M: NIF library "Path" has no PBeam bindings`.
    NoBindings {
        /// The module that calls `erlang:load_nif/2`.
        module: String,
        /// The literal path of the call.
        path: String,
    },
    /// `M:F/A: NIF function has no PBeam binding`.
    Unbound {
        /// The module of the library.
        library: String,
        /// The name of the function.
        function: String,
        /// The arity of the function.
        arity: u32,
    },
}

impl fmt::Display for NifGap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NifGap::DynamicPath { module } => write!(
                formatter,
                "{module}: the library name of erlang:load_nif/2 is not a literal string"
            ),
            NifGap::NoBindings { module, path } => write!(
                formatter,
                "{module}: NIF library \"{path}\" has no PBeam bindings"
            ),
            NifGap::Unbound {
                library,
                function,
                arity,
            } => write!(
                formatter,
                "{library}:{function}/{arity}: NIF function has no PBeam binding"
            ),
        }
    }
}

/// The modules that call `erlang:load_nif/2` only on Windows, which PBeam
/// does not support. The `on_load` of `pubkey_os_cacerts` calls `load_nif/0`
/// only when `os:type()` is `{win32, _}`, and gives `ok` on another host
/// (`lib/public_key/src/pubkey_os_cacerts.erl:265-275`).
const WINDOWS_ONLY_LOADS: &[&str] = &["pubkey_os_cacerts"];

/// Return each NIF reference of `module` that PBeam does not bind, as one
/// item, in the order of the code of the module. An empty result passes the
/// gate.
///
/// The items are:
///
/// - `M: the library name of erlang:load_nif/2 is not a literal string`
///   for a call of `erlang:load_nif/2` whose path the gate cannot resolve,
///   in a module without a NIF library. The compiler folds
///   `atom_to_list(?MODULE)` to a literal string, so
///   `{move,{literal,"prim_tty"},{x,0}}` directly before the call resolves.
///   An instruction without effect, such as `init_yregs`, can be between
///   them.
/// - `M: NIF library "Path" has no PBeam bindings` for a resolved call in
///   a module without a NIF library.
/// - `M:F/A: NIF function has no PBeam binding` for each function of the
///   library of `M` that the module declares as a NIF and that has no
///   binding. The functions go in the order of the library, as
///   `erlang:load_nif/2` checks them (`beam/erl_nif.c:4845-4866`). The
///   declared functions are the functions that start with `nif_start`, as
///   the ERTS loader keeps them (`jit/asm_load.c:651-661`), and a binding
///   of PBeam runs only at `nif_start`. The compiler of the pinned OTP puts
///   `nif_start` on the functions of `-nifs`, or on each function of a
///   module without `-nifs` that calls `erlang:load_nif/2`. A declared
///   function that is not in the library is not an item: it runs its BEAM
///   body on ERTS too.
///
/// A call of `erlang:load_nif/2` in a module with a NIF library is not an
/// item, whatever its path: the module selects the library. Thus `crypto`,
/// which builds its path at run time (`crypto.erl:4479`, `4499`), passes. A
/// module of `WINDOWS_ONLY_LOADS` has no item on a host that is not Windows,
/// because its call does not run there.
///
/// # Examples
///
/// ```rust,ignore
/// // prim_tty: -nifs([isatty/1, isprint/1]), load() -> erlang:load_nif("prim_tty", 0).
/// assert_eq!(nif_manifest_gaps(&prim_tty)[0].to_string(), "prim_tty:isprint/1: NIF function has no PBeam binding");
/// // nif_gate_dynamic: load(Path) -> erlang:load_nif(Path, 0).
/// assert_eq!(
///     nif_manifest_gaps(&dynamic),
///     [NifGap::DynamicPath { module: "nif_gate_dynamic".to_owned() }],
/// );
/// ```
pub fn nif_manifest_gaps(module: &BeamModule) -> Vec<NifGap> {
    let name = &module.name;
    let Some(linked) = Linked::of(name) else {
        if !cfg!(windows) && WINDOWS_ONLY_LOADS.contains(&name.as_str()) {
            return Vec::new();
        }
        return load_gaps(module);
    };
    let declared: Vec<(&str, u32)> = module
        .instructions
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::NifStart { name, arity } => Some((name.as_str(), *arity)),
            _ => None,
        })
        .collect();
    linked
        .functions()
        .iter()
        .filter(|&&(function, arity)| {
            declared.contains(&(function, arity)) && !linked.binds(function, arity)
        })
        .map(|&(function, arity)| NifGap::Unbound {
            library: name.clone(),
            function: function.to_owned(),
            arity,
        })
        .collect()
}

/// Return one item for each call of `erlang:load_nif/2` in `module`, a
/// module without a NIF library: the literal path of the call, or the
/// dynamic path.
///
/// # Examples
///
/// ```rust,ignore
/// // nif_gate_library: load() -> erlang:load_nif("priv/nif_gate_library", 0).
/// assert_eq!(load_gaps(&library)[0].to_string(), "nif_gate_library: NIF library \"priv/nif_gate_library\" has no PBeam bindings");
/// ```
fn load_gaps(module: &BeamModule) -> Vec<NifGap> {
    let instructions = &module.instructions;
    let mut gaps = Vec::new();
    for (index, instruction) in instructions.iter().enumerate() {
        let Instruction::CallExternal { import, .. } = instruction else {
            continue;
        };
        if module.imports[*import as usize].kind != ImportKind::LoadNif {
            continue;
        }
        let module = module.name.clone();
        gaps.push(match library_path(&instructions[..index]) {
            None => NifGap::DynamicPath { module },
            Some(path) => NifGap::NoBindings { module, path },
        });
    }
    gaps
}

/// Return the path that the code before a call of `erlang:load_nif/2` puts
/// in `x0`, or `None` when the last instruction with an effect is not a
/// move of a literal string to `x0`.
///
/// # Examples
///
/// ```rust,ignore
/// let path = Term::list_from("zlib".bytes().map(|byte| Term::Integer(byte.into())));
/// let code = [Instruction::Move(Operand::Value(path), Operand::X(0)), Instruction::Noop];
/// assert_eq!(library_path(&code).as_deref(), Some("zlib"));
/// assert_eq!(library_path(&[Instruction::Move(Operand::Y(0), Operand::X(0))]), None);
/// ```
fn library_path(code: &[Instruction]) -> Option<String> {
    let last = code
        .iter()
        .rev()
        .find(|instruction| !matches!(instruction, Instruction::Noop));
    let Some(Instruction::Move(Operand::Value(path), Operand::X(0))) = last else {
        return None;
    };
    let (characters, proper) = path.list_elements()?;
    let text = characters.iter().map(|character| match character {
        Term::Integer(code) => char::from_u32(u32::try_from(*code).ok()?),
        _ => None,
    });
    proper.then(|| text.collect()).flatten()
}
