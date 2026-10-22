//! The file resources of the node, as the `efile_data_t` objects of
//! `prim_file_nif.c` and their magic references (`erl_binary.h`). The
//! scheduler keeps them as the `Files` value of its `Resources`, and all
//! processes of a run share them. The process that opens a file keeps the
//! monitor of its resource in its target monitor list.
//!
//! A `prim_file` NIF uses the host file on a dirty IO thread, so each
//! resource has the state of `efile_state_t` (`prim_file_nif.h:73-78`),
//! which the scheduler thread and the dirty threads share.

use std::{
    collections::HashMap,
    fs::File,
    sync::{Arc, Mutex, PoisonError},
};

use crate::term::{ProcessId, Reference};

/// The state of one file resource, as `efile_state_t`
/// (`prim_file_nif.h:73-78`). The open host file is in the state `Idle`,
/// and the operation that holds the state `Busy` holds the host file.
#[derive(Debug)]
enum State {
    /// The file is open, and no operation uses it.
    Idle(File),
    /// An operation uses the file.
    Busy,
    /// An operation uses the file, and the owner exited: the end of the
    /// operation closes the file.
    ClosePending,
    /// The file is closed.
    Closed,
}

/// One file resource: the host file and its state.
#[derive(Debug)]
pub(super) struct FileData(Mutex<State>);

impl FileData {
    /// Change the state of the resource with `change`, and return the value
    /// of `change`. No code panics while it holds the lock, so a poisoned
    /// lock still holds a valid state.
    fn with<T>(&self, change: impl FnOnce(&mut State) -> T) -> T {
        change(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Start an operation on the file, as `file_handle_wrapper` does with
    /// the change from `EFILE_STATE_IDLE` to `EFILE_STATE_BUSY`
    /// (`prim_file_nif.c:327-331`): return the host file of an idle
    /// resource, or `None` in each other state, for which the NIF gives
    /// `{error, einval}` (`prim_file_nif.c:348-354`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let file = data.acquire().unwrap();
    /// assert!(data.acquire().is_none());
    /// data.release(file);
    /// ```
    pub(super) fn acquire(&self) -> Option<File> {
        self.with(|state| match std::mem::replace(state, State::Busy) {
            State::Idle(file) => Some(file),
            other => {
                *state = other;
                None
            }
        })
    }

    /// End the operation that holds `file`, as `file_handle_wrapper` does
    /// (`prim_file_nif.c:333-347`): the resource is idle again, or, when
    /// its owner exited during the operation, the resource is closed and
    /// the host file closes with no check of the result.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// data.release(data.acquire().unwrap());
    /// ```
    pub(super) fn release(&self, file: File) {
        self.with(|state| match state {
            State::ClosePending => *state = State::Closed,
            _ => *state = State::Idle(file),
        });
    }

    /// Close an idle resource, as `close_nif` does with the change from
    /// `EFILE_STATE_IDLE` to `EFILE_STATE_CLOSED`
    /// (`prim_file_nif.c:627-639`): return its host file, which the caller
    /// closes, or `None` in each other state (`prim_file_nif.c:640-646`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(data.close().is_some());
    /// assert!(data.close().is_none());
    /// ```
    fn close(&self) -> Option<File> {
        self.with(|state| match std::mem::replace(state, State::Closed) {
            State::Idle(file) => Some(file),
            other => {
                *state = other;
                None
            }
        })
    }

    /// Run the monitor of the resource on its owner, as
    /// `owner_death_callback` (`prim_file_nif.c:377-424`) does: an idle
    /// resource closes, and the host file closes with no check of the
    /// result, as `delayed_close_nif` (`prim_file_nif.c:362-375`) does. A
    /// busy resource closes at the end of its operation.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let file = data.acquire().unwrap();
    /// data.owner_down();
    /// // The host file closes.
    /// data.release(file);
    /// ```
    fn owner_down(&self) {
        self.with(|state| match state {
            State::Busy => *state = State::ClosePending,
            State::Idle(_) => *state = State::Closed,
            State::ClosePending | State::Closed => {}
        });
    }
}

/// The file resources of the node. ERTS frees a file resource when no term
/// refers to it after `close_nif/1`. PBeam has no garbage collection of
/// resources, so it keeps the entry of a closed resource, with no host file,
/// until the end of the run: a later call with its reference gives
/// `{error, einval}`, as in ERTS.
#[derive(Debug, Default)]
pub(super) struct Files {
    /// The resource and the owner of each file resource by its reference,
    /// as the resources of `open_nif` (`prim_file_nif.c:572-593`).
    by_reference: HashMap<Reference, (Arc<FileData>, ProcessId)>,
}

impl Files {
    /// Keep the open host file `file` of the process `owner` under the
    /// reference `reference`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut files = Files::default();
    /// files.insert(Reference(0), File::open("/dev/null")?, ProcessId(1));
    /// ```
    pub(super) fn insert(&mut self, reference: Reference, file: File, owner: ProcessId) {
        let data = Arc::new(FileData(Mutex::new(State::Idle(file))));
        self.by_reference.insert(reference, (data, owner));
    }

    /// Return the file resource `reference`, as `enif_get_resource` with
    /// the `efile` resource type does: `None` for a reference of another
    /// resource type or of `make_ref/0`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// files.insert(Reference(0), file, ProcessId(1));
    /// assert!(files.get(Reference(0)).is_some());
    /// assert!(files.get(Reference(1)).is_none());
    /// ```
    pub(super) fn get(&self, reference: Reference) -> Option<Arc<FileData>> {
        let (data, _) = self.by_reference.get(&reference)?;
        Some(Arc::clone(data))
    }

    /// Return true when `reference` is a file resource that is not closed.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// files.insert(Reference(0), file, ProcessId(1));
    /// assert!(files.is_open(Reference(0)));
    /// assert!(!files.is_open(Reference(1)));
    /// ```
    pub(super) fn is_open(&self, reference: Reference) -> bool {
        self.get(reference)
            .is_some_and(|data| data.with(|state| !matches!(state, State::Closed)))
    }

    /// Mark the idle file resource `reference` closed, as `close_nif`
    /// (`prim_file_nif.c:618-647`) does, and return its host file and its
    /// owner, which the caller closes: `None` for a reference that is not a
    /// file resource, and `Some(None)` for a resource that is not idle.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// files.insert(Reference(0), file, ProcessId(1));
    /// assert!(matches!(files.close(Reference(0)), Some(Some((_, ProcessId(1))))));
    /// assert!(matches!(files.close(Reference(0)), Some(None)));
    /// ```
    pub(super) fn close(&mut self, reference: Reference) -> Option<Option<(File, ProcessId)>> {
        let (data, owner) = self.by_reference.get(&reference)?;
        Some(data.close().map(|file| (file, *owner)))
    }

    /// Run the monitor of the file resource `reference` on its owner after
    /// the owner exits (see [`FileData::owner_down`]). The owner is the
    /// only process that the resource monitors.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // The host file of `Reference(0)` closes.
    /// files.monitor_down(Reference(0));
    /// ```
    pub(super) fn monitor_down(&mut self, reference: Reference) {
        if let Some((data, _)) = self.by_reference.get(&reference) {
            data.owner_down();
        }
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/files.rs"]
mod tests;
