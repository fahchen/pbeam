//! The TTY resources of the node, as the `TTYResource` objects of
//! `prim_tty_nif.c` and their magic references (`erl_binary.h`). The
//! scheduler keeps them as the `Ttys` value of its `Resources`, and all
//! processes of a run share them. Each process keeps the monitors of these
//! resources on it in its target monitor list.

use std::collections::HashMap;

use super::host::terminal;
use crate::term::{ProcessId, Reference};

/// The TTY resources of the node. ERTS frees a TTY resource and its
/// monitors when no term refers to it. PBeam keeps each TTY resource until
/// the end of the run.
#[derive(Debug, Default)]
pub(super) struct Ttys {
    /// Each TTY resource by its reference, as the `TTYResource` objects of
    /// `tty_create_nif` (`prim_tty_nif.c:988-1072`).
    by_reference: HashMap<Reference, TtyResource>,
}

/// One terminal of `prim_tty:tty_create/1`, as the ERTS `TTYResource`
/// (`prim_tty_nif.c:98-118`) on Unix. Its input fd is always 0
/// (`prim_tty_nif.c:1001`).
#[derive(Debug)]
pub(super) struct TtyResource {
    /// The output fd: 1 for `stdout` or 2 for `stderr`.
    pub(super) ofd: libc::c_int,
    /// The state of the terminal, as the `tty` field.
    pub(super) state: TtyState,
    /// The mode of the output fd when `tty_create/1` makes the resource, as
    /// the `tty_rmode` field. It is all zeros when the output fd is not a
    /// terminal.
    pub(super) first_mode: libc::termios,
    /// The mode that `tty_init/2` sets last, as the `tty_smode` field.
    pub(super) set_mode: libc::termios,
    /// The process of the last successful `tty_init/2`, as the `self`
    /// field. `None` is the undefined pid of `tty_create/1`.
    pub(super) owner: Option<ProcessId>,
}

/// The state of a TTY resource, as `enum TTYState` (`prim_tty_nif.c:91-96`).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TtyState {
    /// The output fd is not a terminal. `tty_init/2` does not change it.
    Unavailable,
    /// The output fd is a terminal, and the input is not raw.
    Disabled,
    /// The output fd is a terminal in the raw mode of `tty_init/2`.
    Enabled,
}

impl Ttys {
    /// Keep the TTY resource `tty` under the reference `reference`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut ttys = Ttys::default();
    /// ttys.insert(Reference(0), tty);
    /// ```
    pub(super) fn insert(&mut self, reference: Reference, tty: TtyResource) {
        self.by_reference.insert(reference, tty);
    }

    /// Return the TTY resource of `reference`, as `enif_get_resource` with
    /// the TTY resource type does. A reference of another resource type or
    /// of `make_ref/0` gives `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// ttys.insert(Reference(0), tty);
    /// assert_eq!(ttys.get(Reference(0)).map(|tty| tty.ofd), Some(1));
    /// assert!(ttys.get(Reference(1)).is_none());
    /// ```
    pub(super) fn get(&self, reference: Reference) -> Option<&TtyResource> {
        self.by_reference.get(&reference)
    }

    /// Return the TTY resource of `reference` for a change, as `Ttys::get`
    /// does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// ttys.get_mut(Reference(0)).unwrap().owner = Some(ProcessId(0));
    /// ```
    pub(super) fn get_mut(&mut self, reference: Reference) -> Option<&mut TtyResource> {
        self.by_reference.get_mut(&reference)
    }

    /// Run the monitor of the TTY resource `reference` on the process `pid`
    /// after the process exits, as `tty_monitor_down`
    /// (`prim_tty_nif.c:1235-1241`) does: when `pid` is still the owner of
    /// the resource, set fd 0 to the first mode of the resource, and ignore
    /// the result of `tcsetattr`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // fd 0 gets the first mode of the TTY resource `Reference(0)`.
    /// ttys.monitor_down(ProcessId(1), Reference(0));
    /// ```
    pub(super) fn monitor_down(&self, pid: ProcessId, reference: Reference) {
        let tty = &self.by_reference[&reference];
        if tty.owner == Some(pid) {
            // ERTS ignores the result (prim_tty_nif.c:1239).
            let _ = terminal::set_mode(libc::STDIN_FILENO, &tty.first_mode);
        }
    }
}
