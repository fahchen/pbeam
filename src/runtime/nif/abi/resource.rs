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
use std::fmt;
use std::mem::MaybeUninit;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use super::Handle;
use super::env::Env;
use super::layout::{DownFn, ErlNifMonitor, ErlNifPid, ErlNifResourceTypeInit};
use crate::AtomTable;
use crate::Term;
use crate::runtime::scheduler::node::Node;
use crate::term::ProcessId;
use crate::term::Reference;

/// `ErlNifResourceDtor`: `void (*)(ErlNifEnv*, void*)` (`erl_nif.h:216`).
pub(super) type DtorFn = unsafe extern "C" fn(*mut Env, *mut c_void);

/// The resource objects with a C reference or a term, by the address of
/// their data: the `Weak` of the object and one `Arc` for each C
/// reference.
static REGISTRY: Mutex<BTreeMap<usize, Entry>> = Mutex::new(BTreeMap::new());

/// The live resource objects by their magic reference and the address of
/// the atom table of their run, which `binary_to_term/1` finds, as
/// `erts_magic_ref_lookup_bin` finds a magic binary
/// (`erl_bif_unique.c:198-231`). The atom table tells the runs of one OS
/// process apart, as each run has its own counter of references.
static REFERENCES: Mutex<BTreeMap<(u64, usize), Weak<Resource>>> = Mutex::new(BTreeMap::new());

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

/// The library and the node of a load callback, which a resource type of
/// the callback keeps.
#[derive(Debug, Clone)]
pub(super) struct Loading {
    /// The library of the load, which the type keeps open.
    pub(super) _library: Arc<Handle>,
    /// The node of the load, whose counter gives the references of the
    /// resources. A weak link, so a node that holds a resource term in a
    /// message or a table does not keep itself.
    pub(super) node: Weak<Node>,
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
    /// The library and the node of the load.
    loading: Loading,
}

impl ResourceType {
    /// Return a new type with the destructor `dtor`, for the load of the
    /// library `library` with the atom table `atoms`.
    pub(super) fn new(dtor: Option<DtorFn>, atoms: Arc<AtomTable>, loading: Loading) -> Self {
        Self {
            dtor,
            down: None,
            atoms,
            loading,
        }
    }

    /// Return a new type with the callbacks of `init`, which a library of
    /// NIF 2.16 or newer gives: ERTS reads the fields from the last one
    /// down, so a struct of `members` members has only its first
    /// `members` callbacks (`erl_nif.c:2747-2756`).
    pub(super) fn with_init(
        init: &ErlNifResourceTypeInit,
        atoms: Arc<AtomTable>,
        loading: Loading,
    ) -> Self {
        let members = init.members.max(0);
        Self {
            dtor: (members >= 1).then_some(init.dtor).flatten(),
            down: (members >= 3).then_some(init.down).flatten(),
            atoms,
            loading,
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

impl fmt::Debug for Resource {
    /// Write the magic reference and the number of words of the data.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The type and the address stay out: the type holds C callbacks,
        // and the address is a raw pointer value of the C data.
        formatter
            .debug_struct("Resource")
            .field("reference", &self.reference)
            .field("words", &self.words)
            .finish_non_exhaustive()
    }
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
                );
            };
        }
        lock().remove(&self.data);
        references().remove(&(self.reference.0, Arc::as_ptr(&self.kind.atoms).addr()));
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

/// Lock the table of the magic references, also after a panic of another
/// thread.
fn references() -> std::sync::MutexGuard<'static, BTreeMap<(u64, usize), Weak<Resource>>> {
    REFERENCES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Return the resource term of the live object with the magic reference
/// `reference` in the run of the atom table `atoms`, or `None`. ERTS
/// `dec_term` gives the live magic reference of an encoded magic
/// reference (`external.c:4845-4851`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(find(&atoms, Reference(0).magic_reference()), None);
/// ```
pub(crate) fn find(atoms: &AtomTable, reference: Reference) -> Option<Term> {
    let key = (reference.0, std::ptr::from_ref(atoms).addr());
    let object = references().get(&key).and_then(Weak::upgrade);
    object.map(Term::Resource)
}

/// Return the data of a new object of `kind` of `size` bytes with one C
/// reference, as `enif_alloc_resource` does (`erl_nif.c:3121-3162`). The
/// data is not initialized, as in ERTS. A size that the heap cannot give
/// stops the process, as ERTS stops the node when `erts_alloc` fails.
pub(super) fn allocate(kind: Arc<ResourceType>, size: usize) -> usize {
    let words = size.div_ceil(size_of::<Word>()).max(1);
    let data = Box::into_raw(Box::<[u128]>::new_uninit_slice(words)).cast::<Word>();
    let data = data.expose_provenance();
    // ERTS gives a magic binary the next reference of the scheduler with
    // the magic marker (`erts_make_magic_ref_in_array`,
    // `erl_bif_unique.c:117-122`), so a resource term and a `make_ref/0`
    // reference sort by their creation. After the node is gone, no process
    // can hold the term, so the number does not matter.
    let number = kind
        .loading
        .node
        .upgrade()
        .map_or(0, |node| node.next_reference());
    let reference = Reference(number).magic_reference();
    let atoms = Arc::as_ptr(&kind.atoms).addr();
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
    references().insert((reference.0, atoms), Weak::clone(&entry.object));
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

/// Return the down callback of the type of the object at `data`, which
/// ERTS keeps in the monitors of the object: without it,
/// `enif_monitor_process` gives -1 (`erl_nif.c:3947-3952`).
pub(super) fn down_of(data: usize) -> Option<DownFn> {
    lock()
        .get(&data)
        .and_then(|entry| entry.object.upgrade())
        .and_then(|object| object.kind.down)
}

/// The monitors of `enif_monitor_process` that wait for the exit of their
/// process: the object and the down callback of each monitor, by the
/// number of its reference and the address of its node. Each node counts
/// its references from 0, so the address tells the runs of one OS process
/// apart. ERTS keeps them in the monitor tree of the
/// object (`erl_nif.c:3959-3966`). A monitor does not keep its object:
/// ERTS runs the down callback only while a reference keeps the object
/// (`erl_nif.c:3091-3115`), and the destructor removes the monitors of
/// the object (`erl_nif.c:2974-3030`).
static MONITORS: Mutex<BTreeMap<(u64, usize), Monitored>> = Mutex::new(BTreeMap::new());

/// A monitor of [`MONITORS`]: the object, the address of its data, and the
/// down callback of its type.
struct Monitored {
    object: Weak<Resource>,
    data: usize,
    down: DownFn,
}

/// Lock the monitor table, also after a panic of another thread.
fn monitors() -> std::sync::MutexGuard<'static, BTreeMap<(u64, usize), Monitored>> {
    MONITORS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Add the monitor `reference` to the object at `data` with the down
/// callback `down`, under the node of the type of the object.
pub(super) fn add_monitor(data: usize, down: DownFn, reference: Reference) {
    let object = lock()
        .get(&data)
        .map(|entry| Weak::clone(&entry.object))
        .unwrap_or_default();
    let key = (reference.0, node_of(&object));
    monitors().insert(key, Monitored { object, data, down });
}

/// Return the address of the node of the type of `object`, or 0 when the
/// object is gone. An object in its destructor has no monitors
/// (`erl_nif.c:2974-3030`), and no node has the address 0, so its key finds
/// no monitor. The upgraded `Arc` drops after each lock is released.
fn node_of(object: &Weak<Resource>) -> usize {
    object
        .upgrade()
        .map_or(0, |object| object.kind.loading.node.as_ptr().addr())
}

/// Remove the monitor `monitor` of the object at `data` and return true,
/// or return false when the object has no such monitor, as
/// `enif_demonitor_process` finds it (`erl_nif.c:4014-4023`).
pub(super) fn demonitor(data: usize, monitor: Reference) -> bool {
    // The entry compares the address and does not take the object, so no
    // drop of the object runs its destructor while the table is locked.
    let object = lock().get(&data).map(|entry| Weak::clone(&entry.object));
    let key = (monitor.0, object.as_ref().map_or(0, node_of));
    let mut monitors = monitors();
    let owned = monitors.get(&key).is_some_and(|entry| entry.data == data);
    owned && monitors.remove(&key).is_some()
}

/// Run the down callback of the monitor `monitor` for the exit of the
/// process `pid`, as `erts_fire_nif_monitor` does
/// (`erl_nif.c:3059-3119`): once, only while the monitor is in the table,
/// and only while a reference keeps the object. The env of the callback
/// has no process (`pre_nif_noproc`, `erl_nif.c:3110`). `node` is the
/// node of the process.
pub(crate) fn monitor_down(monitor: Reference, pid: ProcessId, node: &Node) {
    let entry = monitors().remove(&(monitor.0, std::ptr::from_ref(node).addr()));
    let Some((object, down)) = entry.and_then(|entry| Some((entry.object.upgrade()?, entry.down)))
    else {
        return;
    };
    let mut env = Env::new(&object.kind.atoms, None, &[]);
    let mut nif_pid = ErlNifPid::of(pid);
    let mut nif_monitor = ErlNifMonitor::of(monitor);
    // SAFETY: `down` is the down callback of a type whose library the
    // object keeps open, the data is the live object, and the env, the pid,
    // and the monitor live until the callback returns, as the C API gives
    // them to a down callback.
    unsafe {
        down(
            &raw mut env,
            std::ptr::with_exposed_provenance_mut(object.data),
            &raw mut nif_pid,
            &raw mut nif_monitor,
        );
    }
}

/// `ERL_NIF_RT_CREATE` of `ErlNifResourceFlags` (`erl_nif.h:188-192`).
pub(super) const CREATE: c_int = 1;
