//! The NIF manifest gate on the compiled fixtures of `fixtures/nif_gate`:
//! each NIF reference of a module that PBeam does not bind is one item.

use pbeam_runtime::{BeamModule, nif_manifest_gaps};

/// Return the items of the gate for the BEAM file `bytes`.
fn gaps(bytes: &[u8]) -> Vec<String> {
    nif_manifest_gaps(&BeamModule::load(bytes).expect("the fixture loads"))
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn passes_a_module_whose_declared_library_functions_have_bindings() {
    // `-nifs([isatty/1, write_nif/2, extra/0])`: both library functions
    // have a binding, and `extra/0` is not in the library.
    assert!(gaps(include_bytes!("../../fixtures/nif_gate/ok/prim_tty.beam")).is_empty());
}

#[test]
fn reports_each_declared_library_function_without_a_binding_in_library_order() {
    // `-nifs([wcwidth/1, isatty/1, isprint/1])`: the library lists
    // `isprint/1` before `wcwidth/1`.
    assert_eq!(
        gaps(include_bytes!(
            "../../fixtures/nif_gate/missing_binding/prim_tty.beam"
        )),
        [
            "prim_tty:isprint/1: NIF function has no PBeam binding",
            "prim_tty:wcwidth/1: NIF function has no PBeam binding",
        ]
    );
    // Without `-nifs`, each function of the module starts with `nif_start`.
    assert_eq!(
        gaps(include_bytes!(
            "../../fixtures/nif_gate/no_nifs/prim_tty.beam"
        )),
        ["prim_tty:isprint/1: NIF function has no PBeam binding"]
    );
}

#[test]
fn reports_a_library_without_bindings_with_its_path() {
    assert_eq!(
        gaps(include_bytes!(
            "../../fixtures/nif_gate/missing_library/nif_gate_library.beam"
        )),
        ["nif_gate_library: NIF library \"priv/nif_gate_library\" has no PBeam bindings"]
    );
}

#[test]
fn reports_a_library_name_that_is_not_a_literal_string_with_its_module() {
    // `load(Path)` passes its argument, and `load_list()` passes the
    // literal `[priv]`.
    let item = "nif_gate_dynamic: the library name of erlang:load_nif/2 is not a literal string";
    assert_eq!(
        gaps(include_bytes!(
            "../../fixtures/nif_gate/dynamic/nif_gate_dynamic.beam"
        )),
        [item, item]
    );
}

#[test]
fn passes_a_library_name_that_is_not_a_literal_string_of_a_module_with_a_library() {
    // `load(Path)` passes its argument, as `crypto` builds its path at run
    // time. The module selects the PBeam library of `prim_tty`, as ERTS
    // selects a static library (`beam/erl_nif.c:4743-4757`).
    assert!(
        gaps(include_bytes!(
            "../../fixtures/nif_gate/registered_dynamic/prim_tty.beam"
        ))
        .is_empty()
    );
}

#[test]
fn passes_the_windows_only_library_load_of_pubkey_os_cacerts() {
    // The call of `erlang:load_nif/2` runs only on Windows
    // (`pubkey_os_cacerts.erl:265-275`), and the module has no library.
    assert!(
        gaps(include_bytes!(
            "../../fixtures/nif_gate/windows_only/pubkey_os_cacerts.beam"
        ))
        .is_empty()
    );
}
