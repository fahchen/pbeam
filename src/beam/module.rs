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
}

impl BeamModule {
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
