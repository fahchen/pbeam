use std::mem::{offset_of, size_of};
use std::process::Command;

use super::*;

/// The C program that prints the size of `ERL_NIF_TERM`, `ErlNifFunc`,
/// `ErlNifEntry`, `ErlNifBinary`, and `ErlNifPid`, and the offset of each field, one value for each line.
const PROGRAM: &str = r#"
#include <stddef.h>
#include <stdio.h>
#include "erl_nif.h"

int main(void)
{
    printf("%zu\n", sizeof(ERL_NIF_TERM));
    printf("%zu\n", sizeof(ErlNifFunc));
    printf("%zu\n", offsetof(ErlNifFunc, name));
    printf("%zu\n", offsetof(ErlNifFunc, arity));
    printf("%zu\n", offsetof(ErlNifFunc, fptr));
    printf("%zu\n", offsetof(ErlNifFunc, flags));
    printf("%zu\n", sizeof(ErlNifEntry));
    printf("%zu\n", offsetof(ErlNifEntry, major));
    printf("%zu\n", offsetof(ErlNifEntry, minor));
    printf("%zu\n", offsetof(ErlNifEntry, name));
    printf("%zu\n", offsetof(ErlNifEntry, num_of_funcs));
    printf("%zu\n", offsetof(ErlNifEntry, funcs));
    printf("%zu\n", offsetof(ErlNifEntry, load));
    printf("%zu\n", offsetof(ErlNifEntry, reload));
    printf("%zu\n", offsetof(ErlNifEntry, upgrade));
    printf("%zu\n", offsetof(ErlNifEntry, unload));
    printf("%zu\n", offsetof(ErlNifEntry, vm_variant));
    printf("%zu\n", offsetof(ErlNifEntry, options));
    printf("%zu\n", offsetof(ErlNifEntry, sizeof_ErlNifResourceTypeInit));
    printf("%zu\n", offsetof(ErlNifEntry, min_erts));
    printf("%zu\n", sizeof(ErlNifBinary));
    printf("%zu\n", offsetof(ErlNifBinary, size));
    printf("%zu\n", offsetof(ErlNifBinary, data));
    printf("%zu\n", offsetof(ErlNifBinary, ref_bin));
    printf("%zu\n", offsetof(ErlNifBinary, __spare__));
    printf("%zu\n", sizeof(ErlNifPid));
    printf("%zu\n", sizeof(ErlNifMonitor));
    printf("%zu\n", sizeof(ErlNifPort));
    printf("%zu\n", sizeof(ErlNifMapIterator));
    printf("%zu\n", sizeof(ErlNifResourceTypeInit));
    printf("%zu\n", offsetof(ErlNifResourceTypeInit, dtor));
    printf("%zu\n", offsetof(ErlNifResourceTypeInit, stop));
    printf("%zu\n", offsetof(ErlNifResourceTypeInit, down));
    printf("%zu\n", offsetof(ErlNifResourceTypeInit, members));
    printf("%zu\n", offsetof(ErlNifResourceTypeInit, dyncall));
    return 0;
}
"#;

#[test]
fn mirrors_the_layout_of_the_vendored_erl_nif_header() {
    let directory = std::env::temp_dir().join(format!("pbeam-nif-layout-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("layout.c");
    let program = directory.join("layout");
    std::fs::write(&source, PROGRAM).unwrap();
    let include = concat!("-I", env!("CARGO_MANIFEST_DIR"), "/vendor/erts/include");
    let built = Command::new("cc")
        .arg(include)
        .arg("-o")
        .arg(&program)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let output = Command::new(&program).output().unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
    let values: Vec<usize> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| line.parse().unwrap())
        .collect();
    assert_eq!(
        values,
        [
            size_of::<NifTerm>(),
            size_of::<ErlNifFunc>(),
            offset_of!(ErlNifFunc, name),
            offset_of!(ErlNifFunc, arity),
            offset_of!(ErlNifFunc, fptr),
            offset_of!(ErlNifFunc, flags),
            size_of::<ErlNifEntry>(),
            offset_of!(ErlNifEntry, major),
            offset_of!(ErlNifEntry, minor),
            offset_of!(ErlNifEntry, name),
            offset_of!(ErlNifEntry, num_of_funcs),
            offset_of!(ErlNifEntry, funcs),
            offset_of!(ErlNifEntry, load),
            offset_of!(ErlNifEntry, reload),
            offset_of!(ErlNifEntry, upgrade),
            offset_of!(ErlNifEntry, unload),
            offset_of!(ErlNifEntry, vm_variant),
            offset_of!(ErlNifEntry, options),
            offset_of!(ErlNifEntry, sizeof_resource_type_init),
            offset_of!(ErlNifEntry, min_erts),
            size_of::<ErlNifBinary>(),
            offset_of!(ErlNifBinary, size),
            offset_of!(ErlNifBinary, data),
            offset_of!(ErlNifBinary, ref_bin),
            offset_of!(ErlNifBinary, spare),
            size_of::<ErlNifPid>(),
            size_of::<ErlNifMonitor>(),
            size_of::<ErlNifPort>(),
            size_of::<ErlNifMapIterator>(),
            size_of::<ErlNifResourceTypeInit>(),
            offset_of!(ErlNifResourceTypeInit, dtor),
            offset_of!(ErlNifResourceTypeInit, stop),
            offset_of!(ErlNifResourceTypeInit, down),
            offset_of!(ErlNifResourceTypeInit, members),
            offset_of!(ErlNifResourceTypeInit, dyncall),
        ]
    );
}
