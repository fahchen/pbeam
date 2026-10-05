//! The TTY resources of the node, as the `TTYResource` objects of
//! `prim_tty_nif.c` and their magic references (`erl_binary.h`), with the
//! process monitors of these resources. The scheduler keeps them in its
//! `State`, and all processes of a run share them.

use std::collections::HashMap;

use super::host::terminal;
use crate::term::{ProcessId, Reference};

/// The TTY resources of the node and their process monitors. ERTS frees a
/// TTY resource and its monitors when no term refers to it. PBeam keeps
/// each TTY resource until the end of the run.
#[derive(Debug, Default)]
pub(super) struct Ttys {
    /// Each TTY resource by its reference, as the `TTYResource` objects of
    /// `tty_create_nif` (`prim_tty_nif.c:988-1072`).
    resources: HashMap<Reference, TtyResource>,
    /// The process monitors of the TTY resources, as the pid and the TTY
    /// reference of each `tty_init/2` or `tty_select/2` that monitors its
    /// caller (`prim_tty_nif.c:1162-1165`, `1229-1230`), in the order of
    /// the calls. ERTS adds
    /// each such monitor to the end of the monitor list of the process
    /// (`erl_proc_sig_queue.c:6555`), and the exit of the process runs the
    /// list in order (`ERTS_CONTINUE_EXIT_LT_MONITORS`,
    /// `erl_process.c:14575-14582`).
    monitors: Vec<(ProcessId, Reference)>,
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
        self.resources.insert(reference, tty);
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
        self.resources.get(&reference)
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
        self.resources.get_mut(&reference)
    }

    /// Add a monitor of the process `pid` for the TTY resource `reference`,
    /// as `enif_monitor_process` in `tty_init_nif` (`prim_tty_nif.c:1163`)
    /// and in `tty_select_nif` (`prim_tty_nif.c:1230`) does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// ttys.monitor(ProcessId(0), Reference(0));
    /// ```
    pub(super) fn monitor(&mut self, pid: ProcessId, reference: Reference) {
        self.monitors.push((pid, reference));
    }

    /// Run the TTY monitors of the process `pid` after it exits, in the
    /// order of the monitors, and remove them. As `tty_monitor_down`
    /// (`prim_tty_nif.c:1235-1241`) does, a monitor of a TTY resource whose
    /// owner is still `pid` sets fd 0 to the first mode of the resource,
    /// and the result of `tcsetattr` is ignored.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// ttys.monitor(ProcessId(1), Reference(0));
    /// // fd 0 gets the first mode of the TTY resource `Reference(0)`.
    /// ttys.process_down(ProcessId(1));
    /// ```
    pub(super) fn process_down(&mut self, pid: ProcessId) {
        let (down, kept) = std::mem::take(&mut self.monitors)
            .into_iter()
            .partition(|&(monitor, _)| monitor == pid);
        self.monitors = kept;
        for (_, reference) in down {
            let tty = &self.resources[&reference];
            if tty.owner == Some(pid) {
                // ERTS ignores the result (prim_tty_nif.c:1239).
                let _ = terminal::set_mode(libc::STDIN_FILENO, &tty.first_mode);
            }
        }
    }
}
