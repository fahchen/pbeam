//! Native bindings of the `prim_file` NIFs (`erts/emulator/nifs/common/prim_file_nif.c`).
//!
//! A file resource of `open_nif/2` is a reference to a host `File` in the
//! `Files` value of the node resources, as a TTY resource of `prim_tty` is.
//!
//! The bindings have no capability check yet: the `file` kind of the
//! capability security migration adds it (row "Capability checks" of
//! `docs/erts-differences.md`). ERTS runs each of these NIFs on a dirty IO
//! scheduler (`prim_file_nif.c:179-182`, `185`, `196`, `200-201`, `207`,
//! `212`). PBeam runs
//! them on the scheduler thread, so a host call that waits stops all
//! processes until it returns (row "Dirty IO" of
//! `docs/erts-differences.md`).

use super::*;
use crate::runtime::{
    files::Files,
    host::{
        errno::errno_id,
        file::{self, FileInfo, FileType, OpenModes},
    },
    signal::Signal,
};
use std::{
    ffi::{CStr, c_int},
    fs::File,
    io::{IoSlice, SeekFrom},
};

/// The functions of the ERTS `prim_file` NIF library, as `(name, arity)` in
/// the order of `nif_funcs` (`prim_file_nif.c:177-219`). `erlang:load_nif/2`
/// checks each of them in this order. PBeam has a binding for only some of
/// them; `BINDINGS` lists those.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(FUNCTIONS[15], ("read_info_nif", 2));
/// assert_eq!(FUNCTIONS[32], ("file_desc_to_ref_nif", 1));
/// ```
pub(super) const FUNCTIONS: [(&str, u32); 33] = [
    ("open_nif", 2),
    ("close_nif", 1),
    ("read_nif", 2),
    ("write_nif", 2),
    ("pread_nif", 3),
    ("pwrite_nif", 3),
    ("seek_nif", 3),
    ("sync_nif", 2),
    ("truncate_nif", 1),
    ("allocate_nif", 3),
    ("advise_nif", 4),
    ("read_handle_info_nif", 1),
    ("make_hard_link_nif", 2),
    ("make_soft_link_nif", 2),
    ("rename_nif", 2),
    ("read_info_nif", 2),
    ("set_permissions_nif", 2),
    ("set_owner_nif", 3),
    ("set_time_nif", 4),
    ("read_link_nif", 1),
    ("list_dir_nif", 1),
    ("make_dir_nif", 1),
    ("del_file_nif", 1),
    ("del_dir_nif", 1),
    ("get_device_cwd_nif", 1),
    ("set_cwd_nif", 1),
    ("get_cwd_nif", 0),
    ("ipread_s32bu_p32bu_nif", 3),
    ("read_file_nif", 1),
    ("get_handle_nif", 1),
    ("delayed_close_nif", 1),
    ("altname_nif", 1),
    ("file_desc_to_ref_nif", 1),
];

/// The registered bindings of `prim_file`, sorted by name, then by arity.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!((BINDINGS[0].name, BINDINGS[0].arity), ("close_nif", 1));
/// ```
pub(super) const BINDINGS: &[Binding] = &[
    Binding {
        name: "altname_nif",
        arity: 1,
        run: |scheduler, arguments| altname_nif(&mut scheduler.atoms, &arguments[0]),
    },
    Binding {
        name: "close_nif",
        arity: 1,
        run: |scheduler, arguments| close_nif(scheduler, &arguments[0]),
    },
    Binding {
        name: "del_dir_nif",
        arity: 1,
        run: |scheduler, arguments| path_result(scheduler, &arguments[0], file::delete_dir),
    },
    Binding {
        name: "del_file_nif",
        arity: 1,
        run: |scheduler, arguments| path_result(scheduler, &arguments[0], file::delete_file),
    },
    Binding {
        name: "get_cwd_nif",
        arity: 0,
        run: |scheduler, _| get_cwd_nif(&mut scheduler.atoms),
    },
    Binding {
        name: "list_dir_nif",
        arity: 1,
        run: |scheduler, arguments| list_dir_nif(&mut scheduler.atoms, &arguments[0]),
    },
    Binding {
        name: "make_dir_nif",
        arity: 1,
        run: |scheduler, arguments| path_result(scheduler, &arguments[0], file::make_dir),
    },
    Binding {
        name: "make_hard_link_nif",
        arity: 2,
        run: |scheduler, arguments| {
            pair_result(
                scheduler,
                &arguments[0],
                &arguments[1],
                file::make_hard_link,
            )
        },
    },
    Binding {
        name: "make_soft_link_nif",
        arity: 2,
        run: |scheduler, arguments| {
            pair_result(
                scheduler,
                &arguments[0],
                &arguments[1],
                file::make_soft_link,
            )
        },
    },
    Binding {
        name: "open_nif",
        arity: 2,
        run: |scheduler, arguments| open_nif(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "read_file_nif",
        arity: 1,
        run: |scheduler, arguments| read_file_nif(&mut scheduler.atoms, &arguments[0]),
    },
    Binding {
        name: "read_info_nif",
        arity: 2,
        run: |scheduler, arguments| read_info_nif(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "read_link_nif",
        arity: 1,
        run: |scheduler, arguments| read_link_nif(&mut scheduler.atoms, &arguments[0]),
    },
    Binding {
        name: "read_nif",
        arity: 2,
        run: |scheduler, arguments| read_nif(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "rename_nif",
        arity: 2,
        run: |scheduler, arguments| {
            pair_result(scheduler, &arguments[0], &arguments[1], file::rename)
        },
    },
    Binding {
        name: "seek_nif",
        arity: 3,
        run: |scheduler, arguments| {
            seek_nif(scheduler, &arguments[0], &arguments[1], &arguments[2])
        },
    },
    Binding {
        name: "set_cwd_nif",
        arity: 1,
        run: |scheduler, arguments| path_result(scheduler, &arguments[0], file::set_cwd),
    },
    Binding {
        name: "set_owner_nif",
        arity: 3,
        run: |scheduler, arguments| {
            set_owner_nif(scheduler, &arguments[0], &arguments[1], &arguments[2])
        },
    },
    Binding {
        name: "set_permissions_nif",
        arity: 2,
        run: |scheduler, arguments| set_permissions_nif(scheduler, &arguments[0], &arguments[1]),
    },
    Binding {
        name: "set_time_nif",
        arity: 4,
        run: |scheduler, arguments| {
            set_time_nif(
                scheduler,
                &arguments[0],
                &arguments[1],
                &arguments[2],
                &arguments[3],
            )
        },
    },
    Binding {
        name: "write_nif",
        arity: 2,
        run: |scheduler, arguments| write_nif(scheduler, &arguments[0], &arguments[1]),
    },
];

/// The most elements of the iovec of one `write_nif/2` call, as the
/// `max_elements` argument of `enif_inspect_iovec` in `write_nif_impl`
/// (`prim_file_nif.c:694`).
const WRITE_MAX_ELEMENTS: usize = 64;

/// The smallest file time that `build_file_info` gives, in seconds since
/// 1970: `{{1902,1,1},{0,0,0}}` (`EFILE_MIN_FILETIME`,
/// `prim_file_nif.h:97-100`).
const MIN_FILETIME: i64 = -2_145_916_800;

/// The largest file time that `build_file_info` gives, in seconds since
/// 1970: `{{9999,12,31},{23,59,59}}` (`EFILE_MAX_FILETIME`,
/// `prim_file_nif.h:102-105`).
const MAX_FILETIME: i64 = 253_402_300_799;

/// Run `prim_file:open_nif(Path, Modes)`, as the ERTS NIF `open_nif`
/// (`prim_file_nif.c:572-593`) does on Unix:
///
/// 1. `Modes` that is not a list raises `error:badarg`.
/// 2. The modes of [`translate_modelist`].
/// 3. A `Path` that [`marshal_path`] does not accept gives
///    `{error, einval}`.
/// 4. [`file::open`] opens the path. A failed open gives `{error, Errno}`.
/// 5. Else the result is `{ok, Ref}` with a new file resource, and the
///    resource monitors the caller (`create_ref_or_error_tuple`,
///    `prim_file_nif.c:541-570`).
///
/// # Examples
///
/// ```rust,ignore
/// let read = Term::list_from(vec![Term::Atom(scheduler.atoms.put("read")?)]);
/// // {ok, Ref}
/// let result = open_nif(&mut scheduler, &Term::binary(b"/dev/null\0".to_vec()), &read)?;
/// // {error, eisdir}
/// let result = open_nif(&mut scheduler, &Term::binary(b"/\0".to_vec()), &read)?;
/// ```
///
/// # Errors
///
/// A full reference counter gives the error of `Scheduler::make_ref`. A new
/// result atom in a full atom table gives the error of `AtomTable::put`.
fn open_nif(scheduler: &mut Scheduler, path: &Term, modes: &Term) -> Result<BifResult, BeamError> {
    if !modes.is_list() {
        return Ok(BifResult::badarg());
    }
    let modes = translate_modelist(&scheduler.atoms, modes);
    let atoms = &mut scheduler.atoms;
    let Some(path) = marshal_path(path) else {
        return error_tuple(atoms, "einval");
    };
    let file = match file::open(path, modes) {
        Ok(file) => file,
        Err(errno) => return error_tuple(atoms, &errno_id(errno)),
    };
    let reference = scheduler.make_ref()?;
    let caller = scheduler.current_process_mut().id;
    scheduler
        .resources
        .borrow_mut::<Files>()
        .insert(reference, file, caller);
    scheduler
        .send_signal(None, caller, Signal::FileMonitor { file: reference })
        .expect("the caller is alive");
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        Term::Reference(reference),
    ])))
}

/// Return the open modes of the list `modes`, as `efile_translate_modelist`
/// (`prim_file_nif.c:500-539`) does: the atoms `read`, `write`,
/// `exclusive`, `append`, `sync`, `skip_type_check`, and `directory` set
/// their mode, and each other element is ignored, also the tail of an
/// improper list. `append` and `exclusive` add `write`, and modes with
/// neither `read` nor `write` add `read`.
///
/// # Examples
///
/// ```rust,ignore
/// let append = Term::list_from(vec![Term::static_atom("append")]);
/// assert!(translate_modelist(&atoms, &append).write);
/// ```
fn translate_modelist(atoms: &AtomTable, modes: &Term) -> OpenModes {
    let mut open = OpenModes::default();
    for mode in modes.list_iter().into_iter().flatten() {
        let Term::Atom(mode) = mode else {
            continue;
        };
        match atoms.name_of(mode) {
            "read" => open.read = true,
            "write" => open.write = true,
            "exclusive" => open.exclusive = true,
            "append" => open.append = true,
            "sync" => open.sync = true,
            "skip_type_check" => open.skip_type_check = true,
            "directory" => open.directory = true,
            _ => {}
        }
    }
    if open.append || open.exclusive {
        open.write = true;
    } else if !open.read && !open.write {
        open.read = true;
    }
    open
}

/// Run `prim_file:close_nif(Fd)`, as the ERTS NIF `close_nif`
/// (`prim_file_nif.c:618-647`) does: a term that is not a file resource
/// raises `error:badarg`, a closed resource gives `{error, einval}`, and
/// else the binding removes the monitor of the resource on its owner
/// (`enif_demonitor_process`, `prim_file_nif.c:633`), closes the host
/// file with [`file::close`], and gives `ok`, or `{error, Errno}` for a
/// failed `close`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(close_nif(&mut scheduler, &fd)?, BifResult::Value(Term::Atom(_))));
/// // {error, einval}
/// assert!(matches!(close_nif(&mut scheduler, &fd)?, BifResult::Value(Term::Tuple(_))));
/// ```
///
/// # Errors
///
/// The atom `einval` in a full atom table gives the error of
/// `AtomTable::put`.
fn close_nif(scheduler: &mut Scheduler, fd: &Term) -> Result<BifResult, BeamError> {
    let Term::Reference(reference) = fd else {
        return Ok(BifResult::badarg());
    };
    match scheduler.resources.borrow_mut::<Files>().close(*reference) {
        None => Ok(BifResult::badarg()),
        Some(None) => error_tuple(&mut scheduler.atoms, "einval"),
        Some(Some((file, owner))) => {
            scheduler.demonitor_file(owner, *reference);
            let closed = file::close(file).map(|()| Term::static_atom("ok"));
            value_or_error(&mut scheduler.atoms, closed)
        }
    }
}

/// Run `operation` on the open host file of the file resource `fd`, as
/// `file_handle_wrapper` (`prim_file_nif.c:315-357`) does: a term that is
/// not a file resource raises `error:badarg`, and a closed resource gives
/// `{error, einval}`, before `operation` reads its arguments.
///
/// # Examples
///
/// ```rust,ignore
/// let result = file_handle_wrapper(&mut scheduler, &fd, |_file, _atoms| Ok(BifResult::Value(Term::nil())))?;
/// ```
///
/// # Errors
///
/// The error of `operation`. The atom `einval` in a full atom table gives
/// the error of `AtomTable::put`.
fn file_handle_wrapper(
    scheduler: &mut Scheduler,
    fd: &Term,
    operation: impl FnOnce(&File, &mut AtomTable) -> Result<BifResult, BeamError>,
) -> Result<BifResult, BeamError> {
    let Term::Reference(reference) = fd else {
        return Ok(BifResult::badarg());
    };
    match scheduler.resources.borrow_mut::<Files>().get(*reference) {
        None => Ok(BifResult::badarg()),
        Some(None) => error_tuple(&mut scheduler.atoms, "einval"),
        Some(Some(file)) => operation(file, &mut scheduler.atoms),
    }
}

/// Run `prim_file:read_nif(Fd, Size)`, as the ERTS NIF `read_nif_impl`
/// (`prim_file_nif.c:649-686`) does after [`file_handle_wrapper`]: a `Size`
/// that is not a number raises `error:badarg`, and a float, an integer
/// outside the `Sint64` range, or a negative integer gives
/// `{error, einval}`. Else [`file::read`] reads at most `Size` bytes. The
/// result is `eof` for 0 bytes, `{ok, Bytes}`, or `{error, Errno}`.
///
/// # Examples
///
/// ```rust,ignore
/// // {ok, <<"ab">>}
/// let result = read_nif(&mut scheduler, &fd, &Term::Integer(2))?;
/// // {error, einval}
/// let result = read_nif(&mut scheduler, &fd, &Term::Integer(-1))?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn read_nif(scheduler: &mut Scheduler, fd: &Term, size: &Term) -> Result<BifResult, BeamError> {
    file_handle_wrapper(scheduler, fd, |file, atoms| {
        let size = match size {
            Term::Integer(size) => u64::try_from(*size).ok(),
            Term::BigInt(_) | Term::Float(_) => None,
            _ => return Ok(BifResult::badarg()),
        };
        let Some(size) = size else {
            return error_tuple(atoms, "einval");
        };
        match file::read(file, size) {
            Ok(bytes) if bytes.is_empty() => Ok(BifResult::Value(Term::static_atom("eof"))),
            result => ok_tuple(atoms, result.map(Term::binary)),
        }
    })
}

/// Run `prim_file:write_nif(Fd, IOVec)`, as the ERTS NIF `write_nif_impl`
/// (`prim_file_nif.c:688-710`) does after [`file_handle_wrapper`]: the
/// first [`WRITE_MAX_ELEMENTS`] elements of `IOVec` must be binaries and be
/// followed by a list, as `enif_inspect_iovec` (`erl_nif.c:4107-4166`,
/// `4342-4356`) checks, else the call raises `error:badarg` and writes
/// nothing. [`file::write`] then writes their bytes with no copy. A failed write gives
/// `{error, Errno}`. Else the result is `ok` when the rest of the list is
/// `[]`, and `{continue, Rest}` for the rest of a longer list.
///
/// # Examples
///
/// ```rust,ignore
/// let iovec = Term::list_from(vec![Term::binary(b"ab".to_vec())]);
/// assert!(matches!(write_nif(&mut scheduler, &fd, &iovec)?, BifResult::Value(Term::Atom(_))));
/// assert!(matches!(write_nif(&mut scheduler, &fd, &Term::binary(vec![]))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn write_nif(scheduler: &mut Scheduler, fd: &Term, iovec: &Term) -> Result<BifResult, BeamError> {
    file_handle_wrapper(scheduler, fd, |file, atoms| {
        let mut buffers = Vec::new();
        let mut rest = iovec;
        let mut elements = 0;
        while let Some((element, tail)) = rest.cons_cell() {
            let Term::Binary(binary) = element else {
                return Ok(BifResult::badarg());
            };
            // `examine_iovec_term` gives no iovec entry for an empty binary
            // (erl_nif.c:4123-4155).
            if !binary.is_empty() {
                buffers.push(IoSlice::new(binary));
            }
            rest = tail;
            elements += 1;
            if elements >= WRITE_MAX_ELEMENTS {
                break;
            }
        }
        if !rest.is_list() {
            return Ok(BifResult::badarg());
        }
        let value = if rest.is_nil() {
            Term::static_atom("ok")
        } else {
            Term::tuple(vec![Term::Atom(atoms.put("continue")?), rest.clone()])
        };
        value_or_error(atoms, file::write(file, &mut buffers).map(|()| value))
    })
}

/// Run `prim_file:seek_nif(Fd, Mark, Offset)`, as the ERTS NIF
/// `seek_nif_impl` (`prim_file_nif.c:782-806`) does after
/// [`file_handle_wrapper`]: an `Offset` that is not an integer in the
/// `Sint64` range raises `error:badarg`, and then a `Mark` that is not
/// `bof`, `cur`, or `eof` too. Else [`file::seek`] moves the position. The
/// result is `{ok, Position}` or `{error, Errno}`.
///
/// # Examples
///
/// ```rust,ignore
/// // {ok, 0}
/// let result = seek_nif(&mut scheduler, &fd, &Term::Atom(scheduler.atoms.put("bof")?), &Term::Integer(0))?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn seek_nif(
    scheduler: &mut Scheduler,
    fd: &Term,
    mark: &Term,
    offset: &Term,
) -> Result<BifResult, BeamError> {
    file_handle_wrapper(scheduler, fd, |file, atoms| {
        let Term::Integer(offset) = *offset else {
            return Ok(BifResult::badarg());
        };
        let from = match mark {
            Term::Atom(mark) => match atoms.name_of(mark) {
                // `SeekFrom::Start` gives the bits of `offset` to `lseek`
                // as `off_t`, so a negative offset stays negative.
                "bof" => SeekFrom::Start(offset.cast_unsigned()),
                "cur" => SeekFrom::Current(offset),
                "eof" => SeekFrom::End(offset),
                _ => return Ok(BifResult::badarg()),
            },
            _ => return Ok(BifResult::badarg()),
        };
        let position = file::seek(file, from).map(|position| Term::Integer(position.cast_signed()));
        ok_tuple(atoms, position)
    })
}

/// Run `prim_file:read_info_nif(Path, FollowLinks)`, as the ERTS NIF
/// `read_info_nif` (`prim_file_nif.c:1019-1038`) does on Unix:
///
/// 1. A `FollowLinks` that is not an integer in the C `int` range raises
///    `error:badarg` (`enif_get_int`).
/// 2. A `Path` that is not a binary, or a binary that does not end with
///    one or more NUL bytes after its last other byte, gives
///    `{error, einval}` (`efile_marshal_path` and
///    `has_invalid_null_termination`, `unix_prim_file.c:71-101`).
/// 3. The bytes before the first NUL are the path. `FollowLinks` other than
///    0 reads it with `stat`, and 0 with `lstat` (see [`file::read_info`]).
///    A failed read gives `{error, Errno}` with the `erl_errno_id` atom
///    (`posix_error_to_tuple`, `prim_file_nif.c:306-309`).
/// 4. Else the result is the `#file_info` tuple of `build_file_info`
///    (`prim_file_nif.c:996-1017`), with the times in seconds since 1970
///    between [`MIN_FILETIME`] and [`MAX_FILETIME`].
///
/// # Examples
///
/// ```rust,ignore
/// // {file_info,166576,regular,read,{...},{...},{...},33261,1,16777229,0,727681,0,0}
/// let result = read_info_nif(&mut scheduler, &Term::binary(b"/bin/sh\0".to_vec()), &Term::Integer(1))?;
/// assert!(matches!(result, BifResult::Value(Term::Tuple(fields)) if fields.len() == 14));
/// let result = read_info_nif(&mut scheduler, &Term::binary(b"/bin/sh".to_vec()), &Term::Integer(1))?;
/// // {error, einval}
/// assert!(matches!(result, BifResult::Value(Term::Tuple(fields)) if fields.len() == 2));
/// assert!(matches!(read_info_nif(&mut scheduler, &Term::binary(vec![0]), &Term::nil())?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn read_info_nif(
    scheduler: &mut Scheduler,
    path: &Term,
    follow_links: &Term,
) -> Result<BifResult, BeamError> {
    let Some(follow_links) = follow_links
        .small_integer()
        .and_then(|value| i32::try_from(value).ok())
    else {
        return Ok(BifResult::badarg());
    };
    let atoms = &mut scheduler.atoms;
    let Some(path) = marshal_path(path) else {
        return error_tuple(atoms, "einval");
    };
    match file::read_info(path, follow_links != 0) {
        Ok(info) => Ok(file_info(&info)),
        Err(errno) => error_tuple(atoms, &errno_id(errno)),
    }
}

/// Run `prim_file:read_file_nif(Path)`, as the ERTS NIF `read_file_nif`
/// (`prim_file_nif.c:1354-1387`) does on Unix. A `Path` that
/// [`marshal_path`] does not accept gives `{error, einval}`. Else the
/// result is `{ok, Contents}` with the bytes of [`file::read_file`], or
/// `{error, Errno}`.
///
/// # Examples
///
/// ```rust,ignore
/// // {ok, <<>>}
/// let result = read_file_nif(&mut atoms, &Term::binary(b"/dev/null\0".to_vec()))?;
/// // {error, eisdir}
/// let result = read_file_nif(&mut atoms, &Term::binary(b"/\0".to_vec()))?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn read_file_nif(atoms: &mut AtomTable, path: &Term) -> Result<BifResult, BeamError> {
    let Some(path) = marshal_path(path) else {
        return error_tuple(atoms, "einval");
    };
    ok_tuple(atoms, file::read_file(path).map(Term::binary))
}

/// Run `prim_file:list_dir_nif(Path)`, as the ERTS NIF `list_dir_nif`
/// (`prim_file_nif.c:1132-1147`) does on Unix. A `Path` that
/// [`marshal_path`] does not accept gives `{error, einval}`. Else the
/// result is `{ok, Names}` with a binary for each name of
/// [`file::list_dir`], or `{error, Errno}`. `efile_list_dir` adds each name
/// at the head of the list (`unix_prim_file.c:921-937`), so `Names` is in
/// the reverse order of `readdir`.
///
/// # Examples
///
/// ```rust,ignore
/// // {ok, [<<"b">>, <<"a">>]} when readdir gives "a", then "b"
/// let result = list_dir_nif(&mut atoms, &Term::binary(b"/tmp/dir\0".to_vec()))?;
/// // {error, enoent}
/// let result = list_dir_nif(&mut atoms, &Term::binary(b"/pbeam-missing\0".to_vec()))?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn list_dir_nif(atoms: &mut AtomTable, path: &Term) -> Result<BifResult, BeamError> {
    let Some(path) = marshal_path(path) else {
        return error_tuple(atoms, "einval");
    };
    let names = file::list_dir(path)
        .map(|names| Term::list_from(names.into_iter().rev().map(Term::binary)));
    ok_tuple(atoms, names)
}

/// Run `prim_file:read_link_nif(Path)`, as the ERTS NIF `read_link_nif`
/// (`prim_file_nif.c:1115-1130`) does on Unix. A `Path` that
/// [`marshal_path`] does not accept gives `{error, einval}`. Else the
/// result is `{ok, Contents}` with the bytes of [`file::read_link`], or
/// `{error, Errno}`.
///
/// # Examples
///
/// ```rust,ignore
/// // {ok, <<"target">>} for a symbolic link to "target"
/// let result = read_link_nif(&mut atoms, &Term::binary(b"/tmp/link\0".to_vec()))?;
/// // {error, einval}
/// let result = read_link_nif(&mut atoms, &Term::binary(b"/\0".to_vec()))?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn read_link_nif(atoms: &mut AtomTable, path: &Term) -> Result<BifResult, BeamError> {
    let Some(path) = marshal_path(path) else {
        return error_tuple(atoms, "einval");
    };
    ok_tuple(atoms, file::read_link(path).map(Term::binary))
}

/// Run `prim_file:get_cwd_nif()`, as the ERTS NIF `get_cwd_nif`
/// (`prim_file_nif.c:1269-1280`) does on Unix: `{ok, Path}` with the bytes
/// of [`file::get_cwd`], or `{error, Errno}`.
///
/// # Examples
///
/// ```rust,ignore
/// // {ok, <<"/home/user">>}
/// let result = get_cwd_nif(&mut atoms)?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn get_cwd_nif(atoms: &mut AtomTable) -> Result<BifResult, BeamError> {
    ok_tuple(atoms, file::get_cwd().map(Term::binary))
}

/// Run `prim_file:altname_nif(Path)`, as the ERTS NIF `altname_nif`
/// (`prim_file_nif.c:1389-1404`) does on Unix: a `Path` that
/// [`marshal_path`] does not accept gives `{error, einval}`, and each other
/// path gives `{error, enotsup}` (`efile_altname`,
/// `unix_prim_file.c:1085-1090`).
///
/// # Examples
///
/// ```rust,ignore
/// // {error, enotsup}
/// let result = altname_nif(&mut atoms, &Term::binary(b"/tmp\0".to_vec()))?;
/// // {error, einval}
/// let result = altname_nif(&mut atoms, &Term::binary(b"/tmp".to_vec()))?;
/// ```
///
/// # Errors
///
/// The atom `einval` or `enotsup` in a full atom table gives the error of
/// `AtomTable::put`.
fn altname_nif(atoms: &mut AtomTable, path: &Term) -> Result<BifResult, BeamError> {
    if marshal_path(path).is_none() {
        return error_tuple(atoms, "einval");
    }
    error_tuple(atoms, "enotsup")
}

/// Run the `prim_file` NIF that takes the path `path` and calls `operation`
/// on it, as each of `del_file_nif`, `del_dir_nif`, `make_dir_nif`, and
/// `set_cwd_nif` (`prim_file_nif.c:1203-1249`, `1282-1296`) does: a path
/// that [`marshal_path`] does not accept gives `{error, einval}`, else
/// `operation` gives `ok` or `{error, Errno}`.
///
/// # Examples
///
/// ```rust,ignore
/// // {error, enoent}
/// let result = path_result(&mut scheduler, &Term::binary(b"/pbeam-missing\0".to_vec()), file::delete_file)?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn path_result(
    scheduler: &mut Scheduler,
    path: &Term,
    operation: impl FnOnce(&CStr) -> Result<(), c_int>,
) -> Result<BifResult, BeamError> {
    let atoms = &mut scheduler.atoms;
    let Some(path) = marshal_path(path) else {
        return error_tuple(atoms, "einval");
    };
    value_or_error(atoms, operation(path).map(|()| Term::static_atom("ok")))
}

/// Run the `prim_file` NIF that takes the paths `first` and `second` and
/// calls `operation` on them, as `rename_nif`, `make_hard_link_nif`, and
/// `make_soft_link_nif` (`prim_file_nif.c:1149-1201`) do: a path that
/// [`marshal_path`] does not accept gives `{error, einval}`, and the first
/// path is checked before the second. Else `operation` gives `ok` or
/// `{error, Errno}`.
///
/// # Examples
///
/// ```rust,ignore
/// // {error, enoent}
/// let result = pair_result(&mut scheduler, &source, &destination, file::rename)?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn pair_result(
    scheduler: &mut Scheduler,
    first: &Term,
    second: &Term,
    operation: impl FnOnce(&CStr, &CStr) -> Result<(), c_int>,
) -> Result<BifResult, BeamError> {
    let atoms = &mut scheduler.atoms;
    let Some(first) = marshal_path(first) else {
        return error_tuple(atoms, "einval");
    };
    let Some(second) = marshal_path(second) else {
        return error_tuple(atoms, "einval");
    };
    value_or_error(
        atoms,
        operation(first, second).map(|()| Term::static_atom("ok")),
    )
}

/// Run `prim_file:set_permissions_nif(Path, Permissions)`, as the ERTS NIF
/// `set_permissions_nif` (`prim_file_nif.c:1053-1071`) does: a
/// `Permissions` that is not an unsigned 32-bit integer raises
/// `error:badarg` before the path check, a path that [`marshal_path`] does
/// not accept gives `{error, einval}`, and else [`file::set_permissions`]
/// gives `ok` or `{error, Errno}`.
///
/// # Examples
///
/// ```rust,ignore
/// // ok
/// let result = set_permissions_nif(&mut scheduler, &Term::binary(b"/tmp/f\0".to_vec()), &Term::Integer(0o644))?;
/// // badarg
/// let result = set_permissions_nif(&mut scheduler, &path, &Term::Integer(-1))?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn set_permissions_nif(
    scheduler: &mut Scheduler,
    path: &Term,
    permissions: &Term,
) -> Result<BifResult, BeamError> {
    let Some(permissions) = permissions
        .small_integer()
        .and_then(|value| u32::try_from(value).ok())
    else {
        return Ok(BifResult::badarg());
    };
    let atoms = &mut scheduler.atoms;
    let Some(path) = marshal_path(path) else {
        return error_tuple(atoms, "einval");
    };
    value_or_error(
        atoms,
        file::set_permissions(path, permissions).map(|()| Term::static_atom("ok")),
    )
}

/// Run `prim_file:set_owner_nif(Path, Uid, Gid)`, as the ERTS NIF
/// `set_owner_nif` (`prim_file_nif.c:1073-1091`) does: a `Uid` or `Gid`
/// that is not a C `int` raises `error:badarg` before the path check, a
/// path that [`marshal_path`] does not accept gives `{error, einval}`, and
/// else [`file::set_owner`] gives `ok` or `{error, Errno}`. The value -1
/// leaves its field unchanged.
///
/// # Examples
///
/// ```rust,ignore
/// // {error, enoent}
/// let result = set_owner_nif(&mut scheduler, &Term::binary(b"/pbeam-missing\0".to_vec()), &Term::Integer(-1), &Term::Integer(-1))?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn set_owner_nif(
    scheduler: &mut Scheduler,
    path: &Term,
    uid: &Term,
    gid: &Term,
) -> Result<BifResult, BeamError> {
    let uid = uid
        .small_integer()
        .and_then(|value| i32::try_from(value).ok());
    let gid = gid
        .small_integer()
        .and_then(|value| i32::try_from(value).ok());
    let (Some(uid), Some(gid)) = (uid, gid) else {
        return Ok(BifResult::badarg());
    };
    let atoms = &mut scheduler.atoms;
    let Some(path) = marshal_path(path) else {
        return error_tuple(atoms, "einval");
    };
    value_or_error(
        atoms,
        file::set_owner(path, uid, gid).map(|()| Term::static_atom("ok")),
    )
}

/// Run `prim_file:set_time_nif(Path, ATime, MTime, CTime)`, as the ERTS NIF
/// `set_time_nif` (`prim_file_nif.c:1093-1113`) does: a time that is not an
/// integer in the `Sint64` range raises `error:badarg` before the path
/// check, a path that [`marshal_path`] does not accept gives
/// `{error, einval}`, and else [`file::set_time`] gives `ok` or
/// `{error, Errno}`. ERTS ignores `CTime`.
///
/// # Examples
///
/// ```rust,ignore
/// // {error, enoent}
/// let result = set_time_nif(&mut scheduler, &Term::binary(b"/pbeam-missing\0".to_vec()), &Term::Integer(0), &Term::Integer(0), &Term::Integer(0))?;
/// ```
///
/// # Errors
///
/// A new result atom in a full atom table gives the error of
/// `AtomTable::put`.
fn set_time_nif(
    scheduler: &mut Scheduler,
    path: &Term,
    atime: &Term,
    mtime: &Term,
    ctime: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Integer(atime), Term::Integer(mtime), Term::Integer(_)) = (atime, mtime, ctime)
    else {
        return Ok(BifResult::badarg());
    };
    let atoms = &mut scheduler.atoms;
    let Some(path) = marshal_path(path) else {
        return error_tuple(atoms, "einval");
    };
    value_or_error(
        atoms,
        file::set_time(path, *atime, *mtime).map(|()| Term::static_atom("ok")),
    )
}

/// Return `{ok, Value}` for `Ok(Value)`, as `enif_make_tuple2(env, am_ok,
/// Result)` makes it, or the [`error_tuple`] of the `errno` of `Err`.
///
/// # Examples
///
/// ```rust,ignore
/// // {ok, <<>>}
/// let result = ok_tuple(&mut atoms, Ok(Term::binary(vec![])))?;
/// // {error, enoent}
/// let result = ok_tuple(&mut atoms, Err(libc::ENOENT))?;
/// ```
///
/// # Errors
///
/// A new error atom in a full atom table gives the error of
/// `AtomTable::put`.
fn ok_tuple(atoms: &mut AtomTable, result: Result<Term, c_int>) -> Result<BifResult, BeamError> {
    value_or_error(
        atoms,
        result.map(|value| Term::tuple(vec![Term::static_atom("ok"), value])),
    )
}

/// Return the value of `Ok`, or the [`error_tuple`] of the `errno` of
/// `Err`.
///
/// # Examples
///
/// ```rust,ignore
/// // ok
/// let result = value_or_error(&mut atoms, Ok(Term::static_atom("ok")))?;
/// // {error, eio}
/// let result = value_or_error(&mut atoms, Err(libc::EIO))?;
/// ```
///
/// # Errors
///
/// A new error atom in a full atom table gives the error of
/// `AtomTable::put`.
fn value_or_error(
    atoms: &mut AtomTable,
    result: Result<Term, c_int>,
) -> Result<BifResult, BeamError> {
    match result {
        Ok(value) => Ok(BifResult::Value(value)),
        Err(errno) => error_tuple(atoms, &errno_id(errno)),
    }
}

/// Return the path of a `Path` argument, as `efile_marshal_path`
/// (`unix_prim_file.c:91-101`) accepts it: a binary with a NUL byte, whose
/// bytes after the first NUL are all NUL. Return `None` for each other
/// term, also for `<<>>`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(marshal_path(&Term::binary(b"/tmp\0\0".to_vec())), Some(c"/tmp"));
/// assert_eq!(marshal_path(&Term::binary(b"/tmp".to_vec())), None);
/// assert_eq!(marshal_path(&Term::binary(b"/a\0b\0".to_vec())), None);
/// ```
fn marshal_path(path: &Term) -> Option<&CStr> {
    let Term::Binary(bytes) = path else {
        return None;
    };
    let path = CStr::from_bytes_until_nul(bytes).ok()?;
    let rest = &bytes[path.to_bytes().len()..];
    rest.iter().all(|&byte| byte == 0).then_some(path)
}

/// Return `{error, Reason}`, as `posix_error_to_tuple`
/// (`prim_file_nif.c:306-309`) does.
///
/// # Examples
///
/// ```rust,ignore
/// // {error, enoent}
/// assert!(matches!(error_tuple(&mut atoms, "enoent")?, BifResult::Value(Term::Tuple(_))));
/// ```
///
/// # Errors
///
/// The atom `Reason` in a full atom table gives the error of
/// `AtomTable::put`.
fn error_tuple(atoms: &mut AtomTable, reason: &str) -> Result<BifResult, BeamError> {
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("error"),
        Term::Atom(atoms.put(reason)?),
    ])))
}

/// Return the `#file_info` tuple of `info`, as `build_file_info`
/// (`prim_file_nif.c:996-1017`) makes it: the size, the type atom of
/// `efile_filetype_to_atom`, the access atom of `efile_access_to_atom`
/// (`prim_file_nif.c:476-498`), the three times between [`MIN_FILETIME`]
/// and [`MAX_FILETIME`], and the integer fields.
///
/// # Examples
///
/// ```rust,ignore
/// let info = file::read_info(c"/bin/sh", true).unwrap();
/// assert!(matches!(file_info(&info), BifResult::Value(Term::Tuple(fields)) if fields.len() == 14));
/// ```
fn file_info(info: &FileInfo) -> BifResult {
    let file_type = match info.file_type {
        FileType::Device => Atom::DEVICE,
        FileType::Directory => Atom::DIRECTORY,
        FileType::Regular => Atom::REGULAR,
        FileType::Symlink => Atom::SYMLINK,
        FileType::Other => Atom::OTHER,
    };
    let access = match (info.readable, info.writable) {
        (true, false) => Atom::READ,
        (false, true) => Atom::WRITE,
        (true, true) => Atom::READ_WRITE,
        (false, false) => Atom::from_static("none").expect("none is a static atom"),
    };
    let time = |seconds: i64| Term::Integer(seconds.clamp(MIN_FILETIME, MAX_FILETIME));
    let integer = |value: u32| Term::Integer(value.into());
    BifResult::Value(Term::tuple(vec![
        Term::Atom(Atom::FILE_INFO),
        Term::Integer(info.size),
        Term::Atom(file_type),
        Term::Atom(access),
        time(info.atime),
        time(info.mtime),
        time(info.ctime),
        integer(info.mode),
        integer(info.links),
        integer(info.major_device),
        integer(info.minor_device),
        integer(info.inode),
        integer(info.uid),
        integer(info.gid),
    ]))
}
