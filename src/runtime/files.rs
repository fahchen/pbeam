//! The file resources of the node, as the `efile_data_t` objects of
//! `prim_file_nif.c` and their magic references (`erl_binary.h`). The
//! scheduler keeps them as the `Files` value of its `Resources`, and all
//! processes of a run share them. The process that opens a file keeps the
//! monitor of its resource in its target monitor list.

use std::{collections::HashMap, fs::File};

use crate::term::Reference;

/// The file resources of the node. ERTS frees a file resource when no term
/// refers to it after `close_nif/1`. PBeam keeps each file resource until
/// the end of the run, and a closed resource keeps no host file.
#[derive(Debug, Default)]
pub(super) struct Files {
    /// The host file of each file resource by its reference, as the
    /// resources of `open_nif` (`prim_file_nif.c:572-593`). `None` is a
    /// closed resource (`EFILE_STATE_CLOSED`, `prim_file_nif.c:618-647`).
    by_reference: HashMap<Reference, Option<File>>,
}

impl Files {
    /// Keep the open host file `file` under the reference `reference`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut files = Files::default();
    /// files.insert(Reference(0), File::open("/dev/null")?);
    /// ```
    pub(super) fn insert(&mut self, reference: Reference, file: File) {
        self.by_reference.insert(reference, Some(file));
    }

    /// Return the host file of the file resource `reference`, as
    /// `enif_get_resource` with the `efile` resource type does: `None` for
    /// a reference of another resource type or of `make_ref/0`, and
    /// `Some(None)` for a closed resource.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// files.insert(Reference(0), file);
    /// assert!(matches!(files.get_mut(Reference(0)), Some(Some(_))));
    /// assert!(files.get_mut(Reference(1)).is_none());
    /// ```
    pub(super) fn get_mut(&mut self, reference: Reference) -> Option<&mut Option<File>> {
        self.by_reference.get_mut(&reference)
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
        if let Some(file) = self.by_reference.get_mut(&reference) {
            *file = None;
        }
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/files.rs"]
mod tests;
