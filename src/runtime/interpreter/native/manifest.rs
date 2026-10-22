//! The NIF manifest gate: a build-time check of the NIF references of a
//! BEAM module against the static NIF libraries of PBeam and the NIF shared
//! libraries that a bundle ships.
//!
//! A bundle build runs the gate on each module before a run, so a module
//! whose NIF library PBeam cannot load fails the build and not the run. A
//! function of a bound library that has no binding is a report item: a call
//! of it gives a clear result at run time, so it does not fail the build.
//! The runtime does not call the gate.
//!
//! The gate keys a library by the module that calls `erlang:load_nif/2`, as
//! the runtime does (`load_nif` in the parent module). ERTS selects a static
//! NIF library by the module of the caller and does not use the path for it
//! (`beam/erl_nif.c:4743-4757`, `5324-5331`), so a module with a library
//! needs no literal path. A module without a library passes when a shipped
//! NIF shared library has the name of the module and `erlang:load_nif/2`
//! takes it. Else the module must have a literal path, so that the gate can
//! name the library that the module references.

use std::ffi::CString;
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::{Linked, has_nifs, library_error, open_failure};
use crate::beam::{BeamModule, ImportKind, code::Instruction, code::Operand};
use crate::runtime::nif::abi;
use crate::term::Term;

/// One NIF reference of a module that PBeam does not bind. The text of an
/// item is its message. `DynamicPath`, `NoBindings`, and `Shipped` are
/// build errors: PBeam has no library for the module, or the shipped
/// library does not load. `Unbound` is a report item.
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
    /// `M: NIF library "File" does not load: Reason`, for a shipped NIF
    /// shared library that `erlang:load_nif/2` of the module does not take.
    Shipped {
        /// The module that calls `erlang:load_nif/2`.
        module: String,
        /// The path of the `.so` file.
        library: String,
        /// The `erlang:load_nif/2` error text, or the PBeam limit.
        reason: String,
    },
    /// `M:F/A: NIF function has no PBeam binding; a call ...`, with the
    /// result of a call of the function.
    Unbound {
        /// The module of the library.
        library: String,
        /// The name of the function.
        function: String,
        /// The arity of the function.
        arity: u32,
        /// `true` when a call gives the runtime error
        /// `M:F/A NIF is not supported`, `false` when a call runs the BEAM
        /// stub body of the function.
        call_fails: bool,
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
            NifGap::Shipped {
                module,
                library,
                reason,
            } => write!(
                formatter,
                "{module}: NIF library \"{library}\" does not load: {reason}"
            ),
            NifGap::Unbound {
                library,
                function,
                arity,
                call_fails: true,
            } => write!(
                formatter,
                "{library}:{function}/{arity}: NIF function has no PBeam binding; a call gives the runtime error \"{library}:{function}/{arity} NIF is not supported\""
            ),
            NifGap::Unbound {
                library,
                function,
                arity,
                call_fails: false,
            } => write!(
                formatter,
                "{library}:{function}/{arity}: NIF function has no PBeam binding; a call runs the BEAM stub body"
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
/// item, in the order of the code of the module. `libraries` are the NIF
/// shared libraries that the bundle ships for the module, as paths of
/// `.so` files. A result with only `Unbound` items passes the gate.
///
/// A module without a NIF library that calls `erlang:load_nif/2` has no
/// item when a library of `libraries` takes the load. The gate opens each
/// library in order with the loader of the runtime, which runs the
/// constructors and the init function of the library on the build host,
/// until one has the name of the module, as `erlang:load_nif/2` requires
/// (`beam/erl_nif.c:4820-4823`). That library takes the load when it has
/// the vm variant `beam.vanilla` and passes the function checks of
/// `erlang:load_nif/2` (`beam/erl_nif.c:4814-4819`, `4845-4900`), and the
/// module has `nif_start`, which PBeam requires. The gate does not resolve
/// the path of the call, which the run computes, such as
/// `code:priv_dir/1`. Else the items are:
///
/// - `M: NIF library "File" does not load: Reason` for the library with
///   the name of the module that does not take the load, as the only item.
/// - `M: the library name of erlang:load_nif/2 is not a literal string`
///   for a call of `erlang:load_nif/2` whose path the gate cannot resolve,
///   in a module without a NIF library and without a shipped library. The
///   compiler folds `atom_to_list(?MODULE)` to a literal string, so
///   `{move,{literal,"prim_tty"},{x,0}}` directly before the call resolves.
///   An instruction without effect, such as `init_yregs`, can be between
///   them.
/// - `M: NIF library "Path" has no PBeam bindings` for a resolved call in
///   a module without a NIF library and without a shipped library.
/// - After them, `M: NIF library "File" does not load: Reason` for each
///   library of `libraries` that the loader cannot open, with the
///   `erlang:load_nif/2` error text, because it can be the library of the
///   module.
/// - `M:F/A: NIF function has no PBeam binding; a call ...` for each
///   function of the library of `M` that the module declares as a NIF and
///   that has no binding, with the result of a call: the runtime error of a
///   library with `unbound_fails`, or the BEAM stub body. The functions go in the order of the library, as
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
/// assert_eq!(
///     nif_manifest_gaps(&prim_tty, &[])[0].to_string(),
///     "prim_tty:isprint/1: NIF function has no PBeam binding; a call runs the BEAM stub body",
/// );
/// // nif_gate_dynamic: load(Path) -> erlang:load_nif(Path, 0).
/// assert_eq!(
///     nif_manifest_gaps(&dynamic, &[]),
///     [NifGap::DynamicPath { module: "nif_gate_dynamic".to_owned() }],
/// );
/// // nif_add: -nifs([add/2, other/0]); priv/nif_add.so defines add/2.
/// assert!(nif_manifest_gaps(&nif_add, &["priv/nif_add.so".into()]).is_empty());
/// ```
pub fn nif_manifest_gaps(module: &BeamModule, libraries: &[PathBuf]) -> Vec<NifGap> {
    let name = &module.name;
    let Some(linked) = Linked::of(name) else {
        if !cfg!(windows) && WINDOWS_ONLY_LOADS.contains(&name.as_str()) {
            return Vec::new();
        }
        return load_gaps(module, libraries);
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
            call_fails: linked.unbound_fails(),
        })
        .collect()
}

/// Return the items of the calls of `erlang:load_nif/2` in `module`, a
/// module without a NIF library, with the shipped `libraries`, as
/// [`nif_manifest_gaps`] gives them.
///
/// # Examples
///
/// ```rust,ignore
/// // nif_gate_library: load() -> erlang:load_nif("priv/nif_gate_library", 0).
/// assert_eq!(load_gaps(&library, &[])[0].to_string(), "nif_gate_library: NIF library \"priv/nif_gate_library\" has no PBeam bindings");
/// ```
fn load_gaps(module: &BeamModule, libraries: &[PathBuf]) -> Vec<NifGap> {
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
    if gaps.is_empty() {
        return gaps;
    }
    let caller = &module.name;
    for path in libraries {
        let shipped = |reason| NifGap::Shipped {
            module: caller.clone(),
            library: path.display().to_string(),
            reason,
        };
        let name = load_name(path);
        let library = match abi::open(&name) {
            Ok(library) => library,
            Err(failure) => {
                gaps.push(shipped(open_failure(caller, &name, failure).1));
                continue;
            }
        };
        if library.name() != caller {
            continue;
        }
        let reason = library_error(module, caller, &library).or_else(|| {
            (!has_nifs(module)).then(|| {
                "the module has no nif_start instruction, which PBeam needs for a NIF shared library"
                    .to_owned()
            })
        });
        return reason.map(shipped).into_iter().collect();
    }
    gaps
}

/// Return the path that `erlang:load_nif/2` gets for the `.so` file
/// `path`: the path without its `.so` extension, because the loader adds
/// it (`sys/unix/erl_unix_sys_ddll.c:107-158`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(load_name(Path::new("lib/a/priv/a_nif.so")).as_bytes(), b"lib/a/priv/a_nif");
/// ```
fn load_name(path: &Path) -> CString {
    let path = path.with_extension("");
    // A path of the file system has no NUL byte.
    CString::new(path.as_os_str().as_bytes()).expect("a path has no NUL byte")
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
