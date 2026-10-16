//! The file resources of the node, as the `efile_data_t` objects of
//! `prim_file_nif.c` and their magic references (`erl_binary.h`). The
//! scheduler keeps them as the `Files` value of its `Resources`, and all
//! processes of a run share them. The process that opens a file keeps the
//! monitor of its resource in its target monitor list.

use std::{collections::HashMap, fs::File};

use crate::term::{ProcessId, Reference};

/// The file resources of the node. ERTS frees a file resource when no term
/// refers to it after `close_nif/1`. PBeam has no garbage collection of
/// resources, so it keeps the entry of a closed resource, with no host file
/// and no owner, until the end of the run: a later call with its reference
/// gives `{error, einval}`, as in ERTS.
#[derive(Debug, Default)]
pub(super) struct Files {
    /// The host file and the owner of each file resource by its reference,
    /// as the resources of `open_nif` (`prim_file_nif.c:572-593`). `None` is
    /// a closed resource (`EFILE_STATE_CLOSED`, `prim_file_nif.c:618-647`).
    by_reference: HashMap<Reference, Option<(File, ProcessId)>>,
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
        self.by_reference.insert(reference, Some((file, owner)));
    }

    /// Return the host file of the file resource `reference`, as
    /// `enif_get_resource` with the `efile` resource type does: `None` for
    /// a reference of another resource type or of `make_ref/0`, and
    /// `Some(None)` for a closed resource.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// files.insert(Reference(0), file, ProcessId(1));
    /// assert!(matches!(files.get(Reference(0)), Some(Some(_))));
    /// assert!(files.get(Reference(1)).is_none());
    /// ```
    pub(super) fn get(&self, reference: Reference) -> Option<Option<&File>> {
        self.by_reference
            .get(&reference)
            .map(|file| file.as_ref().map(|(file, _)| file))
    }

    /// Close the file resource `reference`, as `close_nif`
    /// (`prim_file_nif.c:618-647`) does, and return the owner of the
    /// resource: `None` for a reference that is not a file resource, and
    /// `Some(None)` for a closed resource.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// files.insert(Reference(0), file, ProcessId(1));
    /// assert_eq!(files.close(Reference(0)), Some(Some(ProcessId(1))));
    /// assert_eq!(files.close(Reference(0)), Some(None));
    /// ```
    pub(super) fn close(&mut self, reference: Reference) -> Option<Option<ProcessId>> {
        let file = self.by_reference.get_mut(&reference)?;
        Some(file.take().map(|(_, owner)| owner))
    }

    /// Run the monitor of the file resource `reference` on its owner after
    /// the owner exits, as `owner_death_callback` and `delayed_close_nif`
    /// (`prim_file_nif.c:362-375`, `377-405`) do: close the host file when
    /// the resource is still open. The owner is the only process that the
    /// resource monitors.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // The host file of `Reference(0)` closes.
    /// files.monitor_down(Reference(0));
    /// ```
    pub(super) fn monitor_down(&mut self, reference: Reference) {
        self.close(reference);
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/files.rs"]
mod tests;
