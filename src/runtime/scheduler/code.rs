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
    beam::ModuleId,
    runtime::{
        interpreter::{CodeSnapshot, CodeTable},
        nif::abi,
    },
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
