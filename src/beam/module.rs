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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug)]
pub struct BeamModule {
    pub(crate) name: String,
    /// The identity of the module, for the run loop.
    pub(crate) id: ModuleId,
    pub(crate) exports: HashMap<(String, u32), u32>,
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
    /// The name and the arity of each function of the module, in stream
    /// order. A `FunctionStart` names its entry in this vector.
    pub(crate) functions: Vec<(String, u32)>,
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
    /// Build the side tables of the instruction stream of the module `name`.
    /// The loader reads the stream once, so a later jump and a later frame
    /// creation read one table entry each.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // [FunctionInfo, Label(1), Label(2), Return]
    /// let tables = ModuleTables::build("fixture", &instructions);
    /// assert_eq!(tables.jump_targets.get(&2), Some(&3));
    /// assert_eq!(tables.functions[0].0, "main");
    /// ```
    fn build(name: &str, instructions: &[Instruction]) -> Self {
        let mut jump_targets = LabelMap::default();
        let mut function_starts = LabelMap::default();
        let mut functions = Vec::new();
        let mut function: Option<usize> = None;
        for (index, instruction) in instructions.iter().enumerate() {
            match instruction {
                Instruction::FunctionInfo {
                    module,
                    name: function_name,
                    arity,
                } if module == name => {
                    functions.push((function_name.clone(), *arity));
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
        ModuleTables {
            jump_targets,
            function_starts,
            functions,
        }
    }
}

impl BeamModule {
    /// Return the module `name` with `exports` and the decoded
    /// `instructions`, and build the side tables of the stream.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let module = BeamModule::new("fixture".into(), HashMap::new(), vec![Instruction::End]);
    /// assert!(module.tables.jump_targets.is_empty());
    /// ```
    pub(crate) fn new(
        name: String,
        exports: HashMap<(String, u32), u32>,
        instructions: Vec<Instruction>,
    ) -> Self {
        let tables = ModuleTables::build(&name, &instructions);
        BeamModule {
            name,
            id: ModuleId::next(),
            exports,
            instructions,
            tables,
        }
    }

    /// Rebuild the side tables from the current `instructions`. Only a test
    /// that changes the instructions of a module after its construction needs
    /// this.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// module.instructions.push(Instruction::End);
    /// module.reindex();
    /// ```
    #[cfg(test)]
    pub(crate) fn reindex(&mut self) {
        self.tables = ModuleTables::build(&self.name, &self.instructions);
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
            .exports
            .get(&(function.to_owned(), arity))
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
