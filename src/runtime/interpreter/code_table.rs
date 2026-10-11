use super::function_at_label;
use crate::{
    beam::BeamModule,
    error::{BeamError, error},
    runtime::process::Frame,
};
use std::{collections::HashMap, rc::Rc};

/// The code table of one run, as the ERTS module table with the export
/// table of the active code index (`module.c`, `export.c`). Every module
/// lookup of the run reads it. A run starts with the modules of the
/// program. No operation removes a module or changes its code, so a module
/// that a lookup gives stays the code of its name for the whole run.
#[derive(Debug, Default)]
pub(in crate::runtime) struct CodeTable {
    /// The loaded modules by name. The program owns the start modules too,
    /// and each run shares them through the `Rc`.
    modules: HashMap<String, Rc<BeamModule>>,
}

impl CodeTable {
    /// Return the code table of a run that starts with `modules`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let code = CodeTable::new(program.modules.clone());
    /// assert!(code.get("erlang").is_some());
    /// ```
    pub(in crate::runtime) fn new(modules: HashMap<String, Rc<BeamModule>>) -> Self {
        CodeTable { modules }
    }

    /// Return the loaded module `name`, or `None` when no module of the run
    /// has the name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.get("erlang").unwrap().name, "erlang");
    /// assert!(code.get("nomod").is_none());
    /// ```
    pub(in crate::runtime) fn get(&self, name: &str) -> Option<&Rc<BeamModule>> {
        self.modules.get(name)
    }

    /// Return the loaded module `name`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.module("fixture")?.name, "fixture");
    /// assert!(code.module("nomod").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// A module that is not loaded is the error `missing BEAM module M`.
    pub(in crate::runtime) fn module(&self, name: &str) -> Result<&Rc<BeamModule>, BeamError> {
        self.get(name)
            .ok_or_else(|| error(format!("missing BEAM module {name}")))
    }

    /// Return the entry label of an exported function that a run starts
    /// from.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let label = code.entry_label("process_probe", "main", 0)?;
    /// assert!(code.entry_label("process_probe", "missing", 0).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// A module that is not loaded is the error `missing BEAM module M`. A
    /// function that the module does not export is the error
    /// `missing export M:F/A`.
    pub(in crate::runtime) fn entry_label(
        &self,
        module: &str,
        function: &str,
        arity: u32,
    ) -> Result<u32, BeamError> {
        self.module(module)?
            .exports
            .get(&(function.to_owned(), arity))
            .copied()
            .ok_or_else(|| error(format!("missing export {module}:{function}/{arity}")))
    }

    /// Create the frame for an exported function of a loaded module. Return
    /// `None` when the module does not export the function: ERTS then runs
    /// the export entry of the function, which calls the error handler
    /// (`call_error_handler` in `beam_common.c:1294-1342`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(code.export_frame("undef_handler", "$handle_undefined_function", 2)?.is_some());
    /// assert!(code.export_frame("undef_target", "missing", 1)?.is_none());
    /// assert!(code.export_frame("nomod", "missing", 1).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// A module that is not loaded is the runtime error
    /// `unresolved external MFA M:F/A`. ERTS loads it from the code path.
    pub(in crate::runtime) fn export_frame(
        &self,
        module: &str,
        function: &str,
        arity: u32,
    ) -> Result<Option<Frame>, BeamError> {
        let loaded = self.get(module).ok_or_else(|| {
            error(format!(
                "unresolved external MFA {module}:{function}/{arity}"
            ))
        })?;
        loaded
            .exports
            .get(&(function.to_owned(), arity))
            .map(|label| frame(loaded, *label, arity))
            .transpose()
    }

    /// Create the frame for an exported function, such as the target of a
    /// BIF trap.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let frame = code.mfa_frame("external", "effect", 1)?;
    /// ```
    ///
    /// # Errors
    ///
    /// A module that is not loaded or a function that the module does not
    /// export is the runtime error `unresolved external MFA M:F/A`.
    pub(in crate::runtime) fn mfa_frame(
        &self,
        module: &str,
        function: &str,
        arity: u32,
    ) -> Result<Frame, BeamError> {
        self.export_frame(module, function, arity)?.ok_or_else(|| {
            error(format!(
                "unresolved external MFA {module}:{function}/{arity}"
            ))
        })
    }

    /// Create the frame for the function at an entry label of the loaded
    /// module `module`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let frame = code.frame("fixture", 2, 0)?;
    /// assert_eq!(frame.function, "main");
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of [`CodeTable::module`] and [`frame`].
    pub(in crate::runtime) fn frame(
        &self,
        module: &str,
        label: u32,
        arity: u32,
    ) -> Result<Frame, BeamError> {
        frame(self.module(module)?, label, arity)
    }
}

/// Create the frame for the function at an entry label of `module`.
///
/// # Examples
///
/// ```rust,ignore
/// let frame = frame(&module, 2, 0)?;
/// assert_eq!(frame.function, "main");
/// ```
///
/// # Errors
///
/// A label that starts no function is an error. A function with another
/// arity is the error `call arity does not match M:F`.
pub(in crate::runtime) fn frame(
    module: &BeamModule,
    label: u32,
    arity: u32,
) -> Result<Frame, BeamError> {
    let function = function_at_label(module, label)?;
    if function.arity != arity {
        return Err(error(format!(
            "call arity does not match {}:{}",
            module.name, function.name
        )));
    }
    Ok(Frame {
        module: module.name.clone(),
        function: function.name.to_owned(),
        next_instruction: function.start,
        stack: Vec::new(),
    })
}
