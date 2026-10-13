use std::{
    collections::HashMap,
    hash::{BuildHasherDefault, Hasher},
    sync::atomic::{AtomicU32, Ordering},
};

use super::code::{self, Instruction, Operand};
use crate::{
    error::{BeamError, error},
    term::{AtomTable, Term},
};

const X0_REGISTER: u32 = 0;

/// The FNV-1a 64-bit offset basis (`FNV_OFFSET_BASIS`, the FNV reference).
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// The FNV-1a 64-bit prime (`FNV_PRIME`, the FNV reference).
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The label map of one module. The keys are the label numbers of one module,
/// and the loader owns the key set, so the map does not need a hash that
/// resists a chosen key. The default `RandomState` mixes each key with
/// `SipHash`, and that cost is more than the short scan that the map replaces
/// for a label near the start of a small module. The mixer of [`LabelHasher`]
/// is cheaper for the `u32` keys of the loader.
pub(crate) type LabelMap<Value> = HashMap<u32, Value, BuildHasherDefault<LabelHasher>>;

/// The FNV-1a mixer of [`LabelMap`]. The standard `Hasher::write_u32` sends
/// the bytes of the key to [`Hasher::write`], so each key uses this mixer.
pub(crate) struct LabelHasher(u64);

impl Hasher for LabelHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(FNV_PRIME);
        }
    }
}

impl Default for LabelHasher {
    fn default() -> Self {
        LabelHasher(FNV_OFFSET_BASIS)
    }
}

/// The identity of a loaded module. [`BeamModule::new`] gives each module a
/// number, and the run loop compares two numbers instead of two module names.
/// The loader owns the number for the whole life of the module, so a frame
/// that names a module stays valid in each run that shares the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ModuleId(u32);

impl ModuleId {
    /// Return the number of a new module, which no other module of this
    /// runtime has.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_ne!(ModuleId::next(), ModuleId::next());
    /// ```
    pub(crate) fn next() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        ModuleId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// The identity of one function name of a module. [`BeamModule::new`] gives
/// each name of the module a number, and a frame, an export key, and a
/// diagnostic use the number instead of the name. The loader owns the number
/// for the whole life of the module, so a frame that names a function stays
/// valid in each run that shares the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NameId(u32);

/// The function names of one module and their identities. Each holder keeps
/// only a [`NameId`], so a name is not copied for each use.
#[derive(Debug, Default)]
struct NameTable {
    /// The name of each [`NameId`], in first-use order.
    names: Vec<String>,
    /// The identity of each name, for the lookup of an export by name.
    ids: HashMap<String, NameId>,
}

impl NameTable {
    /// Return the identity of `name`, and add the name when the module does
    /// not hold it yet.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut names = NameTable::default();
    /// assert_eq!(names.intern("main"), names.intern("main"));
    /// assert_ne!(names.intern("main"), names.intern("other"));
    /// ```
    fn intern(&mut self, name: &str) -> NameId {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let id =
            NameId(u32::try_from(self.names.len()).expect("a module has fewer than 2^32 names"));
        self.names.push(name.to_owned());
        self.ids.insert(name.to_owned(), id);
        id
    }

    /// Return the identity of `name`, or `None` when the module has no
    /// function of that name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut names = NameTable::default();
    /// assert_eq!(names.get("main"), None);
    /// assert_eq!(names.get("main"), Some(names.intern("main")));
    /// ```
    fn get(&self, name: &str) -> Option<NameId> {
        self.ids.get(name).copied()
    }

    /// Return the name of `id`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut names = NameTable::default();
    /// assert_eq!(names.name(names.intern("main")), "main");
    /// ```
    fn name(&self, id: NameId) -> &str {
        &self.names[id.0 as usize]
    }
}

#[derive(Debug)]
pub struct BeamModule {
    pub(crate) name: String,
    /// The identity of the module, for the run loop.
    pub(crate) id: ModuleId,
    /// The entry label of each exported function, by the identity of its
    /// name and its arity. The name itself stays in the name table of the
    /// module tables.
    pub(crate) exports: HashMap<(NameId, u32), u32>,
    pub(crate) instructions: Vec<Instruction>,
    /// The side tables of `instructions`, which the loader builds once (see
    /// [`ModuleTables`]).
    pub(crate) tables: ModuleTables,
}

/// The side tables that the loader builds from the instruction stream of one
/// module, so that a jump and a frame creation read a table instead of
/// scanning the stream. The runtime holds them with the code, and they do not
/// change while the module runs. ERTS keeps the same facts in the loaded code
/// (`erts_lookup_function_info`, `beam_common.c:1205-1292`).
#[derive(Debug)]
pub(crate) struct ModuleTables {
    /// The index of the instruction after each label, as the removed
    /// `label_index` scan gave it. The first label of a number wins, as
    /// `Iterator::position` decided.
    pub(crate) jump_targets: LabelMap<usize>,
    /// The function of each label: the `func_info` of the module that
    /// precedes the label, as the removed `function_at_label` scan found it.
    /// `None` is a label that no such `func_info` precedes. The first label
    /// of a number wins.
    pub(crate) function_starts: LabelMap<Option<FunctionStart>>,
    /// The function names of the module and their identities.
    names: NameTable,
    /// The name and the arity of each function of the module, in stream
    /// order. A `FunctionStart` names its entry in this vector.
    pub(crate) functions: Vec<(NameId, u32)>,
}

/// The function that a label starts.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FunctionStart {
    /// The index of the function in [`ModuleTables::functions`].
    pub(crate) function: usize,
    /// The index of the first instruction after the label.
    pub(crate) start: usize,
}

impl ModuleTables {
    /// Return the identity of the function name `name`, or `None` when the
    /// module has no function of that name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // [FunctionInfo(fixture, main, 0), Label(1), Return]
    /// assert_eq!(tables.name(tables.name_id("main").unwrap()), "main");
    /// assert_eq!(tables.name_id("missing"), None);
    /// ```
    pub(crate) fn name_id(&self, name: &str) -> Option<NameId> {
        self.names.get(name)
    }

    /// Return the name of the function `id`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(tables.name(tables.name_id("main").unwrap()), "main");
    /// ```
    pub(crate) fn name(&self, id: NameId) -> &str {
        self.names.name(id)
    }

    /// Build the side tables of the instruction stream of the module `name`,
    /// and replace each branch label and each call label in the stream with
    /// the code position that the tables give. The loader reads the stream
    /// once, so a later jump and a later frame creation read one field and
    /// run no lookup. `names` holds the identities of the names that an
    /// earlier build of the same module gave, so a rebuild keeps the
    /// identity of a name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // [FunctionInfo, Label(1), Label(2), Return]
    /// let mut instructions = vec![
    ///     Instruction::FunctionInfo { module: "fixture".into(), name: "main".into(), arity: 0 },
    ///     Instruction::Label(1),
    ///     Instruction::Label(2),
    ///     Instruction::Return,
    /// ];
    /// let tables = ModuleTables::build("fixture", &mut instructions, NameTable::default()).unwrap();
    /// assert_eq!(tables.jump_targets.get(&2), Some(&3));
    /// assert_eq!(tables.name(tables.functions[0].0), "main");
    /// ```
    fn build(
        name: &str,
        instructions: &mut [Instruction],
        mut names: NameTable,
    ) -> Result<Self, BeamError> {
        let mut jump_targets = LabelMap::default();
        let mut function_starts = LabelMap::default();
        let mut functions: Vec<(NameId, u32)> = Vec::new();
        let mut function: Option<usize> = None;
        for (index, instruction) in instructions.iter().enumerate() {
            match instruction {
                Instruction::FunctionInfo {
                    module,
                    name: function_name,
                    arity,
                } if module == name => {
                    functions.push((names.intern(function_name), *arity));
                    function = Some(functions.len() - 1);
                }
                Instruction::FunctionInfo { .. } => function = None,
                Instruction::Label(label) => {
                    jump_targets.entry(*label).or_insert(index + 1);
                    function_starts
                        .entry(*label)
                        .or_insert(function.map(|function| FunctionStart {
                            function,
                            start: index + 1,
                        }));
                }
                _ => {}
            }
        }
        let tables = ModuleTables {
            jump_targets,
            function_starts,
            names,
            functions,
        };
        for instruction in instructions.iter_mut() {
            resolve(name, instruction, &tables)?;
        }
        Ok(tables)
    }
}

/// Replace the label of one instruction with the code position that the
/// loader tables give. A branch label becomes the index of the instruction
/// after the label. A call label becomes the index of the function in
/// [`ModuleTables::functions`] and the index of the instruction at the
/// label, so that a call starts where its label is and not at the entry of
/// the function. A label that no table holds keeps the error text of the
/// removed lookup, and the module does not load.
///
/// # Examples
///
/// ```rust,ignore
/// // [Jump(1)] with label 1 at index 3 gives Jump(4).
/// resolve("fixture", &mut jump, &tables)?;
/// assert_eq!(jump, Instruction::Jump(4));
/// ```
fn resolve(
    name: &str,
    instruction: &mut Instruction,
    tables: &ModuleTables,
) -> Result<(), BeamError> {
    let jump_targets = &tables.jump_targets;
    let function_starts = &tables.function_starts;
    match instruction {
        Instruction::Call {
            function, target, ..
        }
        | Instruction::CallLast {
            function, target, ..
        }
        | Instruction::CallOnly {
            function, target, ..
        } => {
            let label = *target;
            match function_starts.get(&label) {
                Some(Some(entry)) => {
                    *function = entry.function as u32;
                    *target = entry.start as u32;
                }
                Some(None) => {
                    return Err(error(format!("entry label {label} has no function info")));
                }
                None => {
                    return Err(error(format!("entry label {label} not found in {name}")));
                }
            }
        }
        Instruction::Jump(target)
        | Instruction::LoopReceiveEnd(target)
        | Instruction::Wait(target) => resolve_branch(target, jump_targets)?,
        Instruction::Catch { label, .. } | Instruction::WaitTimeout { label, .. } => {
            resolve_branch(label, jump_targets)?
        }
        Instruction::Bif { fail_label, .. }
        | Instruction::Test { fail_label, .. }
        | Instruction::TestFunctionArity { fail_label, .. }
        | Instruction::TestArity { fail_label, .. }
        | Instruction::GetMapElements { fail_label, .. }
        | Instruction::HasMapFields { fail_label, .. }
        | Instruction::PutMapExact { fail_label, .. }
        | Instruction::TestExactEqual { fail_label, .. }
        | Instruction::TestExactEqualOperands { fail_label, .. }
        | Instruction::TestEqual { fail_label, .. }
        | Instruction::TestNotEqual { fail_label, .. }
        | Instruction::TestExactNotEqual { fail_label, .. }
        | Instruction::TestLess { fail_label, .. }
        | Instruction::TestGreaterEqual { fail_label, .. }
        | Instruction::TestTaggedTuple { fail_label, .. }
        | Instruction::BinaryMatchStart { fail_label, .. }
        | Instruction::BinaryGetBinary { fail_label, .. }
        | Instruction::BinaryGetUtf8 { fail_label, .. }
        | Instruction::BinaryMatch { fail_label, .. }
        | Instruction::BinaryCreate { fail_label, .. }
        | Instruction::LoopReceive { fail_label, .. } => resolve_branch(fail_label, jump_targets)?,
        Instruction::SelectTupleArity {
            fail_label,
            choices,
            ..
        } => {
            resolve_branch(fail_label, jump_targets)?;
            for (_, target) in choices {
                resolve_branch(target, jump_targets)?;
            }
        }
        Instruction::SelectValue {
            fail_label,
            choices,
            ..
        } => {
            resolve_branch(fail_label, jump_targets)?;
            for (_, target) in choices {
                resolve_branch(target, jump_targets)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Replace one branch label with the index of the instruction after the
/// label. A target of 0 is the "no fail label" operand of a BIF, and it
/// stays 0 for the arm that handles it.
///
/// # Examples
///
/// ```rust,ignore
/// let mut target = 5;
/// resolve_branch(&mut target, &targets)?;
/// assert_eq!(target, 9);
/// ```
///
/// # Errors
///
/// A label that the module does not hold is the error `missing BEAM label
/// L`.
fn resolve_branch(target: &mut u32, jump_targets: &LabelMap<usize>) -> Result<(), BeamError> {
    if *target == 0 {
        return Ok(());
    }
    let label = *target;
    let index = jump_targets
        .get(&label)
        .copied()
        .ok_or_else(|| error(format!("missing BEAM label {label}")))?;
    // A BEAM `Code` chunk is at most 2^32 - 1 bytes, so its instruction
    // index is below 2^32 and the cast keeps every value.
    *target = index as u32;
    Ok(())
}

impl BeamModule {
    /// Return the module `name` with `exports` and the decoded
    /// `instructions`, and build the side tables of the stream. Each export
    /// name goes into the name table of the module, so its name lives in one
    /// place. Each branch label and each call label becomes the resolved
    /// code position; a label that does not resolve is an error.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let module = BeamModule::new("fixture".into(), HashMap::new(), vec![Instruction::End])?;
    /// assert!(module.tables.jump_targets.is_empty());
    /// ```
    pub(crate) fn new(
        name: String,
        exports: HashMap<(String, u32), u32>,
        mut instructions: Vec<Instruction>,
    ) -> Result<Self, BeamError> {
        let mut tables = ModuleTables::build(&name, &mut instructions, NameTable::default())?;
        let exports = exports
            .into_iter()
            .map(|((function, arity), label)| ((tables.names.intern(&function), arity), label))
            .collect();
        Ok(BeamModule {
            name,
            id: ModuleId::next(),
            exports,
            instructions,
            tables,
        })
    }

    /// Parse one BEAM file into a module, with a new atom table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let module = BeamModule::load(CONSTANT)?;
    /// assert!(BeamModule::load(b"not beam").is_err());
    /// ```
    pub fn load(bytes: &[u8]) -> Result<Self, BeamError> {
        code::parse(bytes, &mut AtomTable::new())
    }

    /// Run an exported function that returns a constant from `x0`. It runs
    /// no scheduler, so the `TestYield` of the entry does nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let module = BeamModule::load(CONSTANT)?;
    /// assert_eq!(module.execute("number", &[])?, Term::Integer(7));
    /// ```
    pub fn execute(&self, function: &str, arguments: &[Term]) -> Result<Term, BeamError> {
        let arity = checked_arity(arguments.len())?;
        let entry = self
            .tables
            .name_id(function)
            .and_then(|id| self.exports.get(&(id, arity)))
            .ok_or_else(|| error(format!("missing export {}:{function}/{arity}", self.name)))?;
        let mut active = false;
        let mut label_matches = false;
        let mut value = None;
        for instruction in &self.instructions {
            match instruction {
                Instruction::FunctionInfo {
                    module,
                    name,
                    arity: found,
                } => {
                    active = module == &self.name && name == function && *found == arity;
                    label_matches = false;
                }
                Instruction::Label(label) if active && label == entry => label_matches = true,
                Instruction::Label(_) | Instruction::TestYield if active => {}
                Instruction::Move(Operand::Value(term), Operand::X(X0_REGISTER))
                    if active && label_matches && value.is_none() =>
                {
                    value = Some(term.clone());
                }
                Instruction::Return if active && label_matches => {
                    return value.ok_or_else(|| error("return before setting x0"));
                }
                Instruction::End => break,
                _ if active && label_matches => return Err(error("unsupported entry instruction")),
                _ => {}
            }
        }
        Err(error(format!(
            "entry label {entry} not found for {}:{function}/{arity}",
            self.name
        )))
    }
}

/// Convert an argument count to a BEAM arity.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(checked_arity(2)?, 2);
/// assert!(checked_arity(usize::MAX).is_err());
/// ```
fn checked_arity(argument_count: usize) -> Result<u32, BeamError> {
    u32::try_from(argument_count).map_err(|_| error("arity exceeds BEAM limit"))
}

#[cfg(test)]
#[path = "../../tests/beam/module.rs"]
mod tests;
