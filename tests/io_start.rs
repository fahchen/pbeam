use pbeam_runtime::BeamProgram;

mod common;

use common::pinned_beam;

#[test]
fn loads_the_pinned_io_lib_modules() {
    // `user_drv:init_shell/2` (user_drv.erl:374) calls `io_lib:bformat/2`,
    // which runs `io_lib_format`, `lists`, and `unicode`.
    // `prim_tty:init_term/1` (prim_tty.erl:284) calls `os:type/0`. The pinned
    // `io_lib` has `bs_get_integer2/7` (opcode 117), `gc_bif3/7` (opcode 152),
    // and `is_any_native_record/2` (opcode 186), which the start path does
    // not run.
    let modules = ["io_lib", "io_lib_format", "lists", "unicode", "os"].map(pinned_beam);
    let beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    BeamProgram::load(&beams).unwrap();
}
