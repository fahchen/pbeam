//! The NIF C ABI: open a NIF shared library, read its `ErlNifEntry`, and
//! call its functions with the `enif_*` functions of [`env`].
//!
//! This module is the only part of PBeam where `unsafe` is allowed for the
//! NIF ABI (`docs/adr/0001-unsafe-in-the-nif-abi-layer.md`). It gives a
//! safe interface: no raw pointer and no C struct leaves it. Each `unsafe`
//! block states the invariant and the C API contract that it relies on.
//! As in ERTS, the native code runs in the process of the runtime: a
//! library that breaks the C API, or that crashes, stops the process.
//!
//! [`open`] does the steps of `erts_load_nif` that touch the library
//! (`erl_nif.c:4778-4813`): `dlopen` of `Name.so` with `RTLD_NOW |
//! RTLD_LOCAL` (`erl_unix_sys_ddll.c:107-158`), `dlsym` of `nif_init`, else
//! `_nif_init` (`erl_unix_sys_ddll.c:230-242`), the call of the init
//! function, and the version checks. PBeam supports only NIF version 2.18,
//! the version of OTP 29.1.1 (`erl_nif.h:67-68`). The caller does the
//! other checks with the safe [`Library`].

mod env;
mod layout;
mod mutex;
mod process;
mod resource;
pub(crate) mod thread;
mod unsupported;

pub(crate) use resource::Resource;

use std::ffi::{CStr, CString, c_char, c_int, c_void};

use std::sync::Arc;

use crate::AtomTable;
use crate::BeamError;
use crate::Term;
use crate::error::error;
use crate::runtime::scheduler::node::Node;
use crate::runtime::{Job, NativeResult};
use crate::term::ProcessId;
use env::Env;
use layout::{ErlNifEntry, InitFn, LoadFn, NifFn};

/// The NIF version that PBeam supports (`ERL_NIF_MAJOR_VERSION` and
/// `ERL_NIF_MINOR_VERSION`, `erl_nif.h:67-68`).
pub(crate) const VERSION: (i32, i32) = (2, 18);

/// The failure of [`open`], before the library has a valid entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OpenError {
    /// `dlopen` failed with the text of `dlerror`.
    Open(String),
    /// The library has neither `nif_init` nor `_nif_init`. The text is the
    /// `dlerror` of the `_nif_init` lookup.
    NoInit(String),
    /// The init function returned NULL.
    NullEntry,
    /// The library needs a newer NIF version than 2.18: the `min_erts` of
    /// the entry.
    Newer(String),
    /// The NIF version is older than 2.0, or the experimental 2.5.
    Old(i32, i32),
    /// The NIF version is one that ERTS loads and PBeam does not support:
    /// 2.0 to 2.17, except 2.5.
    Unsupported(i32, i32),
}

/// The process of a NIF call or a load callback: its id, its atom table,
/// and its node.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Caller<'a> {
    /// The calling process.
    pub(crate) process: ProcessId,
    /// The atom table of the process.
    pub(crate) atoms: &'a Arc<AtomTable>,
    /// The node of the process.
    pub(crate) node: &'a Arc<Node>,
}

/// The end of a NIF call: its result and the messages of its `enif_send`
/// calls, which the scheduler of the caller sends in their order.
#[derive(Debug)]
pub(crate) struct Called {
    /// The result of the call.
    pub(crate) result: Result<NativeResult, BeamError>,
    /// The receiver and the term of each message.
    pub(crate) sends: Vec<(ProcessId, Term)>,
}

/// A NIF function of a library.
#[derive(Debug)]
pub(crate) struct Function {
    /// The name, decoded as Latin-1, as ERTS finds its atom
    /// (`erl_nif.c:4853`).
    pub(crate) name: String,
    /// The arity.
    pub(crate) arity: u32,
    /// The `flags` of the C entry: 0 for a normal NIF.
    pub(crate) flags: u32,
    /// The C function.
    fptr: NifFn,
}

impl Function {
    /// Return the job of the function: `ERL_NIF_DIRTY_JOB_CPU_BOUND` (1)
    /// and `ERL_NIF_DIRTY_JOB_IO_BOUND` (2) run on a dirty thread
    /// (`erl_drv_nif.h:49-52`). The loader rejects other flags than 0, 1,
    /// and 2.
    pub(crate) fn job(&self) -> Job {
        match self.flags {
            1 => Job::CpuBound,
            2 => Job::IoBound,
            _ => Job::Normal,
        }
    }

    /// Call the function with `arguments` on the current thread for the
    /// process `caller` and return its result: a value, or the
    /// `error:Reason` of `enif_raise_exception`, and its messages.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // nif_add:add/2 of fixtures/nif/nif_add.c.
    /// let add = &library.functions()[library.position("add", 2).unwrap()];
    /// assert!(matches!(add.call(caller, &[Term::Integer(1), Term::Integer(2)]).result, Ok(NativeResult::Value(Term::Integer(3)))));
    /// ```
    ///
    /// The result is a runtime error for a NIF that returns a handle that no
    /// term of the call has, and for a broken contract of `enif_send`.
    pub(crate) fn call(&self, caller: Caller<'_>, arguments: &[Term]) -> Called {
        let mut env = Env::new(caller.atoms, Some(caller.process), arguments);
        let argv = env.handles();
        // The arity of a BEAM function is at most 255.
        let argc = c_int::try_from(argv.len()).unwrap_or(c_int::MAX);
        let pointer = &raw mut env;
        let (handle, ended) = process::within(caller.atoms, caller.node, pointer, || {
            // SAFETY: `fptr` is the function of a library that `open`
            // loaded, and the library stays open while `self` is alive. The
            // env is alive and unique for the call, and `argv` holds `argc`
            // handles of it, as the C API gives a NIF its arguments.
            unsafe { (self.fptr)(pointer, argc, argv.as_ptr()) }
        });
        let result = match ended.unsupported {
            Some(text) => Err(error(text)),
            None => env.finish(handle),
        };
        Called {
            result,
            sends: ended.sends,
        }
    }
}

/// An open NIF library of version 2.18. Drop closes it with `dlclose`.
#[derive(Debug)]
pub(crate) struct Library {
    /// The module name of the entry, decoded as Latin-1.
    name: String,
    /// The vm variant of the entry.
    vm_variant: String,
    /// The functions of the entry, in their order.
    functions: Vec<Function>,
    /// The load callback, or `None`.
    load: Option<LoadFn>,
    /// Whether the entry has an upgrade callback.
    upgrade: bool,
    /// The `dlopen` handle. The library and each resource type of its load
    /// share it, and the drop of the last one closes the library. It is
    /// the last field, so the drop closes the library after the other
    /// fields.
    handle: Arc<Handle>,
}

impl Library {
    /// Return the module name of the library.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Return the vm variant of the library, `beam.vanilla` for ERTS.
    pub(crate) fn vm_variant(&self) -> &str {
        &self.vm_variant
    }

    /// Return the functions of the library, in the order of its entry.
    pub(crate) fn functions(&self) -> &[Function] {
        &self.functions
    }

    /// Return the index of the first function of exactly `name/arity` in
    /// [`Library::functions`], or `None`.
    pub(crate) fn position(&self, name: &str, arity: u32) -> Option<usize> {
        self.functions
            .iter()
            .position(|function| function.name == name && function.arity == arity)
    }

    /// Return whether the library has an upgrade callback.
    pub(crate) fn upgrades(&self) -> bool {
        self.upgrade
    }

    /// Run the load callback with `info` for the process `caller`, and
    /// return its veto, or 0 when the library has no load callback
    /// (`erl_nif.c:4980-4987`). The callback runs in no thread-local
    /// context of [`process`], but the run of `caller` is the run that a
    /// thread of the library finds from then on, so `enif_alloc_env` and
    /// `enif_send` work in it, as ERTS finds its global tables from each
    /// thread (`erl_nif.c:669-675`, `2296-2338`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // The callback of the NIF_ADD_LOAD library returns its load info.
    /// assert_eq!(library.load(caller, &Term::Integer(7)), 7);
    /// ```
    pub(crate) fn load(&self, caller: Caller<'_>, info: &Term) -> i32 {
        process::enter_run(caller.atoms, caller.node);
        caller.node.load_nif_library();
        self.load.map_or(0, |load| {
            let mut env = Env::load(caller.atoms, caller.process, info, &self.handle);
            // PBeam has no `enif_priv_data` yet, so no call reads the
            // private data of the callback.
            let mut private: *mut c_void = std::ptr::null_mut();
            // The handle of `info`: 1, or the atom handle of an atom.
            let info = env.handles()[0];
            // SAFETY: `load` is the callback of a library that `open`
            // loaded and that `self` keeps open. The env is alive and
            // unique for the call, `info` is the handle of the load info,
            // and `private` is a writable `void*`, as the C API gives them.
            unsafe { load(&raw mut env, &raw mut private, info) }
        })
    }
}

/// A `dlopen` handle, kept as its address so the library is `Send` and
/// `Sync` with no `unsafe impl`. A handle of `dlopen` is valid on each
/// thread. Drop closes it.
#[derive(Debug)]
struct Handle(usize);

impl Drop for Handle {
    fn drop(&mut self) {
        let handle = std::ptr::with_exposed_provenance_mut::<c_void>(self.0);
        // SAFETY: the handle is the result of one successful `dlopen`, and
        // this drop is its only `dlclose`. No function of the library runs
        // after the drop, because each `Function` lives in its `Library`.
        unsafe { libc::dlclose(handle) };
    }
}

/// Return the text of the last `dl` error of the thread, or an empty text.
fn last_error() -> String {
    // SAFETY: `dlerror` has no argument; its result is NULL or a
    // NUL-terminated string that stays valid until the next `dl` call of
    // the thread.
    let text = unsafe { libc::dlerror() };
    if text.is_null() {
        return String::new();
    }
    // SAFETY: the text is not NULL, and no `dl` call comes before the copy.
    unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned()
}

/// Return the Latin-1 text of the C string `text`.
///
/// # Safety
///
/// `text` is a NUL-terminated string that stays valid during the call.
unsafe fn latin1(text: *const c_char) -> String {
    // SAFETY: the caller gives a NUL-terminated string.
    let bytes = unsafe { CStr::from_ptr(text) }.to_bytes();
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

/// Return the address of the symbol `name` of `handle`, or the `dlerror`
/// text when the lookup fails, as `erts_sys_ddll_sym2` does with `dlerror`
/// (`erl_unix_sys_ddll.c:163-183`).
fn symbol(handle: &Handle, name: &CStr) -> Result<Option<InitFn>, String> {
    let pointer = std::ptr::with_exposed_provenance_mut::<c_void>(handle.0);
    // SAFETY: the handle is open, and `name` is a NUL-terminated string.
    // `dlerror` clears the last error first, as ERTS does.
    let symbol = unsafe {
        libc::dlerror();
        libc::dlsym(pointer, name.as_ptr())
    };
    let failure = last_error();
    if !failure.is_empty() {
        return Err(failure);
    }
    // SAFETY: `nif_init` is the `ErlNifEntry* nif_init(void)` of
    // `ERL_NIF_INIT_DECL` (`erl_nif.h:407-408`). A NULL address is `None`.
    Ok(unsafe { std::mem::transmute::<*mut c_void, Option<InitFn>>(symbol) })
}

/// Open the NIF library `name` with the suffix `.so`, call its init
/// function, and check its NIF version, as `erts_load_nif` does
/// (`erl_nif.c:4778-4813`).
///
/// # Examples
///
/// ```rust,ignore
/// let library = open(c"/tmp/nif_add").unwrap();
/// assert_eq!(library.name(), "nif_add");
/// assert!(matches!(open(c"/tmp/missing"), Err(OpenError::Open(_))));
/// ```
///
/// # Errors
///
/// Each [`OpenError`]. On each error, the library is closed.
pub(crate) fn open(name: &CStr) -> Result<Library, OpenError> {
    let path = [name.to_bytes(), b".so"].concat();
    // A `CStr` has no NUL byte before its end, and `.so` has none.
    let path = CString::new(path).expect("the path of a C string has no NUL byte");
    // SAFETY: `path` is a NUL-terminated string. `dlerror` clears the last
    // error first, as `erts_sys_ddll_open_noext` does.
    let opened = unsafe {
        libc::dlerror();
        libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL)
    };
    if opened.is_null() {
        return Err(OpenError::Open(last_error()));
    }
    let handle = Handle(opened.expose_provenance());
    let init = symbol(&handle, c"nif_init")
        .or_else(|_| symbol(&handle, c"_nif_init"))
        .map_err(OpenError::NoInit)?;
    let init = init.ok_or(OpenError::NullEntry)?;
    // SAFETY: the init function of a NIF library has no argument and
    // returns its entry or NULL (`erl_nif.h:422-444`).
    let entry = unsafe { init() };
    if entry.is_null() {
        return Err(OpenError::NullEntry);
    }
    // SAFETY: the entry of each NIF version starts with `major` and
    // `minor` (`erl_nif.h:135-139`), so the reads stay in the entry of an
    // older or a newer version too.
    let (major, minor) = unsafe {
        (
            (&raw const (*entry).major).read(),
            (&raw const (*entry).minor).read(),
        )
    };
    if (major, minor) > VERSION {
        // SAFETY: an entry newer than 2.14 has `min_erts`, a
        // NUL-terminated string (`erl_nif.h:153-154`).
        let min_erts = unsafe { latin1((&raw const (*entry).min_erts).read()) };
        return Err(OpenError::Newer(min_erts));
    }
    if major < 2 || (major, minor) == (2, 5) {
        return Err(OpenError::Old(major, minor));
    }
    if (major, minor) != VERSION {
        return Err(OpenError::Unsupported(major, minor));
    }
    // SAFETY: the entry is a full `ErlNifEntry` of version 2.18, which the
    // init function keeps alive while the library is open.
    let entry: &ErlNifEntry = unsafe { &*entry };
    let count = usize::try_from(entry.num_of_funcs).unwrap_or(0);
    let functions = (0..count)
        .map(|index| {
            // SAFETY: `funcs` points to `num_of_funcs` entries that the
            // init function keeps alive, as the C API requires, and `index`
            // is below `num_of_funcs`.
            let function = unsafe { &*entry.funcs.add(index) };
            Function {
                // SAFETY: the `name` of a function is a NUL-terminated
                // string, as the C API requires.
                name: unsafe { latin1(function.name) },
                arity: function.arity,
                flags: function.flags,
                fptr: function.fptr,
            }
        })
        .collect();
    Ok(Library {
        // SAFETY: the `name` of an entry is a NUL-terminated string, as the
        // C API requires.
        name: unsafe { latin1(entry.name) },
        // SAFETY: the `vm_variant` of a 2.18 entry is a NUL-terminated
        // string, as the C API requires.
        vm_variant: unsafe { latin1(entry.vm_variant) },
        functions,
        load: entry.load,
        upgrade: entry.upgrade.is_some(),
        handle: Arc::new(handle),
    })
}
