pub(crate) mod attributes;
pub(crate) mod code;
mod module;

pub use module::BeamModule;
pub(crate) use module::{Import, ImportKind, ModuleId, NameId};
