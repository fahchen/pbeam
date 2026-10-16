use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const FILE_OWNER_PROBE: &[u8] = include_bytes!("../../fixtures/prim_file/file_owner_probe.beam");

#[test]
fn ends_the_owner_of_an_open_file_resource_as_pinned_erts() {
    // `file_owner_probe:main/1` opens a file with `prim_file:open/2` in two
    // processes that exit without a close. ERTS closes each file when its
    // owner exits (`owner_death_callback`, prim_file_nif.c:377-405), and
    // PBeam closes it in the exit of the owner, also when the monitor
    // signal of the resource still waits in the signal queue.
    let directory = std::env::temp_dir().join(format!("pbeam-file-owner-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("file");
    let path_text = path.to_str().unwrap();
    let erlang = pinned_beam("erlang");
    let prim_file = pinned_beam("prim_file");
    let program = BeamProgram::load(&[&erlang, &prim_file, FILE_OWNER_PROBE]).unwrap();
    let normal = Term::Atom(program.atom("normal").unwrap());
    assert_eq!(
        program
            .execute(
                "file_owner_probe",
                "main",
                &[program.host_term(Term::binary(path_text.as_bytes().to_vec()))]
            )
            .unwrap()
            .term,
        Term::tuple(vec![normal.clone(), normal])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/prim_file",
            "-eval",
            &format!("io:format(\"~p~n\", [file_owner_probe:main(<<\"{path_text}\">>)]), halt()."),
        ])
        .output()
        .unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
    assert_eq!(
        (reference.stdout.as_slice(), reference.stderr.as_slice()),
        (b"{normal,normal}\n".as_slice(), b"".as_slice())
    );
}
