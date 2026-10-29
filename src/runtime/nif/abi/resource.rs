//! The NIF resources: the resource types of `enif_open_resource_type`
//! and the objects of `enif_alloc_resource` (`erl_nif.c:2696-2785`,
//! `3121-3262`).
//!
//! ERTS keeps a resource in a magic binary with a reference count, and a
//! resource term is a magic reference to that binary. The C code holds a
//! reference from `enif_alloc_resource` and from each `enif_keep_resource`,
//! and each resource term holds one, so the destructor runs when the last
//! C reference and the last term go (`erl_nif.c:2961-3030`). PBeam keeps a
//! resource in an `Arc`: a resource term is [`Term::Resource`], and the
//! registry holds one `Arc` for each C reference. The drop of the last
//! `Arc` calls the destructor and frees the object.

use std::collections::BTreeMap;
use std::ffi::{c_int, c_void};
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use super::Handle;
use super::env::Env;
use super::layout::{DownFn, ErlNifResourceTypeInit};
use crate::AtomTable;
use crate::Term;
use crate::term::Reference;

/// `ErlNifResourceDtor`: `void (*)(ErlNifEnv*, void*)` (`erl_nif.h:216`).
pub(super) type DtorFn = unsafe extern "C" fn(*mut Env, *mut c_void);

/// The counter of the references of the resources. It starts at bit 60, so
/// the reference of a resource is never the reference of a
/// `make_ref/0` of the node, whose counter stays far below it.
static NEXT_REFERENCE: AtomicU64 = AtomicU64::new(1 << 60);

/// The resource objects with a C reference or a term, by the address of
/// their data: the `Weak` of the object and one `Arc` for each C
/// reference.
static REGISTRY: Mutex<BTreeMap<usize, Entry>> = Mutex::new(BTreeMap::new());

/// A word of the data of a resource. Its 16-byte alignment is at least
/// the alignment of the ERTS binary data (`ERTS_ALLOC_ALIGN_BYTES`).
type Word = MaybeUninit<u128>;

/// The registry entry of one resource object.
struct Entry {
    /// The object, while a C reference or a term keeps it.
    object: Weak<Resource>,
    /// One `Arc` for each C reference: the one of `enif_alloc_resource`
    /// and one for each `enif_keep_resource`.
    kept: Vec<Arc<Resource>>,
}

/// A resource type of `enif_open_resource_type`. It keeps the library of
/// its load open, so the destructor stays callable while a resource of the
/// type is alive.
pub(crate) struct ResourceType {
    /// The destructor, or `None`.
    dtor: Option<DtorFn>,
    /// The down callback of `enif_monitor_process`, or `None`.
    down: Option<DownFn>,
    /// The atom table of the load, for the env of the destructor.
    atoms: Arc<AtomTable>,
    /// The library of the load.
    _library: Arc<Handle>,
}

impl ResourceType {
    /// Return a new type with the destructor `dtor`, for the load of the
    /// library `library` with the atom table `atoms`.
    pub(super) fn new(dtor: Option<DtorFn>, atoms: Arc<AtomTable>, library: Arc<Handle>) -> Self {
        Self {
            dtor,
            down: None,
            atoms,
            _library: library,
        }
    }

    /// Return a new type with the callbacks of `init`, which a library of
    /// NIF 2.16 or newer gives: ERTS reads the fields from the last one
    /// down, so a struct of `members` members has only its first
    /// `members` callbacks (`erl_nif.c:2747-2756`).
    pub(super) fn with_init(
        init: &ErlNifResourceTypeInit,
        atoms: Arc<AtomTable>,
        library: Arc<Handle>,
    ) -> Self {
        let members = init.members.max(0);
        Self {
            dtor: (members >= 1).then_some(init.dtor).flatten(),
            down: (members >= 3).then_some(init.down).flatten(),
            atoms,
            _library: library,
        }
    }
}

/// A resource object: its magic reference, its type, and the address of
/// its data. The drop of the last `Arc` calls the destructor of the type
/// and frees the data.
pub struct Resource {
    /// The magic reference of the resource terms of the object.
    reference: Reference,
    /// The type of the object.
    kind: Arc<ResourceType>,
    /// The address of the data, which the C code reads and writes.
    data: usize,
    /// The number of words of the data.
    words: usize,
}

impl Resource {
    /// Return the magic reference of the resource terms of the object.
    pub(crate) fn reference(&self) -> Reference {
        self.reference
    }

    /// Return the address of the data when the type of the object is
    /// `kind`, as `enif_get_resource` checks it (`erl_nif.c:3255-3261`).
    pub(super) fn data_of(&self, kind: *const ResourceType) -> Option<usize> {
        std::ptr::eq(Arc::as_ptr(&self.kind), kind).then_some(self.data)
    }
}

impl Drop for Resource {
    fn drop(&mut self) {
        if let Some(dtor) = self.kind.dtor {
            // The env of a destructor has no process (`pre_nif_noproc`,
            // `erl_nif.c:3016-3021`).
            let mut env = Env::new(&self.kind.atoms, None, &[]);
            // SAFETY: `dtor` is the destructor of a library that the type
            // keeps open, and the data is the live object of the type, as
            // the C API gives them to a destructor.
            unsafe {
                dtor(
                    &raw mut env,
                    std::ptr::with_exposed_provenance_mut(self.data),
                )
            };
        }
        lock().remove(&self.data);
        let words = std::ptr::slice_from_raw_parts_mut(
            std::ptr::with_exposed_provenance_mut::<Word>(self.data),
            self.words,
        );
        // SAFETY: `allocate` made the data from a box of `self.words`
        // words, and only this drop takes the box back.
        drop(unsafe { Box::from_raw(words) });
    }
}

/// Lock the registry. The registry stays usable after a panic, because
/// each change leaves a valid map.
fn lock() -> std::sync::MutexGuard<'static, BTreeMap<usize, Entry>> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Return the data of a new object of `kind` of `size` bytes with one C
/// reference, as `enif_alloc_resource` does (`erl_nif.c:3121-3162`). The
/// data is not initialized, as in ERTS. A size that the heap cannot give
/// stops the process, as ERTS stops the node when `erts_alloc` fails.
pub(super) fn allocate(kind: Arc<ResourceType>, size: usize) -> usize {
    let words = size.div_ceil(size_of::<Word>()).max(1);
    let data = Box::into_raw(Box::<[u128]>::new_uninit_slice(words)).cast::<Word>();
    let data = data.expose_provenance();
    let reference = Reference(NEXT_REFERENCE.fetch_add(1, Ordering::Relaxed)).magic_reference();
    let object = Arc::new(Resource {
        reference,
        kind,
        data,
        words,
    });
    let entry = Entry {
        object: Arc::downgrade(&object),
        kept: vec![object],
    };
    lock().insert(data, entry);
    data
}

/// Return a resource term of the object at `data` while a C reference or
/// a term keeps it, for `enif_make_resource`, or `None`.
pub(super) fn term(data: usize) -> Option<Term> {
    let object = lock().get(&data).and_then(|entry| entry.object.upgrade());
    object.map(Term::Resource)
}

/// Add a C reference to the object at `data`, as `enif_keep_resource`
/// does (`erl_nif.c:3177-3188`).
pub(super) fn keep(data: usize) {
    if let Some(entry) = lock().get_mut(&data)
        && let Some(object) = entry.object.upgrade()
    {
        entry.kept.push(object);
    }
}

/// Remove a C reference of the object at `data`, as
/// `enif_release_resource` does (`erl_nif.c:3164-3175`). The drop of the
/// last reference calls the destructor after the registry is unlocked.
pub(super) fn release(data: usize) {
    let released = lock().get_mut(&data).and_then(|entry| entry.kept.pop());
    drop(released);
}

/// Return whether the type of the object at `data` has the down callback
/// of `enif_monitor_process`, which ERTS keeps in the monitors of the
/// object: without it, `enif_monitor_process` gives -1
/// (`erl_nif.c:3947-3952`).
pub(super) fn has_down(data: usize) -> bool {
    lock()
        .get(&data)
        .and_then(|entry| entry.object.upgrade())
        .is_some_and(|object| object.kind.down.is_some())
}

/// `ERL_NIF_RT_CREATE` of `ErlNifResourceFlags` (`erl_nif.h:188-192`).
pub(super) const CREATE: c_int = 1;
