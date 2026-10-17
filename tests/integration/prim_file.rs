use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::Duration,
};

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const FILE_OWNER_PROBE: &[u8] = include_bytes!("../../fixtures/prim_file/file_owner_probe.beam");

/// The time that the reader has to see the end of each FIFO.
const READ_TIME_LIMIT: Duration = Duration::from_secs(10);

#[test]
fn closes_the_file_of_an_owner_that_exits_as_pinned_erts() {
    // `file_owner_probe:main/3` opens two FIFOs for write with
    // `prim_file:open/2` in two processes that exit without a close. ERTS
    // closes each file when its owner exits (`owner_death_callback`,
    // prim_file_nif.c:377-405), and PBeam closes it in the exit of the
    // owner, also when the monitor signal of the resource still waits in
    // the signal queue. The reader of a FIFO gets the end of file only
    // after the last writer closes it, so a file that stays open stops the
    // reader and the run.
    let directory = std::env::temp_dir().join(format!("pbeam-file-owner-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir(&directory).unwrap();
    for name in ["PBeam", "ERTS"] {
        let fifos =
            ["first", "second", "barrier"].map(|fifo| directory.join(format!("{name}-{fifo}")));
        for fifo in &fifos {
            assert!(Command::new("mkfifo").arg(fifo).status().unwrap().success());
        }
        let reader = read_each_to_end(fifos.clone());
        let [first, second, barrier] = fifos.map(|fifo| fifo.to_str().unwrap().to_owned());
        if name == "PBeam" {
            // The program is not `Send`, so the run loads it on its thread.
            let run = thread::spawn(move || {
                let erlang = pinned_beam("erlang");
                let prim_file = pinned_beam("prim_file");
                let program = BeamProgram::load(&[&erlang, &prim_file, FILE_OWNER_PROBE]).unwrap();
                let arguments = [first, second, barrier]
                    .map(|path| program.host_term(Term::binary(path.into_bytes())));
                let normal = Term::Atom(program.atom("normal").unwrap());
                let result = program
                    .execute("file_owner_probe", "main", &arguments)
                    .unwrap();
                result.term == Term::tuple(vec![normal.clone(), normal])
            });
            assert_eq!(reader.recv_timeout(READ_TIME_LIMIT), Ok(()), "{name}");
            assert!(run.join().unwrap(), "{name}");
        } else {
            let eval = format!(
                "io:format(\"~p~n\", [file_owner_probe:main(<<\"{first}\">>, <<\"{second}\">>, <<\"{barrier}\">>)]), halt()."
            );
            let run = thread::spawn(move || {
                Command::new("erl")
                    .args(["-noshell", "-pa", "fixtures/prim_file", "-eval", &eval])
                    .output()
                    .unwrap()
            });
            assert_eq!(reader.recv_timeout(READ_TIME_LIMIT), Ok(()), "{name}");
            let output = run.join().unwrap();
            assert_eq!(
                (output.stdout.as_slice(), output.stderr.as_slice()),
                (b"{normal,normal}\n".as_slice(), b"".as_slice())
            );
        }
    }
    std::fs::remove_dir_all(&directory).unwrap();
}

/// Open each FIFO of `fifos` for read in order on a new thread and read it
/// to its end. Return the channel that gets `()` after the last end.
fn read_each_to_end(fifos: [PathBuf; 3]) -> mpsc::Receiver<()> {
    let (done, receiver) = mpsc::channel();
    thread::spawn(move || {
        for fifo in &fifos {
            read_to_end(fifo);
        }
        let _ = done.send(());
    });
    receiver
}

/// Open the FIFO `fifo` for read, which waits for a writer, and read it
/// until all writers closed it.
fn read_to_end(fifo: &Path) {
    let mut bytes = Vec::new();
    File::open(fifo).unwrap().read_to_end(&mut bytes).unwrap();
}

const PRIM_EXTRAS_PROBE: &[u8] = include_bytes!("../../fixtures/prim_file/prim_extras_probe.beam");

#[test]
fn runs_the_write_and_metadata_nifs_of_prim_file_as_pinned_erts() {
    // `prim_extras_probe:main/1` calls `make_dir/1`, `del_dir/1`,
    // `delete/1`, `rename/2`, `make_link/2`, `make_symlink/2`, `altname/1`,
    // `write_file_info/2,3`, `read_file_info/1,2`, `read_link/1`,
    // `read_link_info/1`, `read_file/1`, `write_file/2`, `get_cwd/0`, and
    // `set_cwd/1`. Each call reaches the NIF of `prim_file_nif.c` that the
    // binding adds, and `unix_prim_file.c` gives the Unix results. The probe
    // returns the `phash2` digest of the result list, so the test compares
    // the exact results of the pinned ERTS and of PBeam on this host. Each
    // run gets its own empty directory, so no result depends on another run.
    let directory = std::env::temp_dir().join(format!("pbeam-prim-extras-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let prim_file = pinned_beam("prim_file");
    let mut digests = Vec::new();
    for name in ["ERTS", "PBeam"] {
        let scratch = directory.join(name);
        std::fs::create_dir_all(&scratch).unwrap();
        let dir = scratch.to_str().unwrap();
        if name == "ERTS" {
            let output = Command::new("erl")
                .args([
                    "-noshell",
                    "-pa",
                    "fixtures/prim_file",
                    "-eval",
                    "io:format(\"~w~n\", [prim_extras_probe:main(init:get_plain_arguments())]), halt().",
                    "-extra",
                    dir,
                ])
                .output()
                .unwrap();
            assert!(output.status.success(), "{name} {:?}", output.stderr);
            digests.push(
                String::from_utf8(output.stdout)
                    .unwrap()
                    .trim()
                    .parse::<u64>()
                    .unwrap(),
            );
        } else {
            let program =
                BeamProgram::load(&[&erlang, &lists, &prim_file, PRIM_EXTRAS_PROBE]).unwrap();
            let argument = Term::list_from(dir.bytes().map(|byte| Term::Integer(byte.into())));
            let argument = program.host_term(Term::list_from([argument]));
            let result = program
                .execute("prim_extras_probe", "main", &[argument])
                .unwrap();
            let Term::Integer(digest) = result.term else {
                panic!("the probe returns the digest");
            };
            digests.push(u64::try_from(digest).unwrap());
        }
    }
    assert_eq!(digests[0], digests[1]);
    std::fs::remove_dir_all(&directory).unwrap();
}
