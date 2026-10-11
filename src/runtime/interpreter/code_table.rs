use super::function_at_label;
use crate::{
    beam::BeamModule,
    error::{BeamError, error},
    runtime::process::Frame,
    term::Reference,
};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

/// The code table of one run, as the ERTS module table with the export
/// table of the active code index (`module.c`, `export.c`). Every module
/// lookup of the run reads it. A run starts with the modules of the
/// program, and `erlang:finish_loading/1` adds a new module. No operation
/// removes a module or changes its code, so a module that a lookup gives
/// stays the code of its name for the whole run.
#[derive(Debug, Default)]
pub(in crate::runtime) struct CodeTable {
    /// The loaded modules by name. The program owns the start modules too,
    /// and each run shares them through the `Rc`.
    modules: HashMap<String, Rc<BeamModule>>,
    /// The prepared code of `erts_internal:prepare_loading/2` by its
    /// reference, as the magic binary of the ERTS loader state
    /// (`beam_bif_load.c:250-265`). `erlang:finish_loading/1` takes an
    /// entry when it loads the module. An entry that does not finish stays
    /// until the run ends; ERTS frees it when the garbage collector frees
    /// the last copy of the reference.
    prepared: HashMap<Reference, BeamModule>,
}

/// The result of [`CodeTable::finish`].
#[derive(Debug, PartialEq, Eq)]
pub(in crate::runtime) enum Finish {
    /// Each module is in the code table now.
    Loaded,
    /// The list names one module more than one time. The names are the
    /// modules of the second and later entries, in the reverse order of the
    /// list, as `exception_list` (`beam_bif_load.c:322-337`) gives them.
    /// No module loads, and each entry stays prepared.
    Duplicated(Vec<String>),
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
        CodeTable {
            modules,
            prepared: HashMap::new(),
        }
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
    /// `None` when the module is not loaded: ERTS then goes to the export
    /// entry of the module, which calls the error handler
    /// (`call_error_handler` in `beam_common.c:1294-1342`). Return `None`
    /// when a loaded module does not export the function: ERTS then runs the
    /// export entry of the function, which also calls the error handler.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(code.export_frame("undef_handler", "$handle_undefined_function", 2)?.is_some());
    /// assert!(code.export_frame("undef_target", "missing", 1)?.is_none());
    /// assert!(code.export_frame("nomod", "missing", 1)?.is_none());
    /// ```
    ///
    /// # Errors
    ///
    /// A function that the module does not export at an entry label is the
    /// errors of [`frame`]. A module name is a name, so this function has no
    /// other error.
    pub(in crate::runtime) fn export_frame(
        &self,
        module: &str,
        function: &str,
        arity: u32,
    ) -> Result<Option<Frame>, BeamError> {
        self.get(module)
            .and_then(|loaded| {
                loaded
                    .exports
                    .get(&(function.to_owned(), arity))
                    .map(|label| frame(loaded, *label, arity))
            })
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

    /// Keep the prepared code `module` under `reference` until
    /// [`CodeTable::finish`] loads it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// code.prepare(reference, module);
    /// assert_eq!(code.prepared_module(&reference), Some("load_target"));
    /// ```
    pub(in crate::runtime) fn prepare(&mut self, reference: Reference, module: BeamModule) {
        self.prepared.insert(reference, module);
    }

    /// Return the module name of the prepared code of `reference`, or
    /// `None` when the reference has no prepared code, as
    /// `erts_module_for_prepared_code` (`beam_load.c:353-370`) gives `NIL`
    /// for a reference that is not prepared code or that finished.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.prepared_module(&reference), Some("load_target"));
    /// assert_eq!(code.prepared_module(&other), None);
    /// ```
    pub(in crate::runtime) fn prepared_module(&self, reference: &Reference) -> Option<&str> {
        self.prepared
            .get(reference)
            .map(|module| module.name.as_str())
    }

    /// Return the module name of each reference of `references`, in order,
    /// or `None` when a reference has no prepared code, as
    /// `erts_module_for_prepared_code` (`beam_load.c:353-370`) gives `NIL`
    /// for a reference that is not prepared code or that finished. The
    /// caller of `erlang:finish_loading/1` uses this result for its
    /// argument check and for its `code` `load` capability check, and then
    /// gives it to [`CodeTable::finish`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.prepared_names(&[first, again]), Some(vec!["load_target".to_owned(); 2]));
    /// assert_eq!(code.prepared_names(&[first]), None);
    /// ```
    pub(in crate::runtime) fn prepared_names(
        &self,
        references: &[Reference],
    ) -> Option<Vec<String>> {
        references
            .iter()
            .map(|reference| self.prepared_module(reference).map(str::to_owned))
            .collect()
    }

    /// Load the prepared code of `references`, whose module names are
    /// `names` in the same order, as `finish_loading_1`
    /// (`beam_bif_load.c:381-507`) does. `names` comes from
    /// [`CodeTable::prepared_names`] for the same `references`, so each
    /// reference has prepared code. A module that the list names more than
    /// one time gives [`Finish::Duplicated`]. Else each module goes into
    /// the code table, and its prepared entry ends.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.finish(&[first, again], vec!["load_target".into(); 2])?, Finish::Duplicated(vec!["load_target".into()]));
    /// assert_eq!(code.finish(&[first], vec!["load_target".into()])?, Finish::Loaded);
    /// assert!(code.get("load_target").is_some());
    /// ```
    ///
    /// # Errors
    ///
    /// A module that is loaded already is the runtime error
    /// `erlang:finish_loading/1 cannot replace the loaded module M`. ERTS
    /// makes the current code old and loads the new code
    /// (`beam_load.c:258-262`). PBeam has no old code.
    pub(in crate::runtime) fn finish(
        &mut self,
        references: &[Reference],
        names: Vec<String>,
    ) -> Result<Finish, BeamError> {
        let mut seen = HashSet::with_capacity(names.len());
        let mut duplicated = Vec::new();
        for name in &names {
            if !seen.insert(name) {
                duplicated.push(name.clone());
            }
        }
        if !duplicated.is_empty() {
            duplicated.reverse();
            return Ok(Finish::Duplicated(duplicated));
        }
        if let Some(name) = names.iter().find(|name| self.modules.contains_key(*name)) {
            return Err(error(format!(
                "erlang:finish_loading/1 cannot replace the loaded module {name}"
            )));
        }
        for (reference, name) in references.iter().zip(names) {
            let module = self
                .prepared
                .remove(reference)
                .expect("checked by prepared_names");
            self.modules.insert(name, Rc::new(module));
        }
        Ok(Finish::Loaded)
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
