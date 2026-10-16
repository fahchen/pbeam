//! Read host files, their information, directories, symbolic links, and the
//! current directory, and open, read, write, and seek host files, as the
//! ERTS `prim_file` NIF library does on Unix (`nifs/unix/unix_prim_file.c`).

use std::{
    ffi::{CStr, OsStr, c_int},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::Path,
};

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

/// Write all of `bytes` to `file`, as `write_nif` with `efile_writev`
/// (`prim_file_nif.c:688-710`, `unix_prim_file.c:343-394`) does, again on
/// `EINTR` and after a partial write, until a write gives 0 bytes.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(write(&OpenOptions::new().write(true).open("/dev/null")?, b"x"), Ok(()));
/// ```
///
/// # Errors
///
/// The `errno` of a failed write.
pub(crate) fn write(mut file: &File, mut bytes: &[u8]) -> Result<(), c_int> {
    while !bytes.is_empty() {
        match file.write(bytes) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(os_errno(error)),
            // `efile_writev` stops at a write of 0 bytes and gives the bytes
            // that it wrote (unix_prim_file.c:382-390).
            Ok(written) => bytes = if written == 0 { &[] } else { &bytes[written..] },
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
