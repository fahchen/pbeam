use super::*;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::process::tests::frame;
use crate::term::debug_term;

/// Return the `Debug` text of a BIF result, with each atom name from `atoms`.
///
/// `Debug for Term` writes the atom index, because a bare term carries no
/// table. The atom names of a BIF result come from the run, so this test
/// reads them through the table of that run.
fn debug_bif(result: &BifResult, atoms: &AtomTable) -> String {
    match result {
        BifResult::Value(term) => format!("Value({})", debug_term(term, atoms)),
        BifResult::Raise { class, reason } => format!(
            "Raise {{ class: {class:?}, reason: {} }}",
            debug_term(reason, atoms)
        ),
        other => format!("{other:?}"),
    }
}

/// Return the result of a call: for the dirty work of a binding, run its
/// host part on this thread, as a dirty IO thread does, and then its finish
/// on `scheduler`.
fn run(scheduler: &mut Scheduler, result: Result<BifResult, BeamError>) -> BifResult {
    match result.unwrap() {
        BifResult::Dirty(Work::Host(Host(host))) => {
            let finish = host();
            finish(scheduler).unwrap()
        }
        result => result,
    }
}

/// Return a scheduler whose current process is a new root process.
fn scheduler() -> Scheduler {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler
        .start(frame(), vec![], AllowAll.initial())
        .unwrap();
    scheduler.schedule_next().unwrap();
    scheduler
}

#[test]
fn checks_the_follow_flag_and_then_the_path_as_erts() {
    // `prim_file` does not export `read_info_nif/2`, and its only caller,
    // `read_info_1/3`, gives the bytes of `internal_name2native/1` and the
    // flag 0 or 1 (prim_file.erl:657-664). Thus no BEAM code reaches these
    // checks of `read_info_nif` (prim_file_nif.c:1019-1038): `enif_get_int`
    // first, then `efile_marshal_path` with `has_invalid_null_termination`
    // (unix_prim_file.c:71-101).
    let mut scheduler = scheduler();
    let path = Term::binary(b"/\0".to_vec());
    for (path, follow, expected) in [
        (
            path.clone(),
            Term::Integer(1 << 31),
            "Raise { class: Error, reason: Atom(\"badarg\") }",
        ),
        (
            path.clone(),
            Term::Integer(-(1 << 31) - 1),
            "Raise { class: Error, reason: Atom(\"badarg\") }",
        ),
        (
            Term::Integer(1),
            Term::static_atom("true"),
            "Raise { class: Error, reason: Atom(\"badarg\") }",
        ),
        (
            Term::list_from(vec![Term::Integer(47), Term::Integer(0)]),
            Term::Integer(1),
            "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))",
        ),
        (
            Term::binary(vec![]),
            Term::Integer(1),
            "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))",
        ),
        (
            Term::binary(b"/".to_vec()),
            Term::Integer(1),
            "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))",
        ),
        (
            Term::binary(b"/\0a\0".to_vec()),
            Term::Integer(1),
            "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))",
        ),
        (
            Term::binary(b"/pbeam-missing\0\0".to_vec()),
            Term::Integer((1 << 31) - 1),
            "Value(Tuple([Atom(\"error\"), Atom(\"enoent\")]))",
        ),
        (
            Term::binary(b"\0".to_vec()),
            Term::Integer(-(1 << 31)),
            "Value(Tuple([Atom(\"error\"), Atom(\"enoent\")]))",
        ),
    ] {
        let result = run(&mut scheduler, read_info_nif(&path, &follow));
        assert_eq!(
            debug_bif(&result, &scheduler.atoms),
            expected,
            "{path:?} {follow:?}"
        );
    }
    let result = run(&mut scheduler, read_info_nif(&path, &Term::Integer(0)));
    assert!(
        matches!(&result, BifResult::Value(Term::Tuple(fields)) if fields.len() == 14
            && matches!(&fields[2], Term::Atom(atom) if scheduler.atoms.name_of(atom) == "directory")),
        "{result:?}"
    );
}

#[test]
fn gives_einval_for_a_path_that_is_not_marshaled_as_erts() {
    // `prim_file` does not export the NIFs, and its callers give the bytes
    // of `internal_name2native/1` (prim_file.erl:564-568, 593-594, 611-612),
    // which always end with one NUL. Thus no BEAM code reaches the
    // `efile_marshal_path` error of `read_file_nif`, `list_dir_nif`, and
    // `read_link_nif` (prim_file_nif.c:1115-1147, 1354-1366).
    let mut scheduler = scheduler();
    let einval = "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))";
    for name in ["read_file_nif", "list_dir_nif", "read_link_nif"] {
        let binding = BINDINGS
            .iter()
            .find(|binding| (binding.name, binding.arity) == (name, 1))
            .unwrap();
        for path in [
            Term::binary(b"/".to_vec()),
            Term::binary(b"/\0a\0".to_vec()),
            Term::list_from(vec![Term::Integer(47), Term::Integer(0)]),
        ] {
            let result = (binding.run)(&mut scheduler, std::slice::from_ref(&path)).unwrap();
            assert_eq!(
                debug_bif(&result, &scheduler.atoms),
                einval,
                "{name} {path:?}"
            );
        }
    }
}

/// Return a new empty scratch directory for the test `name`.
fn scratch(name: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("pbeam-prim-file-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir(&directory).unwrap();
    directory
}

/// Return the `Path` argument of a NIF for `path`: its bytes and a NUL.
fn nif_path(path: &std::path::Path) -> Term {
    let mut bytes = path.as_os_str().as_encoded_bytes().to_vec();
    bytes.push(0);
    Term::binary(bytes)
}

/// Return the list of the atoms `names`.
fn atoms(scheduler: &mut Scheduler, names: &[&str]) -> Term {
    Term::list_from(
        names
            .iter()
            .map(|name| Term::Atom(scheduler.atoms.put(name).unwrap())),
    )
}

/// Open `path` with the mode atoms `modes`, and return the file resource.
fn open(scheduler: &mut Scheduler, path: &std::path::Path, modes: &[&str]) -> Term {
    let modes = atoms(scheduler, modes);
    let result = open_nif(scheduler, &nif_path(path), &modes);
    let result = run(scheduler, result);
    resource(&result)
}

/// Return the file resource of the result `{ok, Ref}` of `open_nif/2`.
fn resource(result: &BifResult) -> Term {
    let BifResult::Value(Term::Tuple(result)) = result else {
        panic!("the open gives a tuple");
    };
    assert!(matches!(result[1], Term::Reference(_)));
    result[1].clone()
}

/// Return the `Debug` text of the result of `run` on `scheduler`.
fn text(
    scheduler: &mut Scheduler,
    run: impl FnOnce(&mut Scheduler) -> Result<BifResult, BeamError>,
) -> String {
    let result = run(scheduler);
    let result = self::run(scheduler, result);
    debug_bif(&result, &scheduler.atoms)
}

const BADARG: &str = "Raise { class: Error, reason: Atom(\"badarg\") }";

#[test]
fn opens_files_with_the_modes_and_errors_of_erts() {
    // `prim_file:open/2` gives each mode list and each encoded path to
    // `open_nif/2` and catches `error:badarg` (prim_file.erl:146-155), so
    // these results come from `open_nif` (prim_file_nif.c:500-593) and
    // `efile_open` (unix_prim_file.c:142-226).
    let mut scheduler = scheduler();
    let directory = scratch("open");
    let file = directory.join("file");
    std::fs::write(&file, b"ab").unwrap();
    let path = nif_path(&file);
    let read = atoms(&mut scheduler, &["read"]);
    assert_eq!(
        text(&mut scheduler, |s| open_nif(
            s,
            &path,
            &Term::static_atom("ok")
        )),
        BADARG
    );
    assert_eq!(
        text(&mut scheduler, |s| open_nif(
            s,
            &Term::binary(b"a".to_vec()),
            &read
        )),
        "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))"
    );
    for (target, modes, expected) in [
        (
            directory.join("missing"),
            vec!["read"],
            "Value(Tuple([Atom(\"error\"), Atom(\"enoent\")]))",
        ),
        (
            directory.clone(),
            vec!["read"],
            "Value(Tuple([Atom(\"error\"), Atom(\"eisdir\")]))",
        ),
        (
            file.clone(),
            vec!["exclusive"],
            "Value(Tuple([Atom(\"error\"), Atom(\"eexist\")]))",
        ),
        (
            file.clone(),
            vec!["directory"],
            "Value(Tuple([Atom(\"error\"), Atom(\"enotdir\")]))",
        ),
        (
            file.join("x"),
            vec!["write"],
            "Value(Tuple([Atom(\"error\"), Atom(\"enotdir\")]))",
        ),
    ] {
        let modes = atoms(&mut scheduler, &modes);
        let target = nif_path(&target);
        assert_eq!(
            text(&mut scheduler, |s| open_nif(s, &target, &modes)),
            expected
        );
    }
    open(&mut scheduler, &directory, &["directory"]);
    open(&mut scheduler, &directory, &["skip_type_check"]);
    open(&mut scheduler, &directory.join("new"), &["exclusive"]);
    open(&mut scheduler, &file, &["read", "write", "append", "sync"]);
    // Each element that is not a mode atom is ignored, also the tail of an
    // improper list, and a list with no mode reads.
    let ignored = Term::list_with_tail(
        vec![Term::Integer(1), Term::static_atom("ok")],
        Term::Integer(2),
    );
    let opened = open_nif(&mut scheduler, &path, &ignored);
    let fd = resource(&run(&mut scheduler, opened));
    let size = Term::Integer(10);
    assert_eq!(
        text(&mut scheduler, |s| read_nif(s, &fd, &size)),
        "Value(Tuple([Atom(\"ok\"), Binary([97, 98])]))"
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn reads_writes_seeks_and_closes_a_file_resource_as_erts() {
    // `read_nif_impl`, `write_nif_impl`, and `seek_nif_impl`
    // (prim_file_nif.c:649-710, 782-806) after `file_handle_wrapper`
    // (prim_file_nif.c:315-357), and `close_nif` (prim_file_nif.c:618-647).
    let mut scheduler = scheduler();
    let directory = scratch("handle");
    let file = directory.join("file");
    let fd = open(&mut scheduler, &file, &["read", "write"]);
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let iovec = Term::list_from(vec![binary(b"ab"), binary(b""), binary(b"cd")]);
    assert_eq!(
        text(&mut scheduler, |s| write_nif(s, &fd, &iovec)),
        "Value(Atom(\"ok\"))"
    );
    let long = Term::list_from(std::iter::repeat_n(binary(b"z"), 65));
    assert_eq!(
        text(&mut scheduler, |s| write_nif(s, &fd, &long)),
        "Value(Tuple([Atom(\"continue\"), List([Binary([122])])]))"
    );
    for bad in [
        binary(b"x"),
        Term::list_from(vec![Term::Integer(1)]),
        Term::list_with_tail(vec![binary(b"x")], binary(b"y")),
    ] {
        assert_eq!(text(&mut scheduler, |s| write_nif(s, &fd, &bad)), BADARG);
    }
    let mark =
        |scheduler: &mut Scheduler, name: &str| Term::Atom(scheduler.atoms.put(name).unwrap());
    let (bof, cur, eof) = (
        mark(&mut scheduler, "bof"),
        mark(&mut scheduler, "cur"),
        mark(&mut scheduler, "eof"),
    );
    for (mark, offset, expected) in [
        (
            eof.clone(),
            Term::Integer(0),
            "Value(Tuple([Atom(\"ok\"), Integer(68)]))",
        ),
        (
            cur.clone(),
            Term::Integer(-1),
            "Value(Tuple([Atom(\"ok\"), Integer(67)]))",
        ),
        (
            bof.clone(),
            Term::Integer(-1),
            "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))",
        ),
        (bof.clone(), Term::static_atom("ok"), BADARG),
        (Term::Integer(1), Term::Integer(0), BADARG),
        (Term::static_atom("ok"), Term::Integer(0), BADARG),
        (
            bof.clone(),
            Term::Integer(1),
            "Value(Tuple([Atom(\"ok\"), Integer(1)]))",
        ),
    ] {
        assert_eq!(
            text(&mut scheduler, |s| seek_nif(s, &fd, &mark, &offset)),
            expected
        );
    }
    for (size, expected) in [
        (
            Term::Integer(2),
            "Value(Tuple([Atom(\"ok\"), Binary([98, 99])]))",
        ),
        (Term::Integer(0), "Value(Atom(\"eof\"))"),
        (Term::static_atom("ok"), BADARG),
        (
            Term::Integer(-1),
            "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))",
        ),
        (
            Term::Float(1.0),
            "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))",
        ),
        (
            Term::Integer(1 << 62),
            "Value(Tuple([Atom(\"error\"), Atom(\"enomem\")]))",
        ),
    ] {
        assert_eq!(text(&mut scheduler, |s| read_nif(s, &fd, &size)), expected);
    }
    assert_eq!(
        text(&mut scheduler, |s| close_nif(s, &fd)),
        "Value(Atom(\"ok\"))"
    );
    assert_eq!(
        text(&mut scheduler, |s| close_nif(s, &fd)),
        "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))"
    );
    let size = Term::Integer(1);
    // No BEAM code reaches `read_nif/2` without the `prim_buffer` NIFs
    // (#733), so this call enters through the binding.
    let read = BINDINGS
        .iter()
        .find(|binding| binding.name == "read_nif")
        .unwrap();
    let arguments = [fd.clone(), size.clone()];
    assert_eq!(
        text(&mut scheduler, |s| (read.run)(s, &arguments)),
        "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))"
    );
    assert_eq!(
        text(&mut scheduler, |s| read_nif(s, &fd, &size)),
        "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))"
    );
    // A term that is not a file resource raises badarg.
    let other = Term::Reference(scheduler.make_ref().unwrap());
    for fd in [other, Term::static_atom("ok")] {
        assert_eq!(text(&mut scheduler, |s| close_nif(s, &fd)), BADARG);
        assert_eq!(text(&mut scheduler, |s| read_nif(s, &fd, &size)), BADARG);
    }
    // A read-only file gives the errno of a write, and a write-only file
    // the errno of a read.
    let read_only = open(&mut scheduler, &file, &["read"]);
    assert_eq!(
        text(&mut scheduler, |s| write_nif(s, &read_only, &iovec)),
        "Value(Tuple([Atom(\"error\"), Atom(\"ebadf\")]))"
    );
    let write_only = open(&mut scheduler, &file, &["write"]);
    assert_eq!(
        text(&mut scheduler, |s| read_nif(s, &write_only, &size)),
        "Value(Tuple([Atom(\"error\"), Atom(\"ebadf\")]))"
    );
    // A read of 0 bytes reads the host file too (unix_prim_file.c:292-341).
    let empty = Term::Integer(0);
    assert_eq!(
        text(&mut scheduler, |s| read_nif(s, &write_only, &empty)),
        "Value(Tuple([Atom(\"error\"), Atom(\"ebadf\")]))"
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"");
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn changes_names_directories_and_links_with_the_errors_of_erts() {
    // Each binding of the write and metadata paths marshals the path with
    // `efile_marshal_path` before its host call (prim_file_nif.c:1149-1249,
    // 1282-1296, 1389-1404), and `unix_prim_file.c` gives the Unix
    // results. The test drives each binding directly, because `prim_file`
    // gives only paths that end with a NUL to the NIFs.
    let mut scheduler = scheduler();
    let directory = scratch("changes");
    let empty = directory.join("empty");
    let full = directory.join("full");
    let missing = directory.join("missing");
    std::fs::create_dir(&empty).unwrap();
    std::fs::create_dir(&full).unwrap();
    std::fs::write(full.join("child"), b"").unwrap();
    let einval = "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))";
    let enoent = "Value(Tuple([Atom(\"error\"), Atom(\"enoent\")]))";
    let eexist = "Value(Tuple([Atom(\"error\"), Atom(\"eexist\")]))";
    let enotsup = "Value(Tuple([Atom(\"error\"), Atom(\"enotsup\")]))";
    let invalid = Term::binary(b"/".to_vec());
    for name in [
        "del_file_nif",
        "del_dir_nif",
        "make_dir_nif",
        "set_cwd_nif",
        "altname_nif",
    ] {
        let binding = BINDINGS
            .iter()
            .find(|binding| (binding.name, binding.arity) == (name, 1))
            .unwrap();
        assert_eq!(
            text(&mut scheduler, |s| (binding.run)(
                s,
                std::slice::from_ref(&invalid)
            )),
            einval,
            "{name}"
        );
    }
    for name in ["rename_nif", "make_hard_link_nif", "make_soft_link_nif"] {
        let binding = BINDINGS
            .iter()
            .find(|binding| (binding.name, binding.arity) == (name, 2))
            .unwrap();
        assert_eq!(
            text(&mut scheduler, |s| (binding.run)(
                s,
                &[invalid.clone(), nif_path(&empty)]
            )),
            einval,
            "{name} first"
        );
        assert_eq!(
            text(&mut scheduler, |s| (binding.run)(
                s,
                &[nif_path(&empty), invalid.clone()]
            )),
            einval,
            "{name} second"
        );
    }
    // `rename` of the root gives `EINVAL`, and a non-empty target gives
    // `EEXIST` for the `ENOTEMPTY` of the host (unix_prim_file.c:945-958).
    assert_eq!(
        text(&mut scheduler, |_| pair_result(
            &Term::binary(b"/\0".to_vec()),
            &nif_path(&empty),
            file::rename
        )),
        einval
    );
    assert_eq!(
        text(&mut scheduler, |_| pair_result(
            &nif_path(&empty),
            &nif_path(&full),
            file::rename
        )),
        eexist
    );
    // A missing source gives `ENOENT` for `rename` too.
    assert_eq!(
        text(&mut scheduler, |_| pair_result(
            &nif_path(&missing),
            &nif_path(&empty),
            file::rename
        )),
        enoent
    );
    // A missing directory gives `ENOENT` for `chdir`.
    assert_eq!(
        text(&mut scheduler, |_| path_result(
            &nif_path(&missing),
            file::set_cwd
        )),
        enoent
    );
    // A failed first `chmod` repeats the call without the set-id bits
    // (unix_prim_file.c:821-834).
    assert_eq!(
        text(&mut scheduler, |_| set_permissions_nif(
            &nif_path(&missing),
            &Term::Integer(0o644)
        )),
        enoent
    );
    // The three metadata NIFs check their arguments, then the path
    // (prim_file_nif.c:1053-1113).
    assert_eq!(
        text(&mut scheduler, |_| set_permissions_nif(
            &invalid,
            &Term::Integer(0o644)
        )),
        einval
    );
    assert_eq!(
        text(&mut scheduler, |_| set_owner_nif(
            &invalid,
            &Term::Integer(-1),
            &Term::Integer(-1)
        )),
        einval
    );
    assert_eq!(
        text(&mut scheduler, |_| set_time_nif(
            &invalid,
            &Term::Integer(0),
            &Term::Integer(0),
            &Term::Integer(0)
        )),
        einval
    );
    // `altname` is `ENOTSUP` on Unix (unix_prim_file.c:1085-1090).
    assert_eq!(
        text(&mut scheduler, |_| altname_nif(&nif_path(&empty))),
        enotsup
    );
    // The mutable modes exclude the sticky bit (unix_prim_file.c:822).
    let sticky = directory.join("sticky");
    std::fs::write(&sticky, b"").unwrap();
    assert_eq!(
        text(&mut scheduler, |_| set_permissions_nif(
            &nif_path(&sticky),
            &Term::Integer(0o1777)
        )),
        "Value(Atom(\"ok\"))"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&sticky).unwrap().permissions().mode() & 0o7777,
        0o777
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn advises_a_file_with_the_errors_of_erts() {
    // `advise_nif_impl` reads the two numbers, then the advise
    // (prim_file_nif.c:859-884), so a call with a bad number and a bad
    // advise gives einval for the number. The pinned ERTS gives
    // `[ok, {error, einval}, ok, {error, einval}, {error, badarg}]` for a
    // file that it opened for read (measured).
    let mut scheduler = scheduler();
    let directory = scratch("advise");
    let file = directory.join("file");
    std::fs::write(&file, b"abc").unwrap();
    let fd = open(&mut scheduler, &file, &["read"]);
    let sequential = Term::Atom(scheduler.atoms.put("sequential").unwrap());
    let never = Term::Atom(scheduler.atoms.put("never").unwrap());
    let zero = Term::Integer(0);
    let einval = "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))";
    assert_eq!(
        text(&mut scheduler, |s| advise_nif(
            s,
            &fd,
            &zero,
            &zero,
            &sequential
        )),
        "Value(Atom(\"ok\"))"
    );
    // An advice that ERTS does not know gives einval.
    assert_eq!(
        text(&mut scheduler, |s| advise_nif(s, &fd, &zero, &zero, &never)),
        einval
    );
    for offset in [
        Term::Integer(-1),
        Term::Float(0.0),
        Term::BigInt(std::sync::Arc::new(9223372036854775808u64.into())),
    ] {
        assert_eq!(
            text(&mut scheduler, |s| advise_nif(
                s,
                &fd,
                &offset,
                &zero,
                &sequential
            )),
            einval
        );
    }
    // A value that is not a number raises badarg.
    assert_eq!(
        text(&mut scheduler, |s| advise_nif(
            s,
            &fd,
            &Term::binary(b"x".to_vec()),
            &zero,
            &sequential
        )),
        BADARG
    );
    // A closed resource gives einval.
    assert_eq!(
        text(&mut scheduler, |s| close_nif(s, &fd)),
        "Value(Atom(\"ok\"))"
    );
    assert_eq!(
        text(&mut scheduler, |s| advise_nif(
            s,
            &fd,
            &zero,
            &zero,
            &sequential
        )),
        einval
    );
    std::fs::remove_dir_all(&directory).unwrap();
}
