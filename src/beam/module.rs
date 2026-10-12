use std::collections::HashMap;

use super::code::{self, Instruction, Operand};
use crate::{
    error::{BeamError, error},
    term::{AtomTable, Term},
};

const X0_REGISTER: u32 = 0;

#[derive(Debug)]
pub struct BeamModule {
    pub(crate) name: String,
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
    pub(crate) jump_targets: HashMap<u32, usize>,
    /// The function of each label: the `func_info` of the module that
    /// precedes the label, as the removed `function_at_label` scan found it.
    /// `None` is a label that no such `func_info` precedes. The first label
    /// of a number wins.
    pub(crate) function_starts: HashMap<u32, Option<FunctionStart>>,
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
        let mut jump_targets = HashMap::new();
        let mut function_starts = HashMap::new();
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
