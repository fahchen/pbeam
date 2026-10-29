//! The `pbeam_runtime` library. It loads BEAM modules and bundles, and runs them with the PBeam
//! interpreter and schedulers.

mod beam;
mod bundle;
mod error;
mod runtime;
mod term;

pub use beam::BeamModule;
pub use bundle::{Blob, Bundle, BundleWriter, Entry};
pub mod embedded;
pub use error::BeamError;
pub use runtime::{
    AllowAll, BeamProgram, Binding, Capability, CapabilityKind, Constraint, Delegation, Job,
    Library, LoadFn, NativeFn, NativeResult, NifContext, NifGap, Operation, Policy,
    RegistrationError, RootAuthority, Schedulers, add_erlexec_path, install_start_dispositions,
    nif_manifest_gaps, register_library,
};
pub use term::{Atom, AtomTable, HostTerm, Map, PortId, ProcessId, Reference, Term, debug_term};
