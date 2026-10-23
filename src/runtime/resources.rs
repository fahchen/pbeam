// Copyright 2018-2025 the Deno authors. MIT license.
// Forked from Gotham:
// https://github.com/gotham-rs/gotham/blob/bcbbf8923789e341b7a0e62c59909428ca4e22e2/gotham/src/state/mod.rs
// Copyright 2017 Gotham Project Developers. MIT license.

//! The typed resources of the node: one value of each type, keyed by its
//! `TypeId`. All processes of a run share them through the scheduler.
//!
//! This module is a fork of `core/gotham_state.rs` of
//! [`deno_core`](https://github.com/denoland/deno_core), which is itself a
//! fork of the `state` module of [Gotham](https://github.com/gotham-rs/gotham).
//! Each runtime part keeps its node data as its own type in these resources,
//! so the scheduler does not hold one store type that mixes data of
//! different owners. The fork keeps only the functions that PBeam calls.

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

/// The typed values of the node that each scheduler thread reads: one
/// `Mutex<T>` of each type, keyed by its `TypeId`. A handle in one of these
/// values resolves on each scheduler, as an ERTS resource does
/// (`enif_get_resource`, `erl_nif.c`), so a use does not pin the process.
#[derive(Default)]
pub(super) struct NodeResources {
    /// The lock of each kind by the `TypeId` of its type.
    kinds: Mutex<BTreeMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl NodeResources {
    /// Return the lock of the value of type `T`, and put `T::default()`
    /// when the store has no value of that type. The caller locks it with
    /// [`lock`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let resources = NodeResources::default();
    /// let kind = resources.kind::<Buffers>();
    /// let buffers = lock(&kind);
    /// ```
    pub(super) fn kind<T: Default + Send + 'static>(&self) -> Arc<Mutex<T>> {
        let mut kinds = self
            .kinds
            .lock()
            .expect("the resource kinds are not poisoned");
        let kind = kinds
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Arc::new(Mutex::new(T::default())))
            .clone();
        kind.downcast()
            .expect("the value of the key has the type of the key")
    }
}

impl fmt::Debug for NodeResources {
    /// Write the store without its values.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeResources")
            .finish_non_exhaustive()
    }
}

/// Lock one kind of the node resources.
pub(super) fn lock<T>(kind: &Mutex<T>) -> MutexGuard<'_, T> {
    kind.lock().expect("the resource kind is not poisoned")
}

#[cfg(test)]
#[path = "../../tests/runtime/resources.rs"]
mod tests;
