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
//! function runs it. Each OTP module has one submodule that holds the
//! function table of its library and registers its bindings in
//! `BINDINGS`, and `LIBRARIES` registers each such list under its module.
//! A library without bindings, such as `zstd`, has only the function table:
//! `erlang:load_nif/2` checks it, and each call of a function in it gives
//! the runtime error of `Library::unbound_fails`.
//!
//! An extension crate registers a library of the public type
//! [`crate::runtime::Library`] before the run (see
//! [`crate::runtime::register_library`]). A module has at most one
//! library, so the dispatch, `load_nif`, and the manifest gate find the
//! library of a module as a [`Linked`]: a library of this file, or else an
//! extension library.

use super::bif::denial;
use super::*;
use crate::runtime::{
    Job, NifContext,
    capability::{CapabilityKind, Operation},
    dirty::Work,
    extension,
    host::filename_encoding,
    unicode::is_native_filename,
};

mod asn1rt_nif;
mod crypto;
mod erl_tracer;
mod manifest;
mod prim_buffer;
mod prim_file;
mod prim_net;
mod prim_socket;
mod prim_tty;
mod zlib;
mod zstd;

pub use manifest::nif_manifest_gaps;
pub(super) use prim_tty::dirty_write_result;

/// The native binding of one NIF of a module. ERTS finds a NIF of a library
/// by its exact name and arity (`beam/erl_nif.c:4845-4866`).
///
/// # Examples
///
/// ```rust,ignore
/// const WRITE_NIF: Binding = Binding {
///     name: "write_nif",
///     arity: 2,
///     job: Job::IoBound,
///     run: |scheduler, arguments| write_nif(scheduler, &arguments[0], &arguments[1]),
/// };
/// ```
struct Binding {
    /// The name of the NIF function.
    name: &'static str,
    /// The arity of the NIF function.
    arity: u32,
    /// The scheduler of the NIF, as the `flags` of its ERTS `ErlNifFunc`.
    /// A `CpuBound` or `IoBound` binding checks its arguments on the
    /// scheduler thread and returns `BifResult::Dirty` with the work of the
    /// call, which runs on the dirty pool of the job.
    job: Job,
    /// Runs the NIF for the current process of the scheduler. The lookup
    /// gives exactly `arity` arguments.
    run: fn(&mut Scheduler, &[Term]) -> Result<BifResult, BeamError>,
}

/// The load callback of a static NIF library: it takes the load info and
/// the atom table of the run, and returns the veto of the callback.
type LoadCallback = fn(&Term, &AtomTable) -> i32;

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
///     load: None,
///     unbound_fails: false,
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
    /// The load callback of the library, which ERTS runs with the load info
    /// on the first load (`beam/erl_nif.c:4980-4986`). It returns the veto
    /// of the callback: 0 loads the library. `None` is a library whose load
    /// callback returns 0 for each load info.
    load: Option<LoadCallback>,
    /// `true` when a function of `functions` without a binding gives a
    /// runtime error, `false` when it runs its BEAM stub body.
    unbound_fails: bool,
}

/// The registered static NIF libraries, sorted by the module name for a
/// binary search.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(LIBRARIES[0].module, "asn1rt_nif");
/// assert_eq!(LIBRARIES[1].module, "crypto");
/// assert_eq!(LIBRARIES[2].module, "erl_tracer");
/// ```
const LIBRARIES: &[Library] = &[
    Library {
        module: "asn1rt_nif",
        functions: &asn1rt_nif::FUNCTIONS,
        bindings: &[],
        load: Some(asn1rt_nif::load),
        unbound_fails: true,
    },
    Library {
        module: "crypto",
        functions: &crypto::FUNCTIONS,
        bindings: crypto::BINDINGS,
        load: Some(crypto::load),
        unbound_fails: true,
    },
    Library {
        module: "erl_tracer",
        functions: &erl_tracer::FUNCTIONS,
        bindings: &[],
        load: None,
        unbound_fails: true,
    },
    Library {
        module: "prim_buffer",
        functions: &prim_buffer::FUNCTIONS,
        bindings: prim_buffer::BINDINGS,
        load: None,
        unbound_fails: false,
    },
    Library {
        module: "prim_file",
        functions: &prim_file::FUNCTIONS,
        bindings: prim_file::BINDINGS,
        load: None,
        unbound_fails: false,
    },
    Library {
        module: "prim_net",
        functions: &prim_net::FUNCTIONS,
        bindings: &[],
        load: None,
        unbound_fails: true,
    },
    Library {
        module: "prim_socket",
        functions: &prim_socket::FUNCTIONS,
        bindings: &[],
        load: Some(prim_socket::load),
        unbound_fails: true,
    },
    Library {
        module: "prim_tty",
        functions: &prim_tty::FUNCTIONS,
        bindings: prim_tty::BINDINGS,
        load: None,
        unbound_fails: false,
    },
    Library {
        module: "zlib",
        functions: &zlib::FUNCTIONS,
        bindings: zlib::BINDINGS,
        load: None,
        unbound_fails: true,
    },
    Library {
        module: "zstd",
        functions: &zstd::FUNCTIONS,
        bindings: &[],
        load: None,
        unbound_fails: true,
    },
];

/// Return the registered static NIF library of `module`, or `None` when
/// PBeam has no static library for the module.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(library("prim_tty").map(|library| library.functions[0]), Some(("isatty", 1)));
/// assert_eq!(library("prim_file").map(|library| library.functions[0]), Some(("open_nif", 2)));
/// assert!(library("runtime_tools").is_none());
/// ```
fn library(module: &str) -> Option<&'static Library> {
    let index = LIBRARIES.binary_search_by_key(&module, |library| library.module);
    Some(&LIBRARIES[index.ok()?])
}

/// Return the binding of exactly `name/arity` in the static `library`, or
/// `None`.
///
/// # Examples
///
/// ```rust,ignore
/// let prim_tty = library("prim_tty").unwrap();
/// assert!(static_binding(prim_tty, "write_nif", 2).is_some());
/// ```
fn static_binding(library: &'static Library, name: &str, arity: u32) -> Option<&'static Binding> {
    let bindings = library.bindings;
    let index =
        bindings.binary_search_by_key(&(name, arity), |binding| (binding.name, binding.arity));
    Some(&bindings[index.ok()?])
}

/// Return whether PBeam links a static NIF library of `module` itself.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(has_static_library("prim_tty"));
/// assert!(!has_static_library("native_probe"));
/// ```
pub(crate) fn has_static_library(module: &str) -> bool {
    library(module).is_some()
}

/// The NIF library of a module: a static library of this file, or else a
/// registered extension library. Registration rejects an extension library
/// of a module with a static library, so a module has at most one library.
#[derive(Clone, Copy)]
enum Linked {
    /// A library that PBeam links itself.
    Static(&'static Library),
    /// A library that an extension crate registers.
    Extension(&'static extension::Library),
}

impl Linked {
    /// Return the library of `module`, or `None` when the module has no
    /// library.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(matches!(Linked::of("prim_tty"), Some(Linked::Static(_))));
    /// assert!(Linked::of("runtime_tools").is_none());
    /// ```
    fn of(module: &str) -> Option<Self> {
        library(module)
            .map(Linked::Static)
            .or_else(|| extension::library(module).map(Linked::Extension))
    }

    /// Return the functions of the library, in the order of its `funcs`.
    fn functions(self) -> &'static [(&'static str, u32)] {
        match self {
            Linked::Static(library) => library.functions,
            Linked::Extension(library) => library.functions,
        }
    }

    /// Return whether the library has a binding of exactly `name/arity`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Linked::of("prim_tty").unwrap().binds("write_nif", 2));
    /// ```
    fn binds(self, name: &str, arity: u32) -> bool {
        match self {
            Linked::Static(library) => static_binding(library, name, arity).is_some(),
            Linked::Extension(library) => extension_binding(library, name, arity).is_some(),
        }
    }

    /// Return the runtime error of `module:name/arity` when the function is
    /// in a library whose functions without a binding fail, such as
    /// `crypto`, or `None` when the function runs its BEAM stub body.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let crypto = Linked::of("crypto").unwrap();
    /// assert!(crypto.unbound_error("crypto", "info_lib", 0).is_some());
    /// let prim_file = Linked::of("prim_file").unwrap();
    /// assert!(prim_file.unbound_error("prim_file", "open_nif", 2).is_none());
    /// ```
    fn unbound_error(self, module: &str, name: &str, arity: u32) -> Option<BeamError> {
        let fails = match self {
            Linked::Static(library) => library.unbound_fails,
            Linked::Extension(library) => library.unbound_fails,
        };
        (fails && self.functions().contains(&(name, arity)))
            .then(|| error(format!("{module}:{name}/{arity} NIF is not supported")))
    }
}

/// Return the binding of exactly `name/arity` in the extension `library`,
/// or `None`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(extension_binding(&EXTENSION_PROBE, "add", 2).is_some());
/// ```
fn extension_binding(
    library: &'static extension::Library,
    name: &str,
    arity: u32,
) -> Option<&'static extension::Binding> {
    library
        .bindings
        .iter()
        .find(|binding| binding.name == name && binding.arity == arity)
}

/// Run the native binding of `module:name/arity`, with the arity of
/// `arguments`, for the current process of `scheduler`, and return the job
/// of the binding with its result. Return `None` when PBeam has no binding
/// for the MFA; the function then runs its BEAM stub body. A function of a
/// library whose functions without a binding fail, such as `crypto`, gives
/// the runtime error `M:F/A NIF is not supported`.
///
/// The functions of an extension library run only for the instance
/// `module` of a successful `erlang:load_nif/2`, because ERTS makes the NIF
/// code callable only then (`beam/erl_nif.c:4990-5007`); before the load
/// and after a failed load, the function runs its BEAM stub body. A
/// function that `module` declares with `PBeam.Native` (the `pbeam_native`
/// attribute) runs its binding without a load, as
/// `docs/portable-beam-spec.md` gives for a declared native. An extension
/// binding runs at once for the job `Normal`. For a `CpuBound` or `IoBound`
/// job, the result is `BifResult::Dirty` with the work of the call, and the
/// binding runs on a dirty thread.
///
/// # Examples
///
/// ```rust,ignore
/// let stdout = Term::Atom(scheduler.atoms.put("stdout")?);
/// // `false` when stdout is a file.
/// assert!(matches!(call(&mut scheduler, &prim_tty, "isatty", &[stdout]), Some((Job::Normal, Ok(BifResult::Value(_))))));
/// assert!(call(&mut scheduler, &nif_error_probe, "nif_stub", &[]).is_none());
/// ```
///
/// # Errors
///
/// A binding gives a runtime error for a part that PBeam does not implement
/// yet, or when a new result atom does not fit in the atom table.
pub(super) fn call(
    scheduler: &mut Scheduler,
    module: &BeamModule,
    name: &str,
    arguments: &[Term],
) -> Option<(Job, Result<BifResult, BeamError>)> {
    let arity = u32::try_from(arguments.len()).ok()?;
    let linked = Linked::of(&module.name)?;
    let bound = match linked {
        Linked::Static(library) => static_binding(library, name, arity)
            .map(|binding| (binding.job, (binding.run)(scheduler, arguments))),
        Linked::Extension(library) => {
            let declared = module
                .native_declarations()
                .iter()
                .any(|(declared, found)| declared == name && *found == arity);
            if !declared && !scheduler.extension_loads.contains(&module.id) {
                return None;
            }
            extension_binding(library, name, arity)
                .map(|binding| (binding.job, run_extension(scheduler, binding, arguments)))
        }
    };
    bound.or_else(|| {
        linked
            .unbound_error(&module.name, name, arity)
            .map(|error| (Job::Normal, Err(error)))
    })
}

/// Run the extension `binding` with `arguments`: at once for the job
/// `Normal`, or as the work of a dirty thread for a `CpuBound` or `IoBound`
/// job.
///
/// # Examples
///
/// ```rust,ignore
/// // A `CpuBound` binding gives its work to the dirty CPU pool.
/// assert!(matches!(run_extension(&mut scheduler, &CPU_BINDING, &[]), Ok(BifResult::Dirty(_))));
/// ```
fn run_extension(
    scheduler: &mut Scheduler,
    binding: &extension::Binding,
    arguments: &[Term],
) -> Result<BifResult, BeamError> {
    match binding.job {
        Job::Normal => {
            (binding.run)(&mut NifContext::new(&mut scheduler.atoms), arguments).map(to_bif_result)
        }
        Job::CpuBound | Job::IoBound => Ok(BifResult::Dirty(Work::Native {
            function: binding.run,
            arguments: arguments.to_vec(),
        })),
    }
}

/// Map the narrow result of an extension binding to the interpreter result.
/// A binding raises only the class `error`, as a NIF does.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(to_bif_result(context.badarg()), BifResult::Raise { .. }));
/// ```
pub(super) fn to_bif_result(result: crate::runtime::NativeResult) -> BifResult {
    match result {
        crate::runtime::NativeResult::Value(term) => BifResult::Value(term),
        crate::runtime::NativeResult::Raise { reason } => BifResult::Raise {
            class: ExceptionClass::Error,
            reason,
        },
    }
}

/// Run `erlang:load_nif(Path, LoadInfo)` from code of the module `caller`,
/// with the arguments in `x0` and `x1`, as the ERTS `i_load_nif`
/// instruction does with `erts_load_nif` (`beam/erl_nif.c:4708-5039`). The
/// steps are in the ERTS order, with the PBeam capability check as step 2:
///
/// 1. A `Path` that ERTS does not accept as a file name raises
///    `error:badarg` (`beam/erl_nif.c:4730-4741`,
///    `beam_jit_common.cpp:679-704`). `is_native_filename` gives the check
///    with the file name encoding of the host.
/// 2. The caller needs a `runtime` capability that permits `load_nif`,
///    else the call raises `error:capability_denied` and loads nothing.
///    The check comes after the argument check and before the library
///    state, so a denied call gives the same result for each library.
/// 3. A module without a PBeam static library or a registered extension
///    library gives a runtime error. ERTS opens the library with `dlopen`
///    (`beam/erl_nif.c:4778-4787`).
/// 4. When the library of the module is loaded, the result is
///    `{error, {reload, Text}}` (`beam/erl_nif.c:4774-4777`).
/// 5. In the order of the library functions, a function that the module
///    does not define gives `{error, {bad_lib, "Function not found
///    M:F/A"}}`. When the module has a `-nifs` list, a function that is not
///    in it gives `{error, {bad_lib, "Function not declared as nif
///    M:F/A"}}` (`beam/erl_nif.c:4845-4866`). A function that is a second
///    time in the library gives `{error, {bad_lib, "Duplicate NIF entry
///    for M:F/A"}}` (`beam/erl_nif.c:4896-4900`). The loader keeps the `-nifs`
///    list only when a function of the module starts with `nif_start`
///    (`jit/asm_load.c:651-661`, `beam/erl_nif.c:4700-4704`).
/// 6. The load callback of the library runs with `LoadInfo`. A veto that is
///    not 0 gives `{error, {load, "Library load-call unsuccessful
///    (Veto)."}}` (`beam/erl_nif.c:4980-4986`). The load callback of
///    `prim_tty` always returns 0 (`prim_tty_nif.c:1274-1279`). The load
///    callbacks of `crypto`, `prim_socket`, and `asn1rt_nif` check the
///    load info (`crypto.c:182-357`, `prim_socket_nif.c:18862-18867`,
///    `asn1_erl_nif.c:1320-1329`).
/// 7. Else the library is loaded, and the result is `ok`.
///
/// An error does not load the library, so the next call gives the same
/// error. `LoadInfo` goes only to the load callback.
///
/// # Examples
///
/// ```rust,ignore
/// // x0 = "prim_tty", from prim_tty: the first call gives `ok`, the next
/// // gives the `reload` error.
/// assert!(matches!(load_nif(&mut scheduler, "prim_tty")?, BifResult::Value(_)));
/// // x0 = foo raises `error:badarg`.
/// assert!(matches!(load_nif(&mut scheduler, "prim_tty")?, BifResult::Raise { .. }));
/// // x0 = "load_nif_probe", from load_nif_probe.
/// assert!(load_nif(&mut scheduler, "load_nif_probe").is_err());
/// ```
///
/// # Errors
///
/// A module without a PBeam static library gives a runtime error that
/// names the module. The load callback of `crypto` gives a runtime error for a
/// `FipsMode` of `true`. A new error atom or a new `capability_denied` atom in a full
/// atom table gives the error of `AtomTable::put`.
pub(super) fn load_nif(scheduler: &mut Scheduler, caller: &str) -> Result<BifResult, BeamError> {
    let arguments = call_arguments(&scheduler.current_process_mut().registers, 2)?;
    if !is_native_filename(&arguments[0], filename_encoding()) {
        return Ok(BifResult::badarg());
    }
    if let Some(denied) = denial(scheduler, CapabilityKind::Runtime, Operation::LoadNif, None)? {
        return Ok(denied);
    }
    let Some(linked) = Linked::of(caller) else {
        // ponytail: PBeam opens no NIF library; see "NIF stubs" in docs/erts-differences.md.
        return Err(error(format!(
            "erlang:load_nif/2 from {caller} is not supported: PBeam has no native bindings for the module"
        )));
    };
    let extension = matches!(linked, Linked::Extension(_));
    if extension && scheduler.is_multi() {
        // Each scheduler of the run keeps its code table, so a load on one
        // scheduler cannot make the NIFs callable on the other schedulers.
        return Err(error(format!(
            "unsupported: erlang:load_nif/2 of the extension library of {caller} in a run of execute_on_schedulers"
        )));
    }
    let module = Arc::clone(scheduler.code.waiting_or_loaded(caller)?);
    let loaded = if extension {
        scheduler.extension_loads.contains(&module.id)
    } else {
        scheduler.nif_libraries.contains(caller)
    };
    if loaded {
        return load_nif_error(
            &mut scheduler.atoms,
            "reload",
            "NIF library already loaded (reload disallowed since OTP 20).",
        );
    }
    let instructions = &module.instructions;
    let has_nifs = instructions
        .iter()
        .any(|instruction| matches!(instruction, Instruction::NifStart { .. }));
    let functions = linked.functions();
    for (index, &(name, arity)) in functions.iter().enumerate() {
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
        if functions[..index].contains(&(name, arity)) {
            let text = format!("Duplicate NIF entry for {caller}:{name}/{arity}");
            return load_nif_error(&mut scheduler.atoms, "bad_lib", &text);
        }
    }
    if extension && !has_nifs {
        // ERTS replaces each function of the library in a module without
        // `nif_start` (`beam/erl_nif.c:4700-4704`, `5050-5098`). PBeam runs
        // an extension binding only at `nif_start`.
        return Err(error(format!(
            "unsupported: erlang:load_nif/2 of the extension library of {caller}: the module has no nif_start instruction"
        )));
    }
    // A library on another instance of the module, such as the code that
    // `erlang:delete_module/1` made old, makes the load an upgrade, and an
    // extension library has no upgrade callback (`beam/erl_nif.c:4956-4961`).
    // PBeam keeps the code of a deleted module, so the instance stays.
    let upgrade = scheduler.extension_loads.iter().any(|&id| {
        id != module.id
            && scheduler
                .code
                .module_by_id(id)
                .is_ok_and(|other| other.name == caller)
    });
    if extension && upgrade {
        let text = "Upgrade not supported by this NIF library.";
        return load_nif_error(&mut scheduler.atoms, "upgrade", text);
    }
    let veto = match linked {
        Linked::Static(library) => library
            .load
            .map_or(0, |load| load(&arguments[1], &scheduler.atoms)),
        Linked::Extension(library) => library.load.map_or(0, |load| {
            load(&mut NifContext::new(&mut scheduler.atoms), &arguments[1])
        }),
    };
    if veto != 0 {
        let text = format!("Library load-call unsuccessful ({veto}).");
        return load_nif_error(&mut scheduler.atoms, "load", &text);
    }
    if extension {
        scheduler.extension_loads.insert(module.id);
    } else {
        scheduler.nif_libraries.insert(caller.to_owned());
    }
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
    let text = text.bytes().map(|byte| Term::Integer(byte.into()));
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("error"),
        Term::tuple(vec![Term::Atom(atoms.put(kind)?), Term::list_from(text)]),
    ])))
}

#[cfg(test)]
#[path = "../../../tests/runtime/interpreter/native.rs"]
mod tests;
