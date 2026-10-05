//! Native bindings: the NIFs that PBeam links statically, separate from the
//! BIF table.
//!
//! ERTS loads a NIF library with `erlang:load_nif/2`, which writes the call
//! of each NIF over the first instruction of its function
//! (`beam/erl_nif.c:4922-4928`, `5050-5098`). A module of a static NIF
//! library, such as `prim_tty`, uses the library that ERTS links, and
//! `erlang:load_nif/2` opens no file (`beam/erl_nif.c:4751-4757`,
//! `5324-5331`). PBeam models each module with bindings as such a static
//! library: `load_nif` runs the checks of `erlang:load_nif/2` against the
//! function list of the library. PBeam opens no library file. A binding in
//! this registry is active when its module loads, also before
//! `erlang:load_nif/2`, and the `nif_start` instruction at the entry of the
//! function runs it. Each OTP module has one submodule that registers its
//! bindings in `BINDINGS`, and `LIBRARIES` registers each such list under
//! its module.

use super::*;
use crate::runtime::{host::filename_encoding, unicode::is_native_filename};

mod prim_tty;

/// The native binding of one NIF of a module. ERTS finds a NIF of a library
/// by its exact name and arity (`beam/erl_nif.c:4845-4866`).
///
/// # Examples
///
/// ```rust,ignore
/// const WRITE_NIF: Binding = Binding {
///     name: "write_nif",
///     arity: 2,
///     run: |scheduler, arguments| write_nif(&scheduler.resources, &arguments[0], &arguments[1]),
/// };
/// ```
struct Binding {
    /// The name of the NIF function.
    name: &'static str,
    /// The arity of the NIF function.
    arity: u32,
    /// Runs the NIF for the current process of the scheduler. The lookup
    /// gives exactly `arity` arguments.
    run: fn(&mut Scheduler, &[Term]) -> Result<BifResult, BeamError>,
}

/// The static NIF library of one module. ERTS finds a static library by the
/// module name (`is_static_nif_module` in `beam/erl_nif.c:5324-5331`).
///
/// # Examples
///
/// ```rust,ignore
/// const PRIM_TTY: Library = Library {
///     module: "prim_tty",
///     functions: &prim_tty::FUNCTIONS,
///     bindings: prim_tty::BINDINGS,
/// };
/// ```
struct Library {
    /// The name of the module of the library.
    module: &'static str,
    /// The functions of the library, as `(name, arity)` in the order of its
    /// `nif_funcs`. `erlang:load_nif/2` checks them in this order.
    functions: &'static [(&'static str, u32)],
    /// The bindings of the functions that PBeam implements, sorted by name,
    /// then by arity.
    bindings: &'static [Binding],
}

/// The registered static NIF libraries, sorted by the module name for a
/// binary search.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(LIBRARIES[0].module, "prim_tty");
/// ```
const LIBRARIES: &[Library] = &[Library {
    module: "prim_tty",
    functions: &prim_tty::FUNCTIONS,
    bindings: prim_tty::BINDINGS,
}];

/// Return the registered static NIF library of `module`, or `None` when
/// PBeam has no bindings for the module.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(library("prim_tty").map(|library| library.functions[0]), Some(("isatty", 1)));
/// assert!(library("prim_file").is_none());
/// ```
fn library(module: &str) -> Option<&'static Library> {
    let index = LIBRARIES.binary_search_by_key(&module, |library| library.module);
    Some(&LIBRARIES[index.ok()?])
}

/// Return the registered binding of exactly `module:name/arity`, or `None`
/// when PBeam has no binding for the MFA.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(binding("prim_tty", "write_nif", 2).is_some());
/// assert!(binding("prim_tty", "write_nif", 1).is_none());
/// assert!(binding("nif_error_probe", "nif_stub", 0).is_none());
/// ```
fn binding(module: &str, name: &str, arity: u32) -> Option<&'static Binding> {
    let bindings = library(module)?.bindings;
    let index =
        bindings.binary_search_by_key(&(name, arity), |binding| (binding.name, binding.arity));
    Some(&bindings[index.ok()?])
}

/// Run the native binding of `module:name/arity`, with the arity of
/// `arguments`, for the current process of `scheduler`. Return `None` when
/// PBeam has no binding for the MFA; the function then runs its BEAM stub
/// body.
///
/// # Examples
///
/// ```rust,ignore
/// let stdout = Term::Atom(scheduler.atoms.put("stdout")?);
/// // `false` when stdout is a file.
/// assert!(matches!(call(&mut scheduler, "prim_tty", "isatty", &[stdout]), Some(Ok(BifResult::Value(_)))));
/// assert!(call(&mut scheduler, "nif_error_probe", "nif_stub", &[]).is_none());
/// ```
///
/// # Errors
///
/// A binding gives a runtime error for a part that PBeam does not implement
/// yet, or when a new result atom does not fit in the atom table.
pub(super) fn call(
    scheduler: &mut Scheduler,
    module: &str,
    name: &str,
    arguments: &[Term],
) -> Option<Result<BifResult, BeamError>> {
    let binding = binding(module, name, u32::try_from(arguments.len()).ok()?)?;
    Some((binding.run)(scheduler, arguments))
}

/// Run `erlang:load_nif(Path, LoadInfo)` from code of the module `caller`,
/// with the arguments in `x0` and `x1`, as the ERTS `i_load_nif`
/// instruction does with `erts_load_nif` (`beam/erl_nif.c:4708-5039`). The
/// steps are in the ERTS order:
///
/// 1. A `Path` that ERTS does not accept as a file name raises
///    `error:badarg` (`beam/erl_nif.c:4730-4741`,
///    `beam_jit_common.cpp:679-704`). `is_native_filename` gives the check
///    with the file name encoding of the host.
/// 2. A module without PBeam bindings gives a runtime error. ERTS opens the
///    library with `dlopen` (`beam/erl_nif.c:4778-4787`).
/// 3. When the library of the module is loaded, the result is
///    `{error, {reload, Text}}` (`beam/erl_nif.c:4774-4777`).
/// 4. In the order of the library functions, a function that the module
///    does not define gives `{error, {bad_lib, "Function not found
///    M:F/A"}}`. When the module has a `-nifs` list, a function that is not
///    in it gives `{error, {bad_lib, "Function not declared as nif
///    M:F/A"}}` (`beam/erl_nif.c:4845-4866`). The loader keeps the `-nifs`
///    list only when a function of the module starts with `nif_start`
///    (`jit/asm_load.c:651-661`, `beam/erl_nif.c:4700-4704`).
/// 5. Else the library is loaded, and the result is `ok`. The load callback
///    of `prim_tty` always returns 0 (`prim_tty_nif.c:1274-1279`).
///
/// An error does not load the library, so the next call gives the same
/// error. `LoadInfo` goes only to the load callback, which `prim_tty` does
/// not read.
///
/// # Examples
///
/// ```rust,ignore
/// // x0 = "prim_tty", from prim_tty: the first call gives `ok`, the next
/// // gives the `reload` error.
/// assert!(matches!(load_nif(program, &mut scheduler, "prim_tty")?, BifResult::Value(_)));
/// // x0 = foo raises `error:badarg`.
/// assert!(matches!(load_nif(program, &mut scheduler, "prim_tty")?, BifResult::Raise { .. }));
/// // x0 = "load_nif_probe", from load_nif_probe.
/// assert!(load_nif(program, &mut scheduler, "load_nif_probe").is_err());
/// ```
///
/// # Errors
///
/// A module without PBeam bindings gives a runtime error that names the
/// module. A new error atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn load_nif(
    program: &BeamProgram,
    scheduler: &mut Scheduler,
    caller: &str,
) -> Result<BifResult, BeamError> {
    let arguments = call_arguments(&scheduler.current_process_mut().registers, 2)?;
    if !is_native_filename(&arguments[0], filename_encoding()) {
        return Ok(BifResult::badarg());
    }
    let Some(library) = library(caller) else {
        // ponytail: PBeam opens no NIF library; see "NIF stubs" in docs/erts-differences.md.
        return Err(error(format!(
            "erlang:load_nif/2 from {caller} is not supported: PBeam has no native bindings for the module"
        )));
    };
    if scheduler.nif_libraries.contains(caller) {
        return load_nif_error(
            &mut scheduler.atoms,
            "reload",
            "NIF library already loaded (reload disallowed since OTP 20).",
        );
    }
    let instructions = &program.module(caller)?.instructions;
    let has_nifs = instructions
        .iter()
        .any(|instruction| matches!(instruction, Instruction::NifStart { .. }));
    for &(name, arity) in library.functions {
        let defined = instructions.iter().any(|instruction| {
            matches!(instruction, Instruction::FunctionInfo { name: found, arity: found_arity, .. }
                if found == name && *found_arity == arity)
        });
        if !defined {
            let text = format!("Function not found {caller}:{name}/{arity}");
            return load_nif_error(&mut scheduler.atoms, "bad_lib", &text);
        }
        let declared = instructions.iter().any(|instruction| {
            matches!(instruction, Instruction::NifStart { name: found, arity: found_arity }
                if found == name && *found_arity == arity)
        });
        if has_nifs && !declared {
            let text = format!("Function not declared as nif {caller}:{name}/{arity}");
            return load_nif_error(&mut scheduler.atoms, "bad_lib", &text);
        }
    }
    scheduler.nif_libraries.insert(caller.to_owned());
    Ok(BifResult::Value(Term::static_atom("ok")))
}

/// Return `{error, {Kind, Text}}` with `Text` as a list of its bytes, as
/// ERTS `load_nif_error` (`beam/erl_nif.c:4530-4558`) does. ERTS makes the
/// atom `Kind` with `mkatom`, so it goes into the atom table of the run.
/// `load_nif` and the BIF `erlang:load_nif/2` use it.
///
/// # Examples
///
/// ```rust,ignore
/// // {error, {reload, "NIF library already loaded (reload disallowed since OTP 20)."}}
/// let result = load_nif_error(&mut atoms, "reload", "NIF library already loaded (reload disallowed since OTP 20).")?;
/// assert!(matches!(result, BifResult::Value(Term::Tuple(_))));
/// ```
///
/// # Errors
///
/// The atom `Kind` in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn load_nif_error(
    atoms: &mut AtomTable,
    kind: &str,
    text: &str,
) -> Result<BifResult, BeamError> {
    let text = text
        .bytes()
        .map(|byte| Term::Integer(byte.into()))
        .collect();
    Ok(BifResult::Value(Term::Tuple(vec![
        Term::static_atom("error"),
        Term::Tuple(vec![Term::Atom(atoms.put(kind)?), Term::List(text)]),
    ])))
}

#[cfg(test)]
mod tests {
    use super::{LIBRARIES, binding};

    /// `library` and `binding` find an entry with binary searches, so the
    /// libraries and the bindings of each library are strictly increasing:
    /// sorted, with no duplicate entry. Each binding is a function of its
    /// library. The lookup matches the exact MFA.
    #[test]
    fn registry_is_sorted_and_matches_the_exact_mfa() {
        assert!(
            LIBRARIES
                .windows(2)
                .all(|pair| pair[0].module < pair[1].module)
        );
        for library in LIBRARIES {
            let bindings = library.bindings;
            assert!(
                bindings
                    .windows(2)
                    .all(|pair| (pair[0].name, pair[0].arity) < (pair[1].name, pair[1].arity))
            );
            assert!(
                bindings
                    .iter()
                    .all(|binding| library.functions.contains(&(binding.name, binding.arity)))
            );
        }
        assert!(binding("prim_tty", "write_nif", 2).is_some());
        assert!(binding("prim_tty", "write_nif", 1).is_none());
        assert!(binding("prim_tty", "write", 2).is_none());
        assert!(binding("prim_file", "write_nif", 2).is_none());
    }
}
