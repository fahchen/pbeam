use super::function_at_label;
use crate::{
    beam::{BeamModule, ModuleId, NameId},
    error::{BeamError, error},
    runtime::process::{Frame, InitialCall},
    term::{AtomTable, Lambda, Reference},
};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

#[cfg(test)]
thread_local! {
    /// The number of calls of [`CodeTable::export_frame`] on this test
    /// thread.
    ///
    /// A call that reads the resolved export target of its import gives the
    /// same value as a lookup by name, so a test of the fast path reads this
    /// counter. The counter is per thread, because the test harness runs the
    /// tests of one binary on parallel threads.
    static EXPORT_FRAME_CALLS: Cell<usize> = const { Cell::new(0) };
}

/// Return the number of calls of [`CodeTable::export_frame`] on this test
/// thread.
///
/// # Examples
///
/// ```rust,ignore
/// let before = export_frame_calls();
/// ```
#[cfg(test)]
pub(in crate::runtime::interpreter) fn export_frame_calls() -> usize {
    EXPORT_FRAME_CALLS.with(Cell::get)
}

/// The code table of one run, as the ERTS module table with the export
/// table of the active code index (`module.c`, `export.c`). Every module
/// lookup of the run reads it. A run starts with the modules of the
/// program, and `erlang:finish_loading/1` adds a new module. No operation
/// removes a loaded module or changes its code, so a module that a lookup
/// gives stays the code of its name for the whole run. Only the purge of
/// the code of a failed `-on_load` function removes code (see
/// [`CodeTable::prepare_on_load_purge`]), and no export names that code.
#[derive(Debug, Default)]
pub(in crate::runtime) struct CodeTable {
    /// The loaded modules by name. The program owns the start modules too,
    /// and each run shares them through the `Arc`.
    modules: HashMap<String, Arc<BeamModule>>,
    /// The loaded modules by their identity, for a frame that names its
    /// module by [`ModuleId`] and not by name, with the export targets of
    /// the imports of each module in this run.
    by_id: HashMap<ModuleId, LoadedModule>,
    /// The prepared code of `erts_internal:prepare_loading/2` by its
    /// reference, as the magic binary of the ERTS loader state
    /// (`beam_bif_load.c:250-265`). `erlang:finish_loading/1` takes an
    /// entry when it loads the module. An entry that does not finish stays
    /// until the run ends; ERTS frees it when the garbage collector frees
    /// the last copy of the reference.
    prepared: HashMap<Reference, Arc<BeamModule>>,
    /// The code of each module that `erlang:finish_loading/1` loaded with an
    /// `-on_load` function, by module name, as the ERTS `on_load` instance
    /// of the module (`beam_load.c:305-313`). The code is in `by_id`, so
    /// that its `on_load` function runs, but not in `modules`: the module is
    /// not loaded, and its exports stay hidden until
    /// [`CodeTable::finish_after_on_load`] keeps the code.
    on_load: HashMap<String, OnLoadCode>,
    /// The modules that have old code, as `modp->old` of ERTS
    /// (`beam_bif_load.c:2412-2455`). `erlang:delete_module/1` moves the
    /// current code here, and a later load keeps the old code and adds new
    /// current code. The code stays in `by_id` for the frames that run it,
    /// and a second delete gives `badarg`. A load with current and old code
    /// gives `not_purged` (`beam_bif_load.c:444-455`).
    old: HashSet<String>,
    /// The module names in the order of their first load, as the indices
    /// of the ERTS module table: `erlang:finish_loading/1` puts the entry of
    /// each module of its list in list order, and an entry keeps its index
    /// after a delete (`beam_bif_load.c:425`, `beam_load.c:258`).
    /// `erlang:loaded/0` reads it.
    order: Vec<String>,
    /// The `-on_load` flag of each reference of
    /// `erts_internal:prepare_loading/2` that `erlang:finish_loading/1`
    /// consumed. The ERTS magic reference stays valid after the load, so
    /// `erlang:has_prepared_code_on_load/1` gives the flag again
    /// (`beam_bif_load.c:268-283`). ERTS frees the loader state when the
    /// garbage collector frees the last copy of the reference; PBeam has no
    /// garbage collection, so the entry stays until the run ends.
    finished_prepared: HashMap<Reference, bool>,
    /// The module name and the code of the purge that
    /// `erts_internal:purge_module(Module, prepare_on_load)` started, as
    /// `purge_state.module` and the old code of the ERTS module
    /// (`beam_bif_load.c:2134-2160`). The code stays in `by_id` until
    /// [`CodeTable::complete_purge`] removes it, and it is in no other map.
    purging: Option<(String, Arc<BeamModule>)>,
}

/// The state of a [`CodeTable`] without the import caches of its
/// scheduler, which the node publishes to the other schedulers of a run.
/// It is the code index that ERTS commits after a load or a delete
/// (`code_ix.c`, `beam_bif_load.c:381-507`). A scheduler that installs it
/// starts with empty import caches.
#[derive(Debug, Clone, Default)]
pub(in crate::runtime) struct CodeSnapshot {
    modules: HashMap<String, Arc<BeamModule>>,
    by_id: HashMap<ModuleId, Arc<BeamModule>>,
    prepared: HashMap<Reference, Arc<BeamModule>>,
    on_load: HashMap<String, OnLoadCode>,
    old: HashSet<String>,
    order: Vec<String>,
    finished_prepared: HashMap<Reference, bool>,
    purging: Option<(String, Arc<BeamModule>)>,
}

/// The code of a module that waits for its `-on_load` function.
#[derive(Debug, Clone)]
struct OnLoadCode {
    /// The new code of the module.
    code: Arc<BeamModule>,
    /// `true` while the exports wait for the code, as the deferred address of
    /// each ERTS export entry (`jit/asm_load.c:1188-1195`).
    /// `erlang:finish_after_on_load(Module, false)` clears it
    /// (`beam_bif_load.c:918-937`).
    deferred: bool,
}

/// A loaded module of one run and the export target of each of its imports
/// in that run.
#[derive(Debug, Clone)]
pub(in crate::runtime) struct LoadedModule {
    /// The code of the module, which the runs of the program share.
    pub(in crate::runtime) code: Arc<BeamModule>,
    /// The export target of each import of the module, by the index of the
    /// import in [`BeamModule::imports`].
    pub(in crate::runtime) imports: ImportTargets,
}

impl LoadedModule {
    /// Return the loaded module of `code`, with no resolved import.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let loaded = LoadedModule::new(Arc::clone(&module));
    /// assert_eq!(loaded.imports.len(), module.imports.len());
    /// ```
    fn new(code: Arc<BeamModule>) -> Self {
        let imports = code.imports.iter().map(|_| Cell::new(None)).collect();
        LoadedModule { code, imports }
    }
}

/// The export target of each import of one module in one run, as the ERTS
/// loader patches the export entry of each import into the code
/// (`jit/asm_load.c:1198-1206`). A `call_ext` reads its slot and makes no
/// lookup by name. A slot is empty until the first call finds the export of
/// its MFA. The slot then keeps the target for the whole run, because no
/// operation of the run removes a module or changes its code (see
/// [`CodeTable`]). A call that finds no export keeps the slot empty, so a
/// later call finds a module that loads after it, as the load sets the code
/// address in the ERTS export entry (`jit/asm_load.c:1194`). The slots
/// belong to the code table of one scheduler and not to the shared code,
/// because each run loads its own modules. A scheduler that installs the
/// code state of another scheduler starts with empty slots (see
/// [`CodeTable::from_snapshot`]).
pub(in crate::runtime) type ImportTargets = Rc<[Cell<Option<ExportTarget>>]>;

/// The function that the export of an MFA enters: the parts of the
/// [`Frame`] that [`CodeTable::export_frame`] creates.
#[derive(Debug, Clone, Copy)]
pub(in crate::runtime) struct ExportTarget {
    module_id: ModuleId,
    function: NameId,
    start: usize,
}

impl ExportTarget {
    /// Return the target of the export frame `frame`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let frame = code.mfa_frame("fixture", "main", 0)?;
    /// assert_eq!(ExportTarget::of(&frame).frame().next_instruction, frame.next_instruction);
    /// ```
    pub(in crate::runtime) fn of(frame: &Frame) -> Self {
        ExportTarget {
            module_id: frame.module_id,
            function: frame.function,
            start: frame.next_instruction,
        }
    }

    /// Create a new frame at the start of the function.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let target = ExportTarget::of(&code.mfa_frame("fixture", "main", 0)?);
    /// assert!(target.frame().stack.is_empty());
    /// ```
    pub(in crate::runtime) fn frame(self) -> Frame {
        Frame {
            module_id: self.module_id,
            function: self.function,
            next_instruction: self.start,
            stack: Vec::new(),
        }
    }
}

/// The result of [`CodeTable::finish`].
#[derive(Debug, PartialEq, Eq)]
pub(in crate::runtime) enum Finish {
    /// Each module is in the code table now.
    Loaded,
    /// The one module of the list has an `-on_load` function. Its code waits
    /// for the function, and its exports stay hidden
    /// (`beam_bif_load.c:484-504`).
    OnLoad(String),
    /// The list names one module more than one time. The names are the
    /// modules of the second and later entries, in the reverse order of the
    /// list, as `exception_list` (`beam_bif_load.c:322-337`) gives them.
    /// No module loads, and each entry stays prepared.
    Duplicated(Vec<String>),
    /// A module of the list has current code and old code from
    /// `erlang:delete_module/1`. The names are the modules in the order of
    /// the list, as `exception_list` (`beam_bif_load.c:444-455`) gives
    /// them. No module loads, and each entry stays prepared.
    NotPurged(Vec<String>),
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
    pub(in crate::runtime) fn new(modules: HashMap<String, Arc<BeamModule>>) -> Self {
        // The program loads its modules in its order, so their numbers
        // give the order.
        let mut start = modules.values().collect::<Vec<_>>();
        start.sort_by_key(|module| module.id);
        let order = start.iter().map(|module| module.name.clone()).collect();
        let by_id = modules
            .values()
            .map(|module| (module.id, LoadedModule::new(Arc::clone(module))))
            .collect();
        CodeTable {
            modules,
            by_id,
            prepared: HashMap::new(),
            on_load: HashMap::new(),
            old: HashSet::new(),
            order,
            finished_prepared: HashMap::new(),
            purging: None,
        }
    }

    /// Return the state of this table without its import caches.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let copy = CodeTable::from_snapshot(code.snapshot());
    /// assert_eq!(copy.names().count(), code.names().count());
    /// ```
    pub(in crate::runtime) fn snapshot(&self) -> CodeSnapshot {
        CodeSnapshot {
            modules: self.modules.clone(),
            by_id: self
                .by_id
                .iter()
                .map(|(id, loaded)| (*id, Arc::clone(&loaded.code)))
                .collect(),
            prepared: self.prepared.clone(),
            on_load: self.on_load.clone(),
            old: self.old.clone(),
            order: self.order.clone(),
            finished_prepared: self.finished_prepared.clone(),
            purging: self.purging.clone(),
        }
    }

    /// Return the code table of `snapshot`, with no resolved import.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let copy = CodeTable::from_snapshot(code.snapshot());
    /// assert!(copy.get("erlang").is_some());
    /// ```
    pub(in crate::runtime) fn from_snapshot(snapshot: CodeSnapshot) -> Self {
        CodeTable {
            modules: snapshot.modules,
            by_id: snapshot
                .by_id
                .into_iter()
                .map(|(id, code)| (id, LoadedModule::new(code)))
                .collect(),
            prepared: snapshot.prepared,
            on_load: snapshot.on_load,
            old: snapshot.old,
            order: snapshot.order,
            finished_prepared: snapshot.finished_prepared,
            purging: snapshot.purging,
        }
    }

    /// Replace the state of this table with `snapshot`. A module that this
    /// table has keeps its import slots, emptied, so that a cursor that
    /// holds the slots finds the code of the snapshot at its next call.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// code.install(other.snapshot());
    /// assert_eq!(code.names().count(), other.names().count());
    /// ```
    pub(in crate::runtime) fn install(&mut self, snapshot: CodeSnapshot) {
        let mut old = std::mem::take(&mut self.by_id);
        let mut table = CodeTable::from_snapshot(CodeSnapshot {
            by_id: HashMap::new(),
            ..snapshot.clone()
        });
        table.by_id = snapshot
            .by_id
            .into_iter()
            .map(|(id, code)| match old.remove(&id) {
                Some(loaded) => {
                    for slot in loaded.imports.iter() {
                        slot.set(None);
                    }
                    (id, loaded)
                }
                None => (id, LoadedModule::new(code)),
            })
            .collect();
        *self = table;
    }

    /// Return the loaded module of `id`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let module = code.get("erlang").unwrap();
    /// assert_eq!(code.module_by_id(module.id)?.name, "erlang");
    /// ```
    ///
    /// # Errors
    ///
    /// An identity that no loaded module of the run has is a runtime
    /// invariant error.
    pub(in crate::runtime) fn module_by_id(
        &self,
        id: ModuleId,
    ) -> Result<&Arc<BeamModule>, BeamError> {
        self.loaded_by_id(id).map(|loaded| &loaded.code)
    }

    /// Return the loaded module of `id` with the export targets of its
    /// imports.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let module = code.get("erlang").unwrap();
    /// assert_eq!(code.loaded_by_id(module.id)?.code.name, "erlang");
    /// ```
    ///
    /// # Errors
    ///
    /// An identity that no loaded module of the run has is a runtime
    /// invariant error.
    pub(in crate::runtime) fn loaded_by_id(
        &self,
        id: ModuleId,
    ) -> Result<&LoadedModule, BeamError> {
        self.by_id.get(&id).ok_or_else(|| {
            error(format!(
                "invariant error: no loaded module has identity {id:?}"
            ))
        })
    }

    /// Return the module name, the function name, and the arity of the start
    /// MFA `initial_call`, from the loader tables.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (module, function, arity) = code.initial_call_names(&atoms, &process.initial_call)?;
    /// assert_eq!((module.as_str(), arity), ("erlang", 0));
    /// ```
    ///
    /// # Errors
    ///
    /// An identity that no loaded module of the run has is a runtime
    /// invariant error.
    pub(in crate::runtime) fn initial_call_names(
        &self,
        atoms: &AtomTable,
        initial_call: &InitialCall,
    ) -> Result<(String, String, usize), BeamError> {
        Ok(match initial_call {
            InitialCall::Entry {
                module,
                function,
                arity,
            } => {
                let loaded = self.module_by_id(*module)?;
                (
                    loaded.name.clone(),
                    loaded.tables.name(*function).to_owned(),
                    *arity,
                )
            }
            InitialCall::Undefined {
                module,
                function,
                arity,
            } => (
                atoms.name_of(module).to_owned(),
                atoms.name_of(function).to_owned(),
                *arity,
            ),
        })
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
    pub(in crate::runtime) fn get(&self, name: &str) -> Option<&Arc<BeamModule>> {
        self.modules.get(name)
    }

    /// Return the names of the modules with current or old code, in the
    /// order of their first load (see [`Self::order`]). The code of a purge
    /// that [`Self::prepare_on_load_purge`] started is old code until the
    /// purge completes, as ERTS moves the `on_load` code to `modp->old`
    /// (`beam_bif_load.c:2134-2141`). The start modules come in the order
    /// of the program, and then each module in the order of
    /// its `erlang:finish_loading/1`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(code.names().any(|name| name == "erlang"));
    /// ```
    pub(in crate::runtime) fn names(&self) -> impl Iterator<Item = &str> {
        self.order
            .iter()
            .filter(|name| {
                self.modules.contains_key(*name)
                    || self.old.contains(*name)
                    || self.old_code(name).is_some()
            })
            .map(String::as_str)
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
    pub(in crate::runtime) fn module(&self, name: &str) -> Result<&Arc<BeamModule>, BeamError> {
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
        let loaded = self.module(module)?;
        loaded
            .tables
            .name_id(function)
            .and_then(|id| loaded.exports.get(&(id, arity)))
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
        #[cfg(test)]
        EXPORT_FRAME_CALLS.with(|calls| calls.set(calls.get() + 1));
        self.get(module)
            .and_then(|loaded| {
                loaded
                    .tables
                    .name_id(function)
                    .and_then(|id| loaded.exports.get(&(id, arity)))
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
    /// assert_eq!(frame.module_id, module.id);
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

    /// Create the frame of the code that a call of the local fun `lambda`
    /// with `arity` arguments enters, as `call_fun` finds the code of a fun
    /// (`beam_common.c:1604-1731`). The ERTS fun entry of the lambda holds
    /// the address that the loader of the code sets for the `FunT` entry of
    /// the same module, index, and arity (`erl_fun.c:48-53`), so the call
    /// enters the entry that the loaded code of the module name of the fun
    /// holds with the index of the fun. The code can wait for its `-on_load`
    /// function, because a fun of that code runs before the code becomes the
    /// loaded code. The entry must have the old unique value of the fun: ERTS
    /// keys the address of the entry by the module checksum and the index and
    /// the arity (`erl_fun.c:79-85`), and PBeam has no code replacement and
    /// no checksum in a fun, so the old unique value, a field of the module
    /// MD5 (`beam_asm.erl:251-262`), identifies the code of the fun. Return
    /// `None` when the code holds no such entry, so that the caller raises
    /// `error:{badfun, Fun}` (`beam_common.c:1697-1706`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.fun_frame("fixture", &lambda, 1)?.map(|frame| frame.module_id), Some(module.id));
    /// assert_eq!(code.fun_frame("fixture", &other_lambda, 1)?, None);
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of [`CodeTable::waiting_or_loaded`] and [`frame`].
    pub(in crate::runtime) fn fun_frame(
        &self,
        module: &str,
        lambda: &Lambda,
        arity: u32,
    ) -> Result<Option<Frame>, BeamError> {
        // ERTS keeps the fun entry at the old code after
        // `erlang:delete_module/1` until a purge, so the old code runs the
        // fun; only an unloaded entry reaches the error handler
        // (`beam_common.c:1681-1685`).
        let waiting = self.waiting_or_loaded(module).ok();
        for loaded in waiting.into_iter().chain(self.codes(module)) {
            if let Some(entry) = loaded
                .tables
                .fun_entries
                .get(&lambda.index())
                .filter(|entry| {
                    entry.lambda.uniq() == lambda.uniq() && entry.lambda.arity() == lambda.arity()
                })
            {
                return frame(loaded, entry.lambda.label(), arity).map(Some);
            }
        }
        Ok(None)
    }

    /// Return the fun entry of the code of `module` with the module MD5
    /// `uniq`, the index `index`, and the arity `arity`, as
    /// `erts_fun_entry_get_or_make_stub` (`erl_fun.c:299-309`) finds it by
    /// the key of `fun_cmp` (`erl_fun.c:79-85`). The code is the current
    /// code, the code that waits for its `-on_load` function, or the old
    /// code that `erlang:delete_module/1` kept. Return `None` when no code
    /// holds the entry, so that the caller makes a stub entry.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(code.fun_entry("fun_ext_probe", lambda.uniq(), 0, 2).is_some());
    /// assert!(code.fun_entry("missing", &[0; 16], 0, 2).is_none());
    /// ```
    pub(in crate::runtime) fn fun_entry(
        &self,
        module: &str,
        uniq: &[u8; 16],
        index: i32,
        arity: u32,
    ) -> Option<Arc<Lambda>> {
        self.codes(module)
            .filter_map(|loaded| loaded.tables.fun_entries.get(&index))
            .map(|entry| &entry.lambda)
            .find(|lambda| lambda.uniq() == uniq && lambda.arity() == arity)
            .cloned()
    }

    /// Return the name of the function that holds the code of `lambda`, as
    /// ERTS `erts_get_fun_mfa` (`erl_fun.c:37-46`) gives it: the function of
    /// the `func_info` before the entry label of the fun entry. The code of
    /// the fun is the current code, the code that waits for its `-on_load`
    /// function, or the old code that `erlang:delete_module/1` kept until
    /// the purge; the first code whose fun entry matches the fun gives the
    /// name of its function. A fun whose code the code table does not hold
    /// gives `None`. A call of a fun puts
    /// its arguments in `x(0)` onwards and then its free variables in the
    /// registers after them, so the arity of that function is the arity of
    /// the fun plus the number of free variables (`jit/asm_load.c:1218-1223`),
    /// and this function compares no arity.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let name = code.function_name_of("fun_probe", &lambda)?;
    /// assert_eq!(name.as_deref(), Some("locals"));
    /// assert_eq!(code.function_name_of("missing", &lambda)?, None);
    /// ```
    ///
    /// # Errors
    ///
    /// An entry label that starts no function is a runtime invariant error.
    pub(in crate::runtime) fn function_name_of(
        &self,
        module: &str,
        lambda: &Lambda,
    ) -> Result<Option<String>, BeamError> {
        for loaded in self.codes(module) {
            let Some(entry) = loaded
                .tables
                .fun_entries
                .get(&lambda.index())
                .filter(|entry| {
                    entry.lambda.uniq() == lambda.uniq() && entry.lambda.arity() == lambda.arity()
                })
            else {
                continue;
            };
            let function = super::function_at_label(loaded, entry.lambda.label())?;
            return Ok(Some(loaded.tables.name(function.name).to_owned()));
        }
        Ok(None)
    }

    /// Return the current code of the module `name`, the code that waits for
    /// its `-on_load` function, and each code of the name in `by_id`, which
    /// includes the old code that `erlang:delete_module/1` kept until the
    /// purge (`beam_bif_load.c:2412-2455`). A code of the name that is not
    /// current and does not wait gives its funs a name, so the caller reads
    /// each code of the name. The code of a purge that runs has no name, as
    /// the preparation of the purge clears the fun address
    /// (`erl_fun.c:159-175`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.codes("missing").count(), 0);
    /// ```
    fn codes<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Arc<BeamModule>> + 'a {
        // The preparation of a purge clears the fun address of the old code
        // (`erl_fun.c:159-175`), so `erts_get_fun_mfa` gives no MFA for its
        // funs until the purge ends.
        let purging = self.old_code(name);
        self.get(name)
            .into_iter()
            .chain(self.on_load.get(name).map(|waiting| &waiting.code))
            .chain(
                self.by_id
                    .values()
                    .map(|loaded| &loaded.code)
                    .filter(move |code| code.name == name && Some(code.id) != purging),
            )
    }

    /// Return the code of `module` that waits for its `-on_load` function,
    /// else the loaded module `module`. `erlang:load_nif/2` from an `on_load`
    /// function checks the functions of the waiting code, as ERTS
    /// `erts_load_nif` takes the `on_load` instance of the module
    /// (`beam/erl_nif.c:4764-4770`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.waiting_or_loaded("on_load_ok")?.name, "on_load_ok");
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of [`CodeTable::module`].
    pub(in crate::runtime) fn waiting_or_loaded(
        &self,
        module: &str,
    ) -> Result<&Arc<BeamModule>, BeamError> {
        match self.on_load.get(module) {
            Some(waiting) => Ok(&waiting.code),
            None => self.module(module),
        }
    }

    /// Keep the prepared code `module` under `reference` until
    /// [`CodeTable::finish`] loads it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// code.prepare(reference, module);
    /// assert_eq!(code.prepared_module(reference), Some("load_target"));
    /// ```
    pub(in crate::runtime) fn prepare(&mut self, reference: Reference, module: BeamModule) {
        self.prepared.insert(reference, Arc::new(module));
    }

    /// Return the module name of the prepared code of `reference`, or
    /// `None` when the reference has no prepared code, as
    /// `erts_module_for_prepared_code` (`beam_load.c:353-370`) gives `NIL`
    /// for a reference that is not prepared code or that finished.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.prepared_module(reference), Some("load_target"));
    /// assert_eq!(code.prepared_module(other), None);
    /// ```
    pub(in crate::runtime) fn prepared_module(&self, reference: Reference) -> Option<&str> {
        self.prepared
            .get(&reference)
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
            .map(|reference| self.prepared_module(*reference).map(str::to_owned))
            .collect()
    }

    /// Move the current code of the module `name` to old code, as
    /// `delete_module_1` and `delete_code` (`beam_bif_load.c:679-736`,
    /// `2412-2455`) do. Return `Some(true)` when the run had current code
    /// of the name, and `Some(false)` when it has old code and a delete
    /// must give `badarg` before the current code changes. Return `None` when the run has
    /// neither: the module is not loaded. The code stays in `by_id` for the
    /// frames that run it, and each import slot of another module that
    /// names the code is emptied, so a later call of the module gives
    /// `undef`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.delete("fixture"), Some(true));
    /// assert_eq!(code.delete("fixture"), Some(false));
    /// assert_eq!(code.delete("nomod"), None);
    /// assert!(code.get("fixture").is_none());
    /// ```
    pub(in crate::runtime) fn delete(&mut self, name: &str) -> Option<bool> {
        if self.old.contains(name) {
            return Some(false);
        }
        let module = self.modules.remove(name)?;
        for loaded in self.by_id.values() {
            for slot in loaded.imports.iter() {
                if slot
                    .get()
                    .is_some_and(|target| target.module_id == module.id)
                {
                    slot.set(None);
                }
            }
        }
        self.old.insert(name.to_owned());
        Some(true)
    }

    /// Load the prepared code of `references`, whose module names are
    /// `names` in the same order, as `finish_loading_1`
    /// (`beam_bif_load.c:381-507`) does. `names` comes from
    /// [`CodeTable::prepared_names`] for the same `references`, so each
    /// reference has prepared code. A module that the list names more than
    /// one time gives [`Finish::Duplicated`]. Else each module goes into
    /// the code table, and its prepared entry ends. A module with an
    /// `-on_load` function waits for its function and gives
    /// [`Finish::OnLoad`]. The caller gives such a module only in a list of
    /// one module (`beam_bif_load.c:395-408`).
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
    /// A module that is loaded already, or that waits for its `-on_load`
    /// function, is the runtime error
    /// `erlang:finish_loading/1 cannot replace the loaded module M`. ERTS
    /// makes the current code old and loads the new code
    /// (`beam_load.c:258-262`). PBeam has no old code. This error also
    /// keeps each filled [`ImportTargets`] slot valid: an operation that
    /// replaces or removes code must empty the slots that name that code.
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
        // ERTS rejects a load only when the module has current and old
        // code (`beam_bif_load.c:443-455`). A module with old code only
        // loads new current code.
        let mut not_purged: Vec<String> = names
            .iter()
            .filter(|name| self.modules.contains_key(*name) && self.old.contains(*name))
            .cloned()
            .collect();
        // `exception_list` prepends each entry of the input order
        // (`beam_bif_load.c:322-337`).
        not_purged.reverse();
        if !not_purged.is_empty() {
            return Ok(Finish::NotPurged(not_purged));
        }
        if let Some(name) = names
            .iter()
            .find(|name| self.modules.contains_key(*name) || self.on_load.contains_key(*name))
        {
            return Err(error(format!(
                "erlang:finish_loading/1 cannot replace the loaded module {name}"
            )));
        }
        for (reference, name) in references.iter().zip(names) {
            if !self.order.contains(&name) {
                self.order.push(name.clone());
            }
            let module = self
                .prepared
                .remove(reference)
                .expect("checked by prepared_names");
            self.finished_prepared
                .insert(*reference, module.on_load.is_some());
            self.by_id
                .insert(module.id, LoadedModule::new(Arc::clone(&module)));
            if module.on_load.is_some() {
                let code = OnLoadCode {
                    code: module,
                    deferred: true,
                };
                self.on_load.insert(name.clone(), code);
                return Ok(Finish::OnLoad(name));
            }
            self.modules.insert(name, module);
        }
        Ok(Finish::Loaded)
    }

    /// Return `true` when the prepared code of a reference of `references`
    /// has an `-on_load` function, as `erts_has_code_on_load`
    /// (`beam_load.c:336-349`) does. Each reference must have prepared code
    /// (see [`CodeTable::prepared_names`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(code.prepared_on_load(&[on_load_reference]));
    /// assert!(!code.prepared_on_load(&[plain_reference]));
    /// ```
    pub(in crate::runtime) fn prepared_on_load(&self, references: &[Reference]) -> bool {
        references.iter().any(|reference| {
            self.prepared
                .get(reference)
                .is_some_and(|module| module.on_load.is_some())
        })
    }

    /// Return the `-on_load` flag of the prepared code of `reference` after
    /// `erlang:finish_loading/1` consumed it, or `None` when no load
    /// consumed the code. The ERTS magic reference stays valid after the
    /// load (`beam_bif_load.c:268-283`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.finished_prepared_on_load(reference), Some(false));
    /// ```
    pub(in crate::runtime) fn finished_prepared_on_load(
        &self,
        reference: Reference,
    ) -> Option<bool> {
        self.finished_prepared.get(&reference).copied()
    }

    /// Return the frame of the `-on_load` function of the module `name` while
    /// its code waits for the function, else `None`, as
    /// `get_on_load_address` (`jit/arm/instr_bif.cpp:994-1008`) does. The
    /// function runs also after `erlang:finish_after_on_load(Module, false)`,
    /// because ERTS keeps the code until the purge.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let frame = code.on_load_frame("on_load_ok")?.unwrap();
    /// assert!(code.on_load_frame("lists")?.is_none());
    /// ```
    ///
    /// # Errors
    ///
    /// An `on_load` function with an arity that is not 0 gives the error of
    /// [`frame`].
    pub(in crate::runtime) fn on_load_frame(&self, name: &str) -> Result<Option<Frame>, BeamError> {
        self.on_load
            .get(name)
            .and_then(|waiting| {
                waiting
                    .code
                    .on_load
                    .map(|label| frame(&waiting.code, label, 0))
            })
            .transpose()
    }

    /// Keep or drop the code of the module `name` after its `-on_load`
    /// function, as `finish_after_on_load_2` (`beam_bif_load.c:819-941`)
    /// does. Return `false`, and change nothing, when no code of `name`
    /// waits for its function. With `keep`, the code becomes the loaded
    /// code of the module, and its exports become visible. Without `keep`,
    /// the exports no longer wait for the code, and the code stays until a
    /// purge, as in ERTS. Else return `true`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(code.finish_after_on_load("on_load_ok", true)?);
    /// assert!(code.get("on_load_ok").is_some());
    /// assert!(!code.finish_after_on_load("on_load_ok", true)?);
    /// ```
    ///
    /// # Errors
    ///
    /// `keep` after an earlier call without `keep` gives the runtime error
    /// `erlang:finish_after_on_load/2 cannot keep the code of M after its
    /// exports were dropped`. ERTS then makes the code current with exports
    /// that call the error handler (`beam_bif_load.c:887-897`), and PBeam has
    /// no such export state.
    pub(in crate::runtime) fn finish_after_on_load(
        &mut self,
        name: &str,
        keep: bool,
    ) -> Result<bool, BeamError> {
        let Some(waiting) = self.on_load.get_mut(name) else {
            return Ok(false);
        };
        if !keep {
            waiting.deferred = false;
            return Ok(true);
        }
        if !waiting.deferred {
            return Err(error(format!(
                "erlang:finish_after_on_load/2 cannot keep the code of {name} after its exports were dropped"
            )));
        }
        let waiting = self.on_load.remove(name).expect("checked above");
        self.modules.insert(name.to_owned(), waiting.code);
        Ok(true)
    }

    /// Start the purge of the code of the module `name` after its failed
    /// `-on_load` function, as `erts_internal:purge_module(Module,
    /// prepare_on_load)` does (`beam_bif_load.c:2100-2189`): the code that
    /// waits for the function becomes the old code of the module, which
    /// [`CodeTable::old_code`] gives. Return `Some(true)` then. Return
    /// `Some(false)` when the run has no module `name`, and `None`, with no
    /// change, while a purge runs.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.prepare_on_load_purge("on_load_error")?, Some(true));
    /// assert_eq!(code.prepare_on_load_purge("on_load_error")?, None);
    /// ```
    ///
    /// # Errors
    ///
    /// A loaded module with no code that waits for its `-on_load` function
    /// gives the runtime error `erts_internal:purge_module/2 found no
    /// on_load code of M`: the ERTS assertion `modp->on_load` fails
    /// (`beam_bif_load.c:2134-2140`).
    pub(in crate::runtime) fn prepare_on_load_purge(
        &mut self,
        name: &str,
    ) -> Result<Option<bool>, BeamError> {
        if self.purging.is_some() {
            return Ok(None);
        }
        let Some(waiting) = self.on_load.remove(name) else {
            if self.modules.contains_key(name) {
                return Err(error(format!(
                    "erts_internal:purge_module/2 found no on_load code of {name}"
                )));
            }
            return Ok(Some(false));
        };
        self.purging = Some((name.to_owned(), waiting.code));
        Ok(Some(true))
    }

    /// Return the identity of the old code of the module `name`, else
    /// `None`. Only a purge that [`CodeTable::prepare_on_load_purge`]
    /// started gives a module old code, because PBeam has no code
    /// replacement.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(code.old_code("lists"), None);
    /// ```
    pub(in crate::runtime) fn old_code(&self, name: &str) -> Option<ModuleId> {
        self.purging
            .as_ref()
            .filter(|(purged, _)| purged == name)
            .map(|(_, code)| code.id)
    }

    /// Return whether a purge that [`CodeTable::prepare_on_load_purge`]
    /// started runs, as `purge_state.module` of ERTS is set
    /// (`beam_bif_load.c:2108-2110`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!code.old_code_pending());
    /// ```
    pub(in crate::runtime) fn old_code_pending(&self) -> bool {
        self.purging.is_some()
    }

    /// Return whether the module `name` has old code after
    /// `erlang:delete_module/1`, as `modp->old.code_hdr` of ERTS
    /// (`beam_bif_load.c:2143`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!code.has_old("lists"));
    /// ```
    pub(in crate::runtime) fn has_old(&self, name: &str) -> bool {
        self.old.contains(name)
    }

    /// Remove the old code of the purge of the module `name`, as
    /// `erts_internal:purge_module(Module, complete)` does
    /// (`beam_bif_load.c:2222-2326`). Return `false`, with no change, when
    /// no purge of `name` runs. The caller must make sure that no process
    /// has a frame in the code: `erts_code_purger` kills each process that
    /// refers to it first (`erts_code_purger.erl:166-182`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(code.complete_purge("on_load_error"));
    /// assert!(!code.complete_purge("on_load_error"));
    /// ```
    pub(in crate::runtime) fn complete_purge(&mut self, name: &str) -> bool {
        if self.old_code(name).is_none() {
            return false;
        }
        let (_, code) = self.purging.take().expect("checked above");
        self.by_id.remove(&code.id);
        true
    }
}

/// Create the frame for the function at an entry label of `module`.
///
/// # Examples
///
/// ```rust,ignore
/// let frame = frame(&module, 2, 0)?;
/// assert_eq!(module.tables.name(frame.function), "main");
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
            module.name,
            module.tables.name(function.name)
        )));
    }
    Ok(Frame {
        module_id: module.id,
        function: function.name,
        next_instruction: function.start,
        stack: Vec::new(),
    })
}

/// Create the frame for the function that a local-call instruction names.
/// The loader replaces the label of the call with the index of the function
/// in the function table of the module and the index of the instruction at
/// the label, so this reads the table and runs no lookup. The frame starts
/// at `target`, which can be a label inside the function and not only its
/// entry, as the pinned ERTS `i_call` jumps to the code position of the
/// operand label (`erts/emulator/beam/emu/instrs.tab:148-180`).
///
/// # Examples
///
/// ```rust,ignore
/// // [FunctionInfo(main), Label(1), CallOnly { arity: 0, function: 0, target: 2 }]
/// let frame = frame_at(&module, 0, 2, 0)?;
/// assert_eq!(module.tables.name(frame.function), "main");
/// ```
///
/// # Errors
///
/// A function with another arity is the error `call arity does not match
/// M:F`. An index that the function table does not hold is a broken loader
/// invariant, and this function stops the run.
pub(in crate::runtime) fn frame_at(
    module: &BeamModule,
    function: u32,
    target: u32,
    arity: u32,
) -> Result<Frame, BeamError> {
    let (name, function_arity) = module
        .tables
        .functions
        .get(function as usize)
        .expect("the loader resolves a call label to a function");
    if *function_arity != arity {
        return Err(error(format!(
            "call arity does not match {}:{}",
            module.name,
            module.tables.name(*name)
        )));
    }
    Ok(Frame {
        module_id: module.id,
        function: *name,
        next_instruction: target as usize,
        stack: Vec::new(),
    })
}

#[cfg(test)]
#[path = "../../../tests/runtime/interpreter/code_table.rs"]
mod tests;
