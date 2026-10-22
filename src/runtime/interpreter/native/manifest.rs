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
//! (`beam/erl_nif.c:4743-4757`, `5324-5331`). The path must still be a
//! literal string, so that the gate can name the library that the module
//! references.

use super::Linked;
use crate::beam::{BeamModule, ImportKind, code::Instruction, code::Operand};
use crate::term::Term;

/// Return each NIF reference of `module` that PBeam does not bind, as one
/// message for each item, in the order of the code of the module. An empty
/// result passes the gate.
///
/// The items are:
///
/// - `M: the library name of erlang:load_nif/2 is not a literal string`
///   for a call of `erlang:load_nif/2` whose path the gate cannot resolve.
///   The compiler folds `atom_to_list(?MODULE)` to a literal string, so
///   `{move,{literal,"prim_tty"},{x,0}}` directly before the call resolves.
///   An instruction without effect, such as `init_yregs`, can be between
///   them.
/// - `M: NIF library "Path" has no PBeam bindings` for a resolved call in
///   a module without a static NIF library.
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
/// # Examples
///
/// ```rust,ignore
/// // prim_tty: -nifs([isatty/1, isprint/1]), load() -> erlang:load_nif("prim_tty", 0).
/// assert_eq!(nif_manifest_gaps(&prim_tty), ["prim_tty:isprint/1: NIF function has no PBeam binding"]);
/// // nif_gate_dynamic: load(Path) -> erlang:load_nif(Path, 0).
/// assert_eq!(
///     nif_manifest_gaps(&dynamic),
///     ["nif_gate_dynamic: the library name of erlang:load_nif/2 is not a literal string"],
/// );
/// ```
pub fn nif_manifest_gaps(module: &BeamModule) -> Vec<String> {
    let name = &module.name;
    let instructions = &module.instructions;
    let linked = Linked::of(name);
    let mut gaps = Vec::new();
    for (index, instruction) in instructions.iter().enumerate() {
        let Instruction::CallExternal { import, .. } = instruction else {
            continue;
        };
        if module.imports[*import as usize].kind != ImportKind::LoadNif {
            continue;
        }
        match library_path(&instructions[..index]) {
            None => gaps.push(format!(
                "{name}: the library name of erlang:load_nif/2 is not a literal string"
            )),
            Some(path) if linked.is_none() => {
                gaps.push(format!(
                    "{name}: NIF library \"{path}\" has no PBeam bindings"
                ));
            }
            Some(_) => {}
        }
    }
    let Some(linked) = linked else {
        return gaps;
    };
    let declared: Vec<(&str, u32)> = instructions
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::NifStart { name, arity } => Some((name.as_str(), *arity)),
            _ => None,
        })
        .collect();
    for &(function, arity) in linked.functions() {
        if declared.contains(&(function, arity)) && !linked.binds(function, arity) {
            gaps.push(format!(
                "{name}:{function}/{arity}: NIF function has no PBeam binding"
            ));
        }
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
