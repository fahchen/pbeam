//! The PBeam boot: the `pbeam_boot` module that starts the OTP standard IO
//! servers before the entry function runs.
//!
//! ERTS starts `standard_error`, `erl_signal_server`, and `user` from
//! `kernel_sup` (`lib/kernel/src/kernel.erl:124-153`), and the `init` boot
//! process then runs `-eval` with `user` as its group leader
//! (`erts/preloaded/src/init.erl:1192-1217`, `1539-1546`). PBeam has no
//! `init`, `application_controller`, or `kernel_sup`. `pbeam_boot.erl` runs
//! the same OTP start functions in the same order and then calls the entry.
//! `pbeam_boot.beam` is the output of `erlc +deterministic pbeam_boot.erl`
//! with the pinned OTP.

use super::*;

/// The name of the boot module.
const BOOT_MODULE: &str = "pbeam_boot";

/// The compiled boot module.
const BOOT_BEAM: &[u8] = include_bytes!("boot/pbeam_boot.beam");

impl BeamProgram {
    /// Load BEAM files into one program, as [`BeamProgram::load`] does, and
    /// then load the boot module `pbeam_boot`. The boot module loads last,
    /// so the atoms of the given modules have the indices that
    /// [`BeamProgram::load`] gives them.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let program = BeamProgram::load_with_boot(&[BOOT_PROBE, &erlang, &user_sup])?;
    /// assert!(program.atom("pbeam_boot").is_some());
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of [`BeamProgram::load`]. A given module with the name
    /// `pbeam_boot` is the error `duplicate BEAM module pbeam_boot`.
    pub fn load_with_boot(modules: &[&[u8]]) -> Result<Self, BeamError> {
        let mut with_boot = modules.to_vec();
        with_boot.push(BOOT_BEAM);
        Self::load(&with_boot)
    }

    /// Run `pbeam_boot:start(Module, Function)` as the root process. It
    /// starts the OTP standard IO servers, sets the registered `user`
    /// process as its group leader, and calls the zero-arity entry
    /// function. Return the result of the entry function. The run ends when
    /// no process can run and no timer is active, so the servers that wait
    /// for a message do not keep it alive.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let program = BeamProgram::load_with_boot(&modules)?;
    /// let result = program.execute_after_boot("boot_probe", "main")?;
    /// ```
    ///
    /// # Errors
    ///
    /// An entry module that is not loaded is the error
    /// `missing BEAM module M`, and an entry function that it does not
    /// export with arity 0 is the error `missing export M:F/0`, as with
    /// [`BeamProgram::execute`]. These checks run before the boot. A boot
    /// step or an entry that fails gives the errors of
    /// [`BeamProgram::execute`].
    pub fn execute_after_boot(&self, module: &str, function: &str) -> Result<Term, BeamError> {
        self.entry_label(module, function, 0)?;
        let arguments = [module, function].map(|name| {
            // ponytail: the atom table holds the name of each loaded module and exported function.
            Term::Atom(self.atom(name).expect("a loaded export has its atoms"))
        });
        self.execute(BOOT_MODULE, "start", &arguments)
    }
}
