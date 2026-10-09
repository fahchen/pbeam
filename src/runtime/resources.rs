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

use std::any::{Any, TypeId, type_name};
use std::collections::BTreeMap;
use std::fmt;

/// The typed values of the node, as `GothamState` of `deno_core`. The
/// resources hold at most one value of each type.
#[derive(Default)]
pub(super) struct Resources {
    /// Each value by the `TypeId` of its type.
    data: BTreeMap<TypeId, Box<dyn Any>>,
}

impl Resources {
    /// Put the value `value` into the resources. The resources keep one
    /// value of each type, so a new value replaces the value of the same
    /// type.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut resources = Resources::default();
    /// resources.put(RegexCodes::default());
    /// ```
    pub(super) fn put<T: 'static>(&mut self, value: T) {
        self.data.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Return a mutable borrow of the value of type `T`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut resources = Resources::default();
    /// resources.put(RegexCodes::default());
    /// resources.borrow_mut::<RegexCodes>().0.insert(reference, code);
    /// ```
    ///
    /// # Panics
    ///
    /// Panic when the resources have no value of type `T`.
    pub(super) fn borrow_mut<T: 'static>(&mut self) -> &mut T {
        self.data
            .get_mut(&TypeId::of::<T>())
            .and_then(|value| value.downcast_mut())
            .unwrap_or_else(|| missing::<T>())
    }
}

impl fmt::Debug for Resources {
    /// Write the resources without their values: a value of `dyn Any` has no
    /// `Debug` implementation.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(format!("{:?}", Resources::default()), "Resources { .. }");
    /// ```
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Resources").finish_non_exhaustive()
    }
}

/// Panic because the resources have no value of type `T`.
///
/// # Examples
///
/// ```rust,ignore
/// // panics with "required type u8 is not present in Resources"
/// missing::<u8>();
/// ```
fn missing<T: 'static>() -> ! {
    panic!(
        "required type {} is not present in Resources",
        type_name::<T>()
    );
}

#[cfg(test)]
#[path = "../../tests/runtime/resources.rs"]
mod tests;
