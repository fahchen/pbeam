//! NIF resources of the node: native objects that a term refers to through
//! a reference, as ERTS magic binaries and their magic references
//! (`erl_binary.h`). All processes of a run share one store.

use std::collections::HashMap;

use super::{host::terminal, pcre2::Code};
use crate::term::{ProcessId, Reference};

/// The resources of the node. ERTS frees a magic binary when no term refers
/// to it. PBeam keeps each resource until the end of the run.
#[derive(Debug, Default)]
pub(super) struct Resources {
    /// The compiled code of each `re_pattern` by its reference, as the
    /// magic binaries of `build_compile_result` (`erl_bif_re.c:600-608`).
    regex_codes: HashMap<Reference, Code>,
    /// Each TTY resource by its reference, as the `TTYResource` objects of
    /// `tty_create_nif` (`prim_tty_nif.c:988-1072`).
    ttys: HashMap<Reference, TtyResource>,
    /// The process monitors of the TTY resources, as the pid and the TTY
    /// reference of each `tty_init/2` or `tty_select/2` that monitors its
    /// caller (`prim_tty_nif.c:1162-1165`, `1229-1230`), in the order of
    /// the calls. ERTS adds
    /// each such monitor to the end of the monitor list of the process
    /// (`erl_proc_sig_queue.c:6555`), and the exit of the process runs the
    /// list in order (`ERTS_CONTINUE_EXIT_LT_MONITORS`,
    /// `erl_process.c:14575-14582`).
    tty_monitors: Vec<(ProcessId, Reference)>,
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

impl Resources {
    /// Keep the compiled code `code` under the reference `reference`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut resources = Resources::default();
    /// resources.insert_regex_code(Reference(0), pcre2::compile(b"a", 0, None, None)?);
    /// ```
    pub(super) fn insert_regex_code(&mut self, reference: Reference, code: Code) {
        self.regex_codes.insert(reference, code);
    }

    /// Keep the TTY resource `tty` under the reference `reference`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut resources = Resources::default();
    /// resources.insert_tty(Reference(0), tty);
    /// ```
    pub(super) fn insert_tty(&mut self, reference: Reference, tty: TtyResource) {
        self.ttys.insert(reference, tty);
    }

    /// Return the TTY resource of `reference`, as `enif_get_resource` with
    /// the TTY resource type does. A reference of another resource type or
    /// of `make_ref/0` gives `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// resources.insert_tty(Reference(0), tty);
    /// assert_eq!(resources.tty(Reference(0)).map(|tty| tty.ofd), Some(1));
    /// assert!(resources.tty(Reference(1)).is_none());
    /// ```
    pub(super) fn tty(&self, reference: Reference) -> Option<&TtyResource> {
        self.ttys.get(&reference)
    }

    /// Return the TTY resource of `reference` for a change, as
    /// `Resources::tty` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// resources.tty_mut(Reference(0)).unwrap().owner = Some(ProcessId(0));
    /// ```
    pub(super) fn tty_mut(&mut self, reference: Reference) -> Option<&mut TtyResource> {
        self.ttys.get_mut(&reference)
    }

    /// Add a monitor of the process `pid` for the TTY resource `reference`,
    /// as `enif_monitor_process` in `tty_init_nif` (`prim_tty_nif.c:1163`)
    /// and in `tty_select_nif` (`prim_tty_nif.c:1230`) does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// resources.monitor_tty(ProcessId(0), Reference(0));
    /// ```
    pub(super) fn monitor_tty(&mut self, pid: ProcessId, reference: Reference) {
        self.tty_monitors.push((pid, reference));
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
    /// resources.monitor_tty(ProcessId(1), Reference(0));
    /// // fd 0 gets the first mode of the TTY resource `Reference(0)`.
    /// resources.process_down(ProcessId(1));
    /// ```
    pub(super) fn process_down(&mut self, pid: ProcessId) {
        let (down, kept) = std::mem::take(&mut self.tty_monitors)
            .into_iter()
            .partition(|&(monitor, _)| monitor == pid);
        self.tty_monitors = kept;
        for (_, reference) in down {
            let tty = &self.ttys[&reference];
            if tty.owner == Some(pid) {
                // ERTS ignores the result (prim_tty_nif.c:1239).
                let _ = terminal::set_mode(libc::STDIN_FILENO, &tty.first_mode);
            }
        }
    }
}
