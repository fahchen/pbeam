//! Read host files, their information, directories, symbolic links, and the
//! current directory, and open, read, write, and seek host files, as the
//! ERTS `prim_file` NIF library does on Unix (`nifs/unix/unix_prim_file.c`).

use std::{
    ffi::{CStr, OsStr, c_int},
    fs::{self, File, OpenOptions},
    io::{self, IoSlice, Read, Seek, SeekFrom, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::Path,
};

use nix::sys::{stat::utimes, time::TimeVal};

use super::errno::errno;

/// The type of a host file, as `efile_filetype_t`
/// (`nifs/common/prim_file_nif.h:56-62`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileType {
    /// A character or block device.
    Device,
    /// A directory.
    Directory,
    /// A regular file.
    Regular,
    /// A symbolic link. Only a read that does not follow links gives it.
    Symlink,
    /// Each other type, for example a FIFO or a socket.
    Other,
}

/// The information of a host file, as `efile_fileinfo_t`
/// (`nifs/common/prim_file_nif.h:80-95`) holds it. ERTS keeps the mode, the
/// link count, the device numbers, the inode, the user, and the group as
/// `Uint32`, so each of them keeps the low 32 bits of its `stat` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileInfo {
    /// `st_size`: the size in bytes.
    pub(crate) size: i64,
    /// The type from `st_mode`.
    pub(crate) file_type: FileType,
    /// `true` when `access` with `R_OK` succeeds.
    pub(crate) readable: bool,
    /// `true` when `access` with `W_OK` succeeds.
    pub(crate) writable: bool,
    /// `st_atime`: the last access, in seconds since 1970.
    pub(crate) atime: i64,
    /// `st_mtime`: the last change of the contents, in seconds since 1970.
    pub(crate) mtime: i64,
    /// `st_ctime`: the last change of the inode, in seconds since 1970.
    pub(crate) ctime: i64,
    /// `st_mode`: the file type and the permission bits.
    pub(crate) mode: u32,
    /// `st_nlink`: the number of hard links.
    pub(crate) links: u32,
    /// `st_dev`: the device of the file system.
    pub(crate) major_device: u32,
    /// `st_rdev`: the device that a device file names.
    pub(crate) minor_device: u32,
    /// `st_ino`: the inode number.
    pub(crate) inode: u32,
    /// `st_uid`: the user of the owner.
    pub(crate) uid: u32,
    /// `st_gid`: the group of the owner.
    pub(crate) gid: u32,
}

/// Return the information of the file at `path`, as `efile_read_info`
/// (`unix_prim_file.c:739-769`) gives it on a host without `NO_ACCESS`. With
/// `follow_links`, `stat` reads the file that a symbolic link names, else
/// `lstat` reads the link itself. A failed call gives its `errno`. Then
/// `access` with `R_OK` and with `W_OK` gives the access of the caller; it
/// always follows a symbolic link. The type and the fields come from
/// `build_file_info` (`unix_prim_file.c:712-737`).
///
/// # Examples
///
/// ```rust,ignore
/// let info = read_info(c"/bin/sh", true)?;
/// assert_eq!(info.file_type, FileType::Regular);
/// assert_eq!(read_info(c"/pbeam-missing", true), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of `stat` or `lstat`, for example `ENOENT` or `ENOTDIR`.
// The `stat` field types differ between hosts, for example `st_mode` is
// `u16` on macOS and `u32` on Linux, so a cast can have the same type.
#[allow(clippy::unnecessary_cast)]
pub(crate) fn read_info(path: &CStr, follow_links: bool) -> Result<FileInfo, c_int> {
    // SAFETY: all zeros is a valid `stat`. `path` is a NUL-terminated
    // string, and `stat` and `lstat` write only into the struct that the
    // second pointer gives.
    let (result, data) = unsafe {
        let mut data: libc::stat = std::mem::zeroed();
        let result = if follow_links {
            libc::stat(path.as_ptr(), &raw mut data)
        } else {
            libc::lstat(path.as_ptr(), &raw mut data)
        };
        (result, data)
    };
    if result < 0 {
        return Err(errno());
    }
    // SAFETY: `path` is a NUL-terminated string, and `access` only reads it.
    let (readable, writable) = unsafe {
        (
            libc::access(path.as_ptr(), libc::R_OK) == 0,
            libc::access(path.as_ptr(), libc::W_OK) == 0,
        )
    };
    let file_type = match data.st_mode & libc::S_IFMT {
        libc::S_IFCHR | libc::S_IFBLK => FileType::Device,
        libc::S_IFDIR => FileType::Directory,
        libc::S_IFREG => FileType::Regular,
        libc::S_IFLNK => FileType::Symlink,
        _ => FileType::Other,
    };
    Ok(FileInfo {
        size: data.st_size as i64,
        file_type,
        readable,
        writable,
        atime: data.st_atime as i64,
        mtime: data.st_mtime as i64,
        ctime: data.st_ctime as i64,
        mode: data.st_mode as u32,
        links: data.st_nlink as u32,
        major_device: data.st_dev as u32,
        minor_device: data.st_rdev as u32,
        inode: data.st_ino as u32,
        uid: data.st_uid as u32,
        gid: data.st_gid as u32,
    })
}

/// Return the contents of the regular file at `path`, as `read_file_nif`
/// (`prim_file_nif.c:1354-1387`) reads them on Unix:
///
/// 1. `open` with `O_RDONLY` (`efile_open` and `get_flags`,
///    `unix_prim_file.c:142-226`). A failed call gives its `errno`.
/// 2. A directory gives `EISDIR` (`open_file_is_dir`, `unix_prim_file.c:126-140`).
/// 3. `fstat` gives the size (`efile_read_handle_info`,
///    `unix_prim_file.c:803-818`). A failed `fstat` or the size 0 reads to
///    the end of the file. Else the read stops after `st_size` bytes or at
///    the end of the file (`read_file`, `prim_file_nif.c:1300-1352`). A
///    failed read gives its `errno`.
///
/// PBeam opens the file with `O_CLOEXEC` too, which only closes it in a
/// child that starts while the read runs.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(read_file(c"/dev/null"), Ok(vec![]));
/// assert_eq!(read_file(c"/"), Err(libc::EISDIR));
/// assert_eq!(read_file(c"/pbeam-missing"), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of `open` or `read`, `EISDIR` for a directory, and `ENOMEM`
/// when the buffer cannot grow, as `enif_realloc_binary` gives it.
pub(crate) fn read_file(path: &CStr) -> Result<Vec<u8>, c_int> {
    let mut file = File::open(host_path(path)).map_err(os_errno)?;
    let metadata = file.metadata().ok();
    if metadata.as_ref().is_some_and(fs::Metadata::is_dir) {
        return Err(libc::EISDIR);
    }
    let size = metadata.map_or(0, |metadata| metadata.len());
    let mut contents = Vec::new();
    let read = if size == 0 {
        file.read_to_end(&mut contents)
    } else {
        file.take(size).read_to_end(&mut contents)
    };
    read.map_err(os_errno)?;
    Ok(contents)
}

/// Return the names of the entries of the directory at `path` in the order
/// of `readdir`, without `.` and `..`, as `efile_list_dir`
/// (`unix_prim_file.c:897-943`) reads them. A failed `opendir` gives its
/// `errno`. As in ERTS, a failed `readdir` ends the list.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(list_dir(c"/")?.contains(&b"tmp".to_vec()));
/// assert_eq!(list_dir(c"/pbeam-missing"), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of `opendir`, for example `ENOENT` or `ENOTDIR`.
pub(crate) fn list_dir(path: &CStr) -> Result<Vec<Vec<u8>>, c_int> {
    Ok(fs::read_dir(host_path(path))
        .map_err(os_errno)?
        .map_while(Result::ok)
        .map(|entry| entry.file_name().into_vec())
        .collect())
}

/// Return the contents of the symbolic link at `path`, as `efile_read_link`
/// (`unix_prim_file.c:859-895`) reads it: `readlink` into a buffer of 256
/// bytes, which doubles while the contents fill it.
///
/// `readlink` that gives 0 bytes, for the empty link that macOS allows,
/// gives the `errno` that an earlier call of the thread left, as in ERTS.
///
/// # Examples
///
/// ```rust,ignore
/// // /tmp/link is a symbolic link to "target".
/// assert_eq!(read_link(c"/tmp/link"), Ok(b"target".to_vec()));
/// assert_eq!(read_link(c"/"), Err(libc::EINVAL));
/// ```
///
/// # Errors
///
/// The `errno` of `readlink`, for example `EINVAL` for a file that is not
/// a symbolic link.
pub(crate) fn read_link(path: &CStr) -> Result<Vec<u8>, c_int> {
    let mut contents = vec![0; 256];
    loop {
        // SAFETY: `path` is a NUL-terminated string, and `readlink` writes
        // at most `contents.len()` bytes into `contents`.
        let copied =
            unsafe { libc::readlink(path.as_ptr(), contents.as_mut_ptr().cast(), contents.len()) };
        let Some(copied) = usize::try_from(copied).ok().filter(|&copied| copied > 0) else {
            return Err(errno());
        };
        if copied < contents.len() {
            contents.truncate(copied);
            return Ok(contents);
        }
        contents.resize(contents.len() * 2, 0);
    }
}

/// Return the current directory of the OS process, as `efile_get_cwd`
/// (`unix_prim_file.c:1050-1083`) reads it with `getcwd`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(get_cwd()?, std::env::current_dir()?.into_os_string().into_vec());
/// ```
///
/// # Errors
///
/// The `errno` of `getcwd` other than `ERANGE`, for example `ENOENT` when
/// the directory was removed.
pub(crate) fn get_cwd() -> Result<Vec<u8>, c_int> {
    Ok(std::env::current_dir()
        .map_err(os_errno)?
        .into_os_string()
        .into_vec())
}

/// The modes of an open, as the `efile_modes_t` flags of
/// `efile_translate_modelist` (`prim_file_nif.c:500-539`). An open has
/// `read`, `write`, or both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OpenModes {
    /// `EFILE_MODE_READ`.
    pub(crate) read: bool,
    /// `EFILE_MODE_WRITE`.
    pub(crate) write: bool,
    /// `EFILE_MODE_APPEND`.
    pub(crate) append: bool,
    /// `EFILE_MODE_EXCLUSIVE`.
    pub(crate) exclusive: bool,
    /// `EFILE_MODE_SYNC`.
    pub(crate) sync: bool,
    /// `EFILE_MODE_SKIP_TYPE_CHECK`.
    pub(crate) skip_type_check: bool,
    /// `EFILE_MODE_DIRECTORY`.
    pub(crate) directory: bool,
}

/// Open the file at `path` with `modes`, as `efile_open`
/// (`unix_prim_file.c:173-226`) does:
///
/// 1. The flags of `get_flags` (`unix_prim_file.c:142-171`): `O_RDONLY`
///    for `read` alone, `O_WRONLY | O_CREAT | O_TRUNC` for `write` alone,
///    and `O_RDWR | O_CREAT` for both. `append` removes `O_TRUNC` and adds
///    `O_APPEND`, `exclusive` adds `O_EXCL`, `sync` adds `O_SYNC`, and
///    `directory` adds `O_DIRECTORY`.
/// 2. `open` with the mode `0777` for `directory`, else `0666`, again on
///    `EINTR`. A failed call gives its `errno`.
/// 3. Without `skip_type_check` and `directory`, a directory gives `EISDIR`
///    (`open_file_is_dir`, `unix_prim_file.c:126-140`).
///
/// PBeam opens the file with `O_CLOEXEC` too, as [`read_file`] does.
///
/// # Examples
///
/// ```rust,ignore
/// let read = OpenModes { read: true, ..OpenModes::default() };
/// assert!(open(c"/dev/null", read).is_ok());
/// assert_eq!(open(c"/", read).err(), Some(libc::EISDIR));
/// ```
///
/// # Errors
///
/// The `errno` of `open`, or `EISDIR` for a directory.
pub(crate) fn open(path: &CStr, modes: OpenModes) -> Result<File, c_int> {
    let mut options = OpenOptions::new();
    if !modes.write {
        options.read(true);
    } else if !modes.read {
        options.write(true).create(true).truncate(!modes.append);
    } else {
        options.read(true).write(true).create(true);
    }
    // The standard library adds `O_CREAT | O_EXCL` for `create_new`.
    options.append(modes.append).create_new(modes.exclusive);
    let mut flags = 0;
    if modes.sync {
        flags |= libc::O_SYNC;
    }
    if modes.directory {
        flags |= libc::O_DIRECTORY;
    }
    options
        .custom_flags(flags)
        .mode(if modes.directory { 0o777 } else { 0o666 });
    let file = options.open(host_path(path)).map_err(os_errno)?;
    if !modes.skip_type_check
        && !modes.directory
        && file.metadata().is_ok_and(|metadata| metadata.is_dir())
    {
        return Err(libc::EISDIR);
    }
    Ok(file)
}

/// Read at most `size` bytes of `file`, as `read_nif` with `efile_readv`
/// (`prim_file_nif.c:649-686`, `unix_prim_file.c:292-341`) does: the
/// buffer of `size` bytes first, then reads until the buffer is full or a
/// read gives 0 bytes, again on `EINTR`. A `size` of 0 reads once into an
/// empty buffer, so a file without read access gives `EBADF`. The result keeps only the bytes
/// that it read, as `enif_realloc_binary` does.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(read(&File::open("/dev/null")?, 10), Ok(vec![]));
/// ```
///
/// # Errors
///
/// `ENOMEM` when the buffer cannot be allocated, as `enif_alloc_binary`
/// gives it, or the `errno` of a failed read, also after some bytes.
pub(crate) fn read(mut file: &File, size: u64) -> Result<Vec<u8>, c_int> {
    if size == 0 {
        // `readv` with one empty buffer still checks the descriptor, so a
        // file without read access gives `EBADF`.
        return loop {
            match file.read(&mut []) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                result => break result.map(|_| Vec::new()).map_err(os_errno),
            }
        };
    }
    let mut contents = Vec::new();
    usize::try_from(size)
        .ok()
        .and_then(|size| contents.try_reserve_exact(size).ok())
        .ok_or(libc::ENOMEM)?;
    file.take(size)
        .read_to_end(&mut contents)
        .map_err(os_errno)?;
    contents.shrink_to_fit();
    Ok(contents)
}

/// Write all bytes of `buffers` to `file`, as `write_nif` with
/// `efile_writev` (`prim_file_nif.c:688-710`, `unix_prim_file.c:343-394`)
/// does with [`write_vectored`]. The
/// buffers refer to the bytes of the binaries, as the iovec of
/// `enif_inspect_iovec` does, so the write copies no byte.
///
/// # Examples
///
/// ```rust,ignore
/// let mut buffers = [IoSlice::new(b"a"), IoSlice::new(b"b")];
/// assert_eq!(write(&OpenOptions::new().write(true).open("/dev/null")?, &mut buffers), Ok(()));
/// ```
///
/// # Errors
///
/// The `errno` of a failed write.
pub(crate) fn write(file: &File, buffers: &mut [IoSlice<'_>]) -> Result<(), c_int> {
    write_vectored(file, buffers, FALLBACK_RW_LENGTH).map_err(os_errno)
}

/// The largest write of the fallback of `efile_writev` after `EINVAL`, as
/// `FALLBACK_RW_LENGTH` (`unix_prim_file.c:48`): macOS gives `EINVAL` for a
/// `write` of more than 2 GB.
const FALLBACK_RW_LENGTH: usize = (1 << 31) - 1;

/// Write all bytes of `buffers` to `writer` with `Write::write_vectored`,
/// as the loop of `efile_writev` (`unix_prim_file.c:343-394`) does:
///
/// 1. `writev` of the buffers. When it gives `EINVAL`, one `write` of the
///    first buffer, and when that gives `EINVAL` too for a buffer of more
///    than `fallback_length` bytes, one `write` of its first
///    `fallback_length` bytes (`unix_prim_file.c:360-376`).
/// 2. After a partial write, the same again with the bytes after the
///    written bytes, and again on `EINTR`, until all bytes are written or a
///    write gives 0 bytes.
///
/// `write` passes [`FALLBACK_RW_LENGTH`] as `fallback_length`.
///
/// # Examples
///
/// ```rust,ignore
/// let mut buffers = [IoSlice::new(b"ab")];
/// write_vectored(&mut Vec::new(), &mut buffers, FALLBACK_RW_LENGTH)?;
/// ```
///
/// # Errors
///
/// The error of a failed write.
fn write_vectored(
    mut writer: impl Write,
    mut buffers: &mut [IoSlice<'_>],
    fallback_length: usize,
) -> io::Result<()> {
    let invalid = |error: &io::Error| error.raw_os_error() == Some(libc::EINVAL);
    let mut more = true;
    while more && !buffers.is_empty() {
        let result = match writer.write_vectored(buffers) {
            Err(error) if invalid(&error) => match writer.write(&buffers[0]) {
                Err(error) if invalid(&error) && buffers[0].len() > fallback_length => {
                    writer.write(&buffers[0][..fallback_length])
                }
                result => result,
            },
            result => result,
        };
        match result {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
            Ok(written) => {
                // `efile_writev` stops at a write of 0 bytes and gives the
                // bytes that it wrote (unix_prim_file.c:382-390).
                more = written > 0;
                IoSlice::advance_slices(&mut buffers, written);
            }
        }
    }
    Ok(())
}

/// Close `file`, as `efile_close` (`unix_prim_file.c:246-265`) does: one
/// `close`, with no new call on `EINTR`, because `close` either always
/// closes the descriptor or leaves it undefined.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(close(File::open("/dev/null")?), Ok(()));
/// ```
///
/// # Errors
///
/// The `errno` of a failed `close`, for example `EIO` for a write that an
/// NFS server rejects at the close.
pub(crate) fn close(file: File) -> Result<(), c_int> {
    nix::unistd::close(file).map_err(|errno| errno as c_int)
}

/// Move the position of `file` to `offset` from `from`, and return the new
/// position, as `efile_seek` (`unix_prim_file.c:508-540`) does with
/// `lseek`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(seek(&file, SeekFrom::Start(0)), Ok(0));
/// ```
///
/// # Errors
///
/// The `errno` of `lseek`, for example `EINVAL` for a negative position.
pub(crate) fn seek(mut file: &File, from: SeekFrom) -> Result<u64, c_int> {
    file.seek(from).map_err(os_errno)
}

/// Delete the file at `path`, as `efile_del_file`
/// (`nifs/unix/unix_prim_file.c:991-1002`) does with `unlink`. Linux gives
/// `EISDIR` for a directory, and ERTS gives `EPERM` there instead; macOS
/// gives `EPERM` directly.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(delete_file(c"/pbeam-missing"), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of `unlink`, with the Linux `EISDIR` that ERTS maps to
/// `EPERM`.
pub(crate) fn delete_file(path: &CStr) -> Result<(), c_int> {
    match fs::remove_file(host_path(path)) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc::EISDIR) => Err(libc::EPERM),
        Err(error) => Err(os_errno(error)),
    }
}

/// Delete the directory at `path`, as `efile_del_dir`
/// (`nifs/unix/unix_prim_file.c:1004-1032`) does with `rmdir`:
/// `ENOTEMPTY` becomes `EEXIST`, and a directory that is the current
/// directory gives `EINVAL`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(delete_dir(c"/pbeam-missing"), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of `rmdir`, with the `ENOTEMPTY` that ERTS maps to
/// `EEXIST`, and `EINVAL` for the current directory.
pub(crate) fn delete_dir(path: &CStr) -> Result<(), c_int> {
    let Err(error) = fs::remove_dir(host_path(path)) else {
        return Ok(());
    };
    let mut errno = os_errno(error);
    if errno == libc::ENOTEMPTY {
        errno = libc::EEXIST;
    }
    if errno == libc::EEXIST
        && let (Ok(directory), Ok(cwd)) =
            (fs::metadata(host_path(path)), fs::metadata(host_path(c".")))
        && directory.ino() == cwd.ino()
        && directory.dev() == cwd.dev()
    {
        return Err(libc::EINVAL);
    }
    Err(errno)
}

/// Create the directory at `path` with the mode `0777` of `DIR_MODE`, as
/// `efile_make_dir` (`nifs/unix/unix_prim_file.c:979-989`) does with
/// `mkdir`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(make_dir(c"/pbeam-missing/dir"), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of `mkdir`.
pub(crate) fn make_dir(path: &CStr) -> Result<(), c_int> {
    fs::create_dir(host_path(path)).map_err(os_errno)
}

/// Make the hard link `new` to `existing`, as `efile_make_hard_link`
/// (`nifs/unix/unix_prim_file.c:963-969`) does with `link`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(make_hard_link(c"/pbeam-missing", c"/tmp/new"), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of `link`.
pub(crate) fn make_hard_link(existing: &CStr, new: &CStr) -> Result<(), c_int> {
    fs::hard_link(host_path(existing), host_path(new)).map_err(os_errno)
}

/// Make the symbolic link `new` to `existing`, as `efile_make_soft_link`
/// (`nifs/unix/unix_prim_file.c:971-977`) does with `symlink`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(make_soft_link(c"target", c"/pbeam-missing/link"), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of `symlink`.
pub(crate) fn make_soft_link(existing: &CStr, new: &CStr) -> Result<(), c_int> {
    std::os::unix::fs::symlink(host_path(existing), host_path(new)).map_err(os_errno)
}

/// Rename `old` to `new`, as `efile_rename`
/// (`nifs/unix/unix_prim_file.c:945-961`) does with `rename`:
/// `ENOTEMPTY` becomes `EEXIST`, and a failed rename of `/` gives `EINVAL`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(rename(c"/pbeam-missing", c"/tmp/new"), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of `rename`, with the two changes of ERTS.
pub(crate) fn rename(old: &CStr, new: &CStr) -> Result<(), c_int> {
    let Err(error) = fs::rename(host_path(old), host_path(new)) else {
        return Ok(());
    };
    let errno = os_errno(error);
    if errno == libc::ENOTEMPTY {
        Err(libc::EEXIST)
    } else if old.to_bytes() == b"/" {
        Err(libc::EINVAL)
    } else {
        Err(errno)
    }
}

/// Set the current directory of the OS process to `path`, as
/// `efile_set_cwd` (`nifs/unix/unix_prim_file.c:1034-1040`) does with
/// `chdir`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(set_cwd(c"/pbeam-missing"), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of `chdir`.
pub(crate) fn set_cwd(path: &CStr) -> Result<(), c_int> {
    std::env::set_current_dir(host_path(path)).map_err(os_errno)
}

/// Set the permission bits of `path` to the mutable modes of
/// `permissions`, as `efile_set_permissions`
/// (`nifs/unix/unix_prim_file.c:821-834`) does with `chmod`: the mode is
/// `permissions` masked with the set-id bits and the nine permission bits.
/// When `chmod` with that mode fails, a second `chmod` clears the set-id
/// bits, and its result is the result of the call.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(set_permissions(c"/pbeam-missing", 0o644), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of the second failed `chmod`.
pub(crate) fn set_permissions(path: &CStr, permissions: u32) -> Result<(), c_int> {
    // `S_ISUID | S_ISGID | S_IRWXU | S_IRWXG | S_IRWXO`
    // (unix_prim_file.c:822). The sticky bit is not mutable.
    const MUTABLE_MODES: u32 = 0o6777;
    let modes = permissions & MUTABLE_MODES;
    let set = |modes| fs::set_permissions(host_path(path), fs::Permissions::from_mode(modes));
    match set(modes) {
        Ok(()) => Ok(()),
        Err(_) => set(modes & !0o6000).map_err(os_errno),
    }
}

/// Set the owner `uid` and the group `gid` of `path`, as
/// `efile_set_owner` (`nifs/unix/unix_prim_file.c:836-842`) does with
/// `chown`. The value -1 of an undefined owner or group leaves that field
/// unchanged.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(set_owner(c"/pbeam-missing", -1, -1), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of `chown`.
pub(crate) fn set_owner(path: &CStr, uid: i32, gid: i32) -> Result<(), c_int> {
    std::os::unix::fs::chown(host_path(path), Some(uid as u32), Some(gid as u32)).map_err(os_errno)
}

/// Set the access time `atime` and the modification time `mtime` of `path`,
/// as `efile_set_time` (`nifs/unix/unix_prim_file.c:844-857`) does with
/// `utime`. ERTS ignores the change time.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(set_time(c"/pbeam-missing", 0, 0), Err(libc::ENOENT));
/// ```
///
/// # Errors
///
/// The `errno` of `utime`.
pub(crate) fn set_time(path: &CStr, atime: i64, mtime: i64) -> Result<(), c_int> {
    let atime = TimeVal::new(atime as libc::time_t, 0);
    let mtime = TimeVal::new(mtime as libc::time_t, 0);
    utimes(host_path(path), &atime, &mtime).map_err(|errno| errno as c_int)
}

/// Return the host path of the bytes of `path`.
fn host_path(path: &CStr) -> &Path {
    Path::new(OsStr::from_bytes(path.to_bytes()))
}

/// Return the `errno` of a failed host call. The standard library gives an
/// error with no `errno` only when its buffer cannot grow, which is
/// `ENOMEM` in ERTS.
fn os_errno(error: io::Error) -> c_int {
    error.raw_os_error().unwrap_or(libc::ENOMEM)
}

#[cfg(test)]
#[path = "../../../tests/runtime/host/file.rs"]
mod tests;
