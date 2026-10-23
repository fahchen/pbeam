//! The code state that the schedulers of one run share.
//!
//! ERTS keeps one code table for the node and switches the active code
//! index after a load at thread progress (`code_ix.c`,
//! `beam_bif_load.c:381-507`). PBeam keeps a working copy of the code
//! table on each scheduler, with the import caches of that scheduler, so
//! the `call_ext` path takes no lock. The node keeps the published state:
//! a snapshot of the code table and the NIF library records, with a
//! generation number. A code operation in a run with scheduler threads
//! takes the lock of the node, installs the newest state, runs, and
//! publishes its result as a new generation. Another scheduler installs
//! the newest state before each external call and after it drains its
//! inbox, so a call after a load that returned sees the load, as after the
//! code barrier of ERTS (`beam_bif_load.c:550-574`). A later code replacement or
//! purge changes only the snapshot and the operations.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use super::Scheduler;
use crate::{
    beam::{BeamModule, ModuleId},
    runtime::{
        interpreter::{CodeSnapshot, CodeTable},
        nif::abi,
    },
    term::{Atom, Lambda, ProcessId, Term},
};

/// The published code state of a node.
#[derive(Debug, Default)]
pub(in crate::runtime) struct NodeCode {
    /// The generation of [`Self::state`], which a scheduler reads without
    /// the lock to see whether it must install a newer state.
    generation: AtomicU64,
    /// The published state. Generation 0 is the start state, which each
    /// scheduler has already.
    state: Mutex<CodeState>,
    /// The fun-entry table of the node.
    pub(in crate::runtime) funs: FunTable,
}

/// The fun-entry table of a node, as the ERTS fun table (`erl_fun.c:62-132`)
/// is: one entry for each module, module MD5 (`uniq`), and index, which
/// `fun_hash` and `fun_cmp` (`erl_fun.c:62-85`) use as the key. The loader
/// puts the entries of each `FunT` entry of a module in it, as
/// `erts_fun_entry_put` (`erl_fun.c:287-297`, `jit/asm_load.c:1208-1244`)
/// does, and a `make_fun3` of the code makes a fun that points to the entry.
/// The table keeps each entry while the node
/// runs. ERTS also frees no entry of loaded code, and PBeam has no purge of
/// a fun entry yet.
#[derive(Debug, Default)]
pub(in crate::runtime) struct FunTable {
    entries: Mutex<HashMap<FunKey, Arc<Lambda>>>,
}

impl FunTable {
    /// Put the fun entries of `module` into the table. A key that the table
    /// has keeps its entry. Only a load of the same code again gives such a
    /// key, and PBeam loads code again only after a purge of the old code,
    /// which it does not run yet (row "Fun-entry table" of
    /// `docs/erts-differences.md`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// node.code.funs.put(&module);
    /// ```
    pub(in crate::runtime) fn put(&self, module: &BeamModule) {
        let mut entries = self.lock();
        for lambda in &module.lambdas {
            entries
                .entry(key(lambda))
                .or_insert_with(|| Arc::clone(lambda));
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<FunKey, Arc<Lambda>>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The key of a fun entry: the atom index of the module, the module MD5, and
/// the index (`fun_cmp`, `erl_fun.c:79-85`).
type FunKey = (u32, [u8; 16], i32);

/// Return the key of `lambda` in the fun-entry table.
fn key(lambda: &Lambda) -> FunKey {
    (lambda.module().index(), *lambda.uniq(), lambda.index())
}

/// The code table and the NIF library records of one generation.
#[derive(Debug, Default)]
pub(in crate::runtime) struct CodeState {
    generation: u64,
    code: CodeSnapshot,
    nif_libraries: HashSet<String>,
    extension_loads: HashSet<ModuleId>,
    shared_libraries: HashMap<ModuleId, Arc<abi::Library>>,
}

impl NodeCode {
    /// Lock the published state.
    pub(in crate::runtime) fn lock(&self) -> MutexGuard<'_, CodeState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Scheduler {
    /// Return the code table after [`Self::sync_code`], for a lookup of an
    /// external function, which sees each code operation that returned on
    /// another scheduler.
    pub(in crate::runtime) fn synced_code(&mut self) -> &CodeTable {
        self.sync_code();
        &self.code
    }

    /// Install `state` when it is newer than the code of this scheduler.
    /// The caller holds the lock of the node code, so that a code
    /// operation runs on the newest state.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let node = Arc::clone(scheduler.node());
    /// let mut state = node.code.lock();
    /// scheduler.install_code(&state);
    /// ```
    pub(in crate::runtime) fn install_code(&mut self, state: &CodeState) {
        if state.generation == self.code_generation {
            return;
        }
        self.code.install(state.code.clone());
        self.nif_libraries.clone_from(&state.nif_libraries);
        self.extension_loads.clone_from(&state.extension_loads);
        self.shared_libraries.clone_from(&state.shared_libraries);
        self.code_generation = state.generation;
    }

    /// Publish the code of this scheduler as the next generation of
    /// `state`, after a code operation.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.publish_code(&mut state);
    /// ```
    pub(in crate::runtime) fn publish_code(&mut self, state: &mut CodeState) {
        state.code = self.code.snapshot();
        state.nif_libraries.clone_from(&self.nif_libraries);
        state.extension_loads.clone_from(&self.extension_loads);
        state.shared_libraries.clone_from(&self.shared_libraries);
        state.generation += 1;
        self.code_generation = state.generation;
        self.node
            .code
            .generation
            .store(state.generation, Ordering::Release);
    }

    /// Return whether the live process `target` has a frame in the old
    /// code of `module`, as `check_process_code` (`beam_bif_load.c:1149-1212`)
    /// looks at the instruction pointer and the continuation pointers of
    /// the stack. The check first installs the newest published code, so
    /// that it sees the purge that another scheduler prepared.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!scheduler.check_process_code(pid, &module));
    /// ```
    pub(in crate::runtime) fn check_process_code(
        &mut self,
        target: ProcessId,
        module: &Atom,
    ) -> bool {
        self.sync_code();
        let old = self.code.old_code(self.atoms.name_of(module));
        self.live_process(target)
            .zip(old)
            .is_some_and(|(process, old)| process.frames.iter().any(|frame| frame.module_id == old))
    }

    /// Install the newest published code state when another scheduler
    /// published one since the last install. A run with one scheduler has
    /// no other publisher.
    /// The check is inline, because each external call makes it.
    #[inline]
    pub(in crate::runtime) fn sync_code(&mut self) {
        if self.multi {
            self.sync_shared_code();
        }
    }

    /// Install the newest published code state of a run with scheduler
    /// threads when it is newer than the code of this scheduler.
    #[cold]
    #[inline(never)]
    fn sync_shared_code(&mut self) {
        if self.node.code.generation.load(Ordering::Acquire) == self.code_generation {
            return;
        }
        let node = Arc::clone(&self.node);
        let state = node.code.lock();
        self.install_code(&state);
    }
}

/// Return the reply `{check_process_code, Tag, Result}` of a
/// `check_process_code` system task (`erl_process.c:11309-11492`).
pub(in crate::runtime) fn check_process_code_reply(tag: Term, refers: bool) -> Term {
    Term::tuple(vec![
        Term::static_atom("check_process_code"),
        tag,
        Term::static_atom(if refers { "true" } else { "false" }),
    ])
}
