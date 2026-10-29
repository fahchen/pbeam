//! The TTY resources of the node, as the `TTYResource` objects of
//! `prim_tty_nif.c` and their magic references (`erl_binary.h`). The
//! node keeps them as its `Ttys` value, and each scheduler and all
//! processes of a run share them. Each process keeps the monitors of these
//! resources on it in its target monitor list. The module also keeps the
//! mode of fd 0 at the start of a run, which the run sets again at its end.

use std::{collections::HashMap, time::Duration};

use super::host::{
    input,
    terminal::{self, TerminalCheck},
};
use crate::term::{ProcessId, Reference, Term};

/// The TTY resources of the node. ERTS frees a TTY resource and its
/// monitors when no term refers to it. PBeam keeps each TTY resource until
/// the end of the run.
#[derive(Debug, Default)]
pub(super) struct Ttys {
    /// Each TTY resource by its reference, as the `TTYResource` objects of
    /// `tty_create_nif` (`prim_tty_nif.c:988-1072`).
    by_reference: HashMap<Reference, TtyResource>,
    /// The read select of fd 0, the input fd of each TTY resource, or
    /// `None` when fd 0 has no select.
    input_select: Option<InputSelect>,
}

/// The read select of fd 0 that `enif_select` with `ERL_NIF_SELECT_READ`
/// makes (`erl_check_io.c:1236-1465`): the `in` event of the NIF select
/// state of fd 0. ERTS keeps one select state for each fd, so a new select
/// replaces the old one. The select is one-shot: when fd 0 has input, the
/// IO poll sends the message `{select, TTY, Reference, ready_input}` to the
/// recipient and removes the select (`erl_check_io.c:2135-2151`).
#[derive(Debug)]
pub(super) struct InputSelect {
    /// The TTY resource of the select, as `state->driver.stop.resource`.
    pub(super) tty: Reference,
    /// The process that gets the message, as the `pid` of the event.
    pub(super) recipient: ProcessId,
    /// The third element of the message.
    pub(super) reference: Term,
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
    /// The process of the last `tty_select/2`, as the `reader` field.
    /// `None` is the undefined pid of `tty_create/1`.
    pub(super) reader: Option<ProcessId>,
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
    /// (`prim_tty_nif.c:1235-1245`) does: when `pid` is still the owner of
    /// the resource, set fd 0 to the first mode of the resource, and ignore
    /// the result of `tcsetattr`. Then, when `pid` is still the reader of
    /// the resource, remove the select of fd 0, as `enif_select` with
    /// `ERL_NIF_SELECT_STOP` does (`erl_check_io.c:1278-1290`, `1485-1517`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // fd 0 gets the first mode of the TTY resource `Reference(0)`.
    /// ttys.monitor_down(ProcessId(1), Reference(0));
    /// ```
    pub(super) fn monitor_down(&mut self, pid: ProcessId, reference: Reference) {
        let tty = &self.by_reference[&reference];
        if tty.owner == Some(pid) {
            // ERTS ignores the result (prim_tty_nif.c:1239).
            let _ = terminal::set_mode(libc::STDIN_FILENO, &tty.first_mode);
        }
        if tty.reader == Some(pid) {
            self.input_select = None;
        }
    }

    /// Select fd 0 for read for the TTY resource `tty`, as `enif_select`
    /// with `ERL_NIF_SELECT_READ` does (`erl_check_io.c:1236-1465`). The
    /// select replaces each earlier select of fd 0.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// ttys.select_input(InputSelect { tty: Reference(0), recipient: ProcessId(1), reference });
    /// assert!(ttys.input_selected());
    /// ```
    pub(super) fn select_input(&mut self, select: InputSelect) {
        self.input_select = Some(select);
    }

    /// Return `true` when fd 0 has a read select.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!Ttys::default().input_selected());
    /// ```
    pub(super) fn input_selected(&self) -> bool {
        self.input_select.is_some()
    }

    /// Remove the read select of fd 0 and return it, as the IO poll does
    /// when fd 0 has input (`erl_check_io.c:2135-2151`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let select = ttys.take_input_select().unwrap();
    /// assert!(ttys.take_input_select().is_none());
    /// ```
    pub(super) fn take_input_select(&mut self) -> Option<InputSelect> {
        self.input_select.take()
    }
}

/// Whether fd 0 has a read select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StdinRead {
    /// No process selects fd 0 for read.
    Unselected,
    /// The reader of the node selects fd 0 for read.
    Selected,
}

impl From<bool> for StdinRead {
    fn from(selected: bool) -> Self {
        if selected {
            Self::Selected
        } else {
            Self::Unselected
        }
    }
}

/// The mode of fd 0 at the start of a run, as the `erl_sys_initial_tty_mode`
/// that `erl_sys_init` reads when fd 0 is a terminal (`sys.c:428-432`).
/// `None` when fd 0 is not a terminal.
#[derive(Debug)]
pub(super) struct InitialMode(Option<libc::termios>);

impl InitialMode {
    /// Read the mode of fd 0 with `tcgetattr`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // fd 0 is a pipe.
    /// assert!(InitialMode::read().0.is_none());
    /// ```
    pub(super) fn read() -> Self {
        Self(terminal::get_mode(libc::STDIN_FILENO))
    }

    /// Return the mode of fd 0 at the start of the run, or `None` when fd 0
    /// is not a terminal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // fd 0 is a pipe.
    /// assert!(InitialMode::read().mode().is_none());
    /// ```
    pub(super) fn mode(&self) -> Option<&libc::termios> {
        self.0.as_ref()
    }

    /// Set fd 0 to the mode at the start of the run when fds 0 and 1 are
    /// terminals, and ignore the result of `tcsetattr`. ERTS does this at
    /// each emulator exit in `sys_tty_reset` (`erl_init.c:2730`,
    /// `sys.c:182-190`), and `erl_child_setup` does it when the emulator
    /// closes its socket, also after `SIGKILL` (`sys_drivers.c:1670-1690`,
    /// `erl_child_setup.c:563-576`).
    ///
    /// `input` tells whether fd 0 has a read select. A change that
    /// turns `ICANON` on sets `PENDIN` in the macOS kernel and wakes each
    /// `select` of the terminal. A `select` reads the input count with
    /// `ttnread`, which clears `PENDIN` again (XNU `bsd/kern/tty.c`:
    /// `ttioctl_locked`, `ttyselect`, `ttnread`). On macOS, ERTS keeps a
    /// terminal fd 0 with a read select in the `select` set of its poll
    /// thread, and the emulator exits at once after the reset
    /// (`erl_init.c:2730-2742`). Thus `PENDIN` stays set on ERTS only when
    /// the poll thread does not run between the reset and the exit. PBeam
    /// gives the end state in which the poll thread runs first: after the
    /// reset, it checks fd 0 once with a `select` of timeout 0.
    /// Linux does not use `PENDIN`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let initial_mode = InitialMode::read();
    /// // A process sets fd 0 to raw mode, and the reader selects fd 0.
    /// initial_mode.reset(StdinRead::Selected);
    /// // fd 0 has its first mode again, without `PENDIN`.
    /// ```
    pub(super) fn reset(&self, input: StdinRead) {
        let Some(mode) = &self.0 else {
            return;
        };
        if terminal::check(libc::STDIN_FILENO) != TerminalCheck::Terminal
            || terminal::check(libc::STDOUT_FILENO) != TerminalCheck::Terminal
        {
            return;
        }
        // ERTS ignores the result (sys.c:188, erl_child_setup.c:566, 575).
        let _ = terminal::set_mode(libc::STDIN_FILENO, mode);
        if input == StdinRead::Selected {
            input::wait_readable(&[libc::STDIN_FILENO], Some(Duration::ZERO));
        }
    }
}
