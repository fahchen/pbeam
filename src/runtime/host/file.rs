//! Read the information of a host file, as the ERTS `prim_file` NIF library
//! reads it on Unix (`nifs/unix/unix_prim_file.c`).

use std::ffi::{CStr, c_int};

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
