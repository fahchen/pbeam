//! Native bindings of the `prim_file` NIFs (`erts/emulator/nifs/common/prim_file_nif.c`).
//!
//! The bindings have no capability check yet: the `file` kind of the
//! capability security migration adds it (row "Capability checks" of
//! `docs/erts-differences.md`). ERTS runs each of these NIFs on a dirty IO
//! scheduler (`prim_file_nif.c:196`, `200-201`, `207`, `212`). PBeam runs
//! them on the scheduler thread, so a host call that waits stops all
//! processes until it returns (row "Dirty IO" of
//! `docs/erts-differences.md`).

use super::*;
use crate::runtime::host::{
    errno::errno_id,
    file::{self, FileInfo, FileType},
};
use std::ffi::{CStr, c_int};

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
/// assert_eq!((BINDINGS[0].name, BINDINGS[0].arity), ("get_cwd_nif", 0));
/// ```
pub(super) const BINDINGS: &[Binding] = &[
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
];

/// The smallest file time that `build_file_info` gives, in seconds since
/// 1970: `{{1902,1,1},{0,0,0}}` (`EFILE_MIN_FILETIME`,
/// `prim_file_nif.h:97-100`).
const MIN_FILETIME: i64 = -2_145_916_800;

/// The largest file time that `build_file_info` gives, in seconds since
/// 1970: `{{9999,12,31},{23,59,59}}` (`EFILE_MAX_FILETIME`,
/// `prim_file_nif.h:102-105`).
const MAX_FILETIME: i64 = 253_402_300_799;

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
/// let result = read_info_nif(&mut scheduler, &Term::Binary(b"/bin/sh\0".to_vec()), &Term::Integer(1))?;
/// assert!(matches!(result, BifResult::Value(Term::Tuple(fields)) if fields.len() == 14));
/// let result = read_info_nif(&mut scheduler, &Term::Binary(b"/bin/sh".to_vec()), &Term::Integer(1))?;
/// // {error, einval}
/// assert!(matches!(result, BifResult::Value(Term::Tuple(fields)) if fields.len() == 2));
/// assert!(matches!(read_info_nif(&mut scheduler, &Term::Binary(vec![0]), &Term::nil())?, BifResult::Raise { .. }));
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
        Ok(info) => file_info(atoms, &info),
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
/// let result = read_file_nif(&mut atoms, &Term::Binary(b"/dev/null\0".to_vec()))?;
/// // {error, eisdir}
/// let result = read_file_nif(&mut atoms, &Term::Binary(b"/\0".to_vec()))?;
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
    ok_tuple(atoms, file::read_file(path).map(Term::Binary))
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
/// let result = list_dir_nif(&mut atoms, &Term::Binary(b"/tmp/dir\0".to_vec()))?;
/// // {error, enoent}
/// let result = list_dir_nif(&mut atoms, &Term::Binary(b"/pbeam-missing\0".to_vec()))?;
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
        .map(|names| Term::list_from(names.into_iter().rev().map(Term::Binary)));
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
/// let result = read_link_nif(&mut atoms, &Term::Binary(b"/tmp/link\0".to_vec()))?;
/// // {error, einval}
/// let result = read_link_nif(&mut atoms, &Term::Binary(b"/\0".to_vec()))?;
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
    ok_tuple(atoms, file::read_link(path).map(Term::Binary))
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
    ok_tuple(atoms, file::get_cwd().map(Term::Binary))
}

/// Return `{ok, Value}` for `Ok(Value)`, as `enif_make_tuple2(env, am_ok,
/// Result)` makes it, or the [`error_tuple`] of the `errno` of `Err`.
///
/// # Examples
///
/// ```rust,ignore
/// // {ok, <<>>}
/// let result = ok_tuple(&mut atoms, Ok(Term::Binary(vec![])))?;
/// // {error, enoent}
/// let result = ok_tuple(&mut atoms, Err(libc::ENOENT))?;
/// ```
///
/// # Errors
///
/// A new error atom in a full atom table gives the error of
/// `AtomTable::put`.
fn ok_tuple(atoms: &mut AtomTable, result: Result<Term, c_int>) -> Result<BifResult, BeamError> {
    match result {
        Ok(value) => Ok(BifResult::Value(Term::tuple(vec![
            Term::static_atom("ok"),
            value,
        ]))),
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
/// assert_eq!(marshal_path(&Term::Binary(b"/tmp\0\0".to_vec())), Some(c"/tmp"));
/// assert_eq!(marshal_path(&Term::Binary(b"/tmp".to_vec())), None);
/// assert_eq!(marshal_path(&Term::Binary(b"/a\0b\0".to_vec())), None);
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
/// assert!(matches!(file_info(&mut atoms, &info)?, BifResult::Value(Term::Tuple(fields)) if fields.len() == 14));
/// ```
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`.
fn file_info(atoms: &mut AtomTable, info: &FileInfo) -> Result<BifResult, BeamError> {
    let file_type = match info.file_type {
        FileType::Device => "device",
        FileType::Directory => "directory",
        FileType::Regular => "regular",
        FileType::Symlink => "symlink",
        FileType::Other => "other",
    };
    let access = match (info.readable, info.writable) {
        (true, false) => "read",
        (false, true) => "write",
        (true, true) => "read_write",
        (false, false) => "none",
    };
    let time = |seconds: i64| Term::Integer(seconds.clamp(MIN_FILETIME, MAX_FILETIME));
    let integer = |value: u32| Term::Integer(value.into());
    Ok(BifResult::Value(Term::tuple(vec![
        Term::Atom(atoms.put("file_info")?),
        Term::Integer(info.size),
        Term::Atom(atoms.put(file_type)?),
        Term::Atom(atoms.put(access)?),
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
    ])))
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/native/prim_file.rs"]
mod tests;
