use pbeam_runtime::BeamProgram;

mod common;

use common::pinned_beam;

#[test]
fn loads_the_pinned_gen_statem_and_gen_modules() {
    // `gen_statem:call/3` has two `raw_raise` instructions (opcode 161),
    // which PBeam skips at load time. Without the skip, the load fails with
    // `unsupported BEAM opcode 161`.
    let gen_statem = pinned_beam("gen_statem");
    let gen_module = pinned_beam("gen");
    BeamProgram::load(&[&gen_statem, &gen_module]).unwrap();
}
