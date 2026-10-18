//! The versioned application bundle: format and compiler identity, an entry
//! MFA, a module table, a resource table, and their payloads.
//!
//! A bundle is one byte string that a launcher embeds with `include_bytes!`
//! ("Bundle and executable build" in `docs/portable-beam-spec.md`).
//! [`Bundle::parse`] borrows each module and resource from the bundle bytes,
//! so a launcher passes a module to [`BeamProgram`](crate::BeamProgram)
//! without a copy. [`BundleWriter`] makes the bytes that a build task
//! collects.
//!
//! The byte layout, the limits, the duplicate rules, and the resource lookup
//! semantics are in `docs/bundle-format.md`.

use std::collections::HashSet;

use crate::error::{BeamError, error};

/// The first eight bytes of every bundle: the ASCII text `PBEAMBND`.
const MAGIC: &[u8; 8] = b"PBEAMBND";

/// The version of the bundle container that this runtime reads and writes.
const FORMAT_VERSION: u16 = 1;

/// The OTP release of this runtime, taken from the `erlang` pin in
/// `mise.toml` by `build.rs`, as `erlang:system_info(otp_release)` gives it.
/// [`Bundle::parse`] rejects a bundle whose recorded release differs.
const OTP_RELEASE: &str = env!("PBEAM_OTP_RELEASE");

/// The largest OTP release string in a bundle.
const MAX_OTP_RELEASE_BYTES: usize = 16;

/// The largest compiler identity string in a bundle.
const MAX_COMPILER_BYTES: usize = 64;

/// The largest count of the module table.
const MAX_MODULES: u32 = 65536;

/// The largest count of the resource table.
const MAX_RESOURCES: u32 = 65536;

/// The largest byte length of a module or of an entry function name. An atom
/// holds at most `MAX_ATOM_CHARACTERS` characters
/// (`erts/emulator/beam/atom.h:29`), and each character takes at most four
/// bytes in UTF-8, so the byte limit is four times the character limit.
const MAX_ATOM_NAME_BYTES: usize = 4 * MAX_ATOM_NAME_CHARACTERS;

/// The largest character count of a module or of an entry function name.
/// ERTS limits an atom to `MAX_ATOM_CHARACTERS` characters
/// (`erts/emulator/beam/atom.h:29`).
const MAX_ATOM_NAME_CHARACTERS: usize = 255;

/// The largest name of a resource. A resource name is not an atom: the name
/// of an application resource is a short path, so it has a larger limit than
/// an atom name.
const MAX_RESOURCE_NAME_BYTES: usize = 1024;

/// The entry function MFA of a bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry<'a> {
    /// The module of the entry function.
    pub module: &'a str,
    /// The function of the entry function.
    pub function: &'a str,
    /// The arity of the entry function.
    pub arity: u32,
}

/// One named byte payload of a bundle, either a module or a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blob<'a> {
    name: &'a str,
    bytes: &'a [u8],
}

impl<'a> Blob<'a> {
    /// Return the name of this payload.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(bundle.module("pbeam_boot").unwrap().len(), 4096);
    /// assert_eq!(bundle.modules()[0].name(), "pbeam_boot");
    /// ```
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// Return the bytes of this payload. They are a slice of the bundle, so
    /// the caller copies nothing to read them.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes: &[u8] = bundle.modules()[0].bytes();
    /// ```
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

/// A parsed application bundle that borrows its payloads from the bundle
/// bytes.
#[derive(Debug)]
pub struct Bundle<'a> {
    otp_release: &'a str,
    compiler: &'a str,
    entry: Entry<'a>,
    modules: Vec<Blob<'a>>,
    resources: Vec<Blob<'a>>,
}

impl<'a> Bundle<'a> {
    /// Parse a bundle. A malformed bundle gives an error and allocates only
    /// for the entries that it reads before the error.
    ///
    /// The parse rejects a magic that is not `PBEAMBND`, a format version
    /// that is not [`FORMAT_VERSION`], an OTP release that is not
    /// `erlang:system_info(otp_release)` of this runtime, a count over
    /// [`MAX_MODULES`] or [`MAX_RESOURCES`], a name over its limit, an empty
    /// resource name, a duplicate name in one table, a truncated field, a
    /// payload that a length leaves short, and trailing bytes after the last
    /// table. An atom name, such as a module name or an entry name, may be
    /// empty.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bundle = Bundle::parse(include_bytes!("app.pbeam"))?;
    /// assert_eq!(bundle.entry().function, "main");
    /// ```
    ///
    /// # Errors
    ///
    /// The message of a [`BeamError::Runtime`] names the field that failed.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, BeamError> {
        let mut cursor = Cursor::new(bytes);
        let magic = cursor.take(MAGIC.len(), "magic")?;
        if magic != MAGIC {
            return Err(error("bundle magic is not PBEAMBND"));
        }
        let version = cursor.u16("format version")?;
        if version != FORMAT_VERSION {
            return Err(error(format!(
                "unsupported bundle format version {version}, expected {FORMAT_VERSION}"
            )));
        }
        let otp_release = cursor.text(MAX_OTP_RELEASE_BYTES, "OTP release")?;
        if otp_release != OTP_RELEASE {
            return Err(error(format!(
                "bundle OTP release {otp_release} is incompatible with this runtime, which supports {OTP_RELEASE}"
            )));
        }
        let compiler = cursor.text(MAX_COMPILER_BYTES, "compiler identity")?;
        let entry = Entry {
            module: cursor.atom_name("entry module")?,
            function: cursor.atom_name("entry function")?,
            arity: cursor.u32("entry arity")?,
        };
        let modules = read_table(
            &mut cursor,
            MAX_MODULES,
            MAX_ATOM_NAME_BYTES,
            "module",
            true,
        )?;
        let resources = read_table(
            &mut cursor,
            MAX_RESOURCES,
            MAX_RESOURCE_NAME_BYTES,
            "resource",
            false,
        )?;
        if !cursor.is_empty() {
            return Err(error(format!(
                "bundle has {} trailing bytes",
                cursor.remaining()
            )));
        }
        Ok(Self {
            otp_release,
            compiler,
            entry,
            modules,
            resources,
        })
    }

    /// Return the OTP release that the bundle records.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(bundle.otp_release(), "29");
    /// ```
    pub fn otp_release(&self) -> &'a str {
        self.otp_release
    }

    /// Return the compiler identity that the bundle records. It names the
    /// toolchain that compiled the modules, for example
    /// `elixir 1.20.4-otp-29`. The runtime records it for diagnosis and does
    /// not reject a bundle by it; the OTP release is the compatibility gate.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(bundle.compiler(), "elixir 1.20.4-otp-29");
    /// ```
    pub fn compiler(&self) -> &'a str {
        self.compiler
    }

    /// Return the entry function MFA of the bundle.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let entry = bundle.entry();
    /// assert_eq!((entry.module, entry.function, entry.arity), ("pbeam_boot", "start", 3));
    /// ```
    pub fn entry(&self) -> Entry<'a> {
        self.entry
    }

    /// Return the module table in the bundle order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let modules: Vec<&[u8]> = bundle.modules().iter().map(Blob::bytes).collect();
    /// ```
    pub fn modules(&self) -> &[Blob<'a>] {
        &self.modules
    }

    /// Return the bytes of the module `name`, or `None` when the module table
    /// has no entry with that name. The name compares byte for byte with the
    /// table names.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(bundle.module("pbeam_boot").is_some());
    /// assert!(bundle.module("other").is_none());
    /// ```
    pub fn module(&self, name: &str) -> Option<&'a [u8]> {
        find(&self.modules, name)
    }

    /// Return the resource table in the bundle order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(bundle.resources().len(), 1);
    /// ```
    pub fn resources(&self) -> &[Blob<'a>] {
        &self.resources
    }

    /// Return the bytes of the resource `name`, or `None` when the resource
    /// table has no entry with that name. The name compares byte for byte
    /// with the table names, and this function applies no path rule. A
    /// resource name is unique in a valid bundle, so the result is one
    /// payload.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(bundle.resource("Elixir.PlugHello.app"), Some(b"..." .as_slice()));
    /// assert_eq!(bundle.resource("missing"), None);
    /// ```
    pub fn resource(&self, name: &str) -> Option<&'a [u8]> {
        find(&self.resources, name)
    }

    /// Encode this bundle into its byte form. A bundle that [`Bundle::parse`]
    /// returned encodes to the bytes that parsed it, because the format is
    /// canonical: tables keep their order and no field is optional.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Bundle::parse(&bytes)?.encode(), bytes);
    /// ```
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_bundle(
            &mut out,
            self.otp_release,
            self.compiler,
            self.entry,
            &self.modules,
            &self.resources,
        );
        out
    }
}

/// A builder of a bundle. It checks each name and each limit as it adds an
/// entry, so the bytes of [`BundleWriter::encode`] always parse.
#[derive(Debug)]
pub struct BundleWriter<'a> {
    bundle: Bundle<'a>,
    module_names: HashSet<&'a str>,
    resource_names: HashSet<&'a str>,
}

impl<'a> BundleWriter<'a> {
    /// Make a writer with an empty module table and an empty resource table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut writer = BundleWriter::new("29", "elixir 1.20.4-otp-29", "pbeam_boot", "start", 3)?;
    /// writer.add_module("pbeam_boot", include_bytes!("pbeam_boot.beam"))?;
    /// ```
    ///
    /// # Errors
    ///
    /// An empty OTP release, an entry name longer than
    /// [`MAX_ATOM_NAME_BYTES`] bytes or [`MAX_ATOM_NAME_CHARACTERS`]
    /// characters, and a string over its field limit. An empty compiler
    /// identity is valid.
    pub fn new(
        otp_release: &'a str,
        compiler: &'a str,
        module: &'a str,
        function: &'a str,
        arity: u32,
    ) -> Result<Self, BeamError> {
        if otp_release.is_empty() {
            return Err(error("bundle OTP release is empty"));
        }
        check_text(otp_release, MAX_OTP_RELEASE_BYTES, "OTP release")?;
        if otp_release != OTP_RELEASE {
            return Err(error(format!(
                "bundle OTP release {otp_release} is incompatible with this runtime, which supports {OTP_RELEASE}"
            )));
        }
        check_text(compiler, MAX_COMPILER_BYTES, "compiler identity")?;
        check_atom_name(module, "entry module")?;
        check_atom_name(function, "entry function")?;
        Ok(Self {
            bundle: Bundle {
                otp_release,
                compiler,
                entry: Entry {
                    module,
                    function,
                    arity,
                },
                modules: Vec::new(),
                resources: Vec::new(),
            },
            module_names: HashSet::new(),
            resource_names: HashSet::new(),
        })
    }

    /// Add a module payload.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// writer.add_module("pbeam_boot", bytes)?;
    /// assert!(writer.add_module("pbeam_boot", bytes).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// An over-long name, a name that the module table already holds, and a
    /// module table that is full. An empty module name is valid.
    pub fn add_module(&mut self, name: &'a str, bytes: &'a [u8]) -> Result<(), BeamError> {
        check_atom_name(name, "module name")?;
        add_blob(
            &mut self.bundle.modules,
            &mut self.module_names,
            name,
            bytes,
            MAX_MODULES,
            "module",
        )
    }

    /// Add a resource payload.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// writer.add_resource("Elixir.PlugHello.app", app_bytes)?;
    /// ```
    ///
    /// # Errors
    ///
    /// An empty or over-long name, a name that the resource table already
    /// holds, and a resource table that is full.
    pub fn add_resource(&mut self, name: &'a str, bytes: &'a [u8]) -> Result<(), BeamError> {
        check_name(name, MAX_RESOURCE_NAME_BYTES, None, "resource name")?;
        add_blob(
            &mut self.bundle.resources,
            &mut self.resource_names,
            name,
            bytes,
            MAX_RESOURCES,
            "resource",
        )
    }

    /// Return the bundle that this writer built.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(writer.bundle().entry().function, "start");
    /// ```
    pub fn bundle(&self) -> &Bundle<'a> {
        &self.bundle
    }

    /// Encode the bundle that this writer built.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bytes = writer.encode();
    /// ```
    pub fn encode(&self) -> Vec<u8> {
        self.bundle.encode()
    }
}

/// Return the bytes of the first blob with the name `name`, or `None`.
fn find<'a>(blobs: &[Blob<'a>], name: &str) -> Option<&'a [u8]> {
    blobs
        .iter()
        .find(|blob| blob.name == name)
        .map(|blob| blob.bytes)
}

/// Add one named payload to a table, with the shared checks of the module
/// table and the resource table.
fn add_blob<'a>(
    blobs: &mut Vec<Blob<'a>>,
    names: &mut HashSet<&'a str>,
    name: &'a str,
    bytes: &'a [u8],
    max_count: u32,
    kind: &str,
) -> Result<(), BeamError> {
    if blobs.len() >= max_count as usize {
        return Err(error(format!(
            "bundle {kind} table is full at {max_count} entries"
        )));
    }
    if !names.insert(name) {
        return Err(error(format!("duplicate bundle {kind} name {name}")));
    }
    blobs.push(Blob { name, bytes });
    Ok(())
}

/// Check that `text` is not longer than `max` bytes.
fn check_text(text: &str, max: usize, what: &str) -> Result<(), BeamError> {
    if text.len() > max {
        return Err(error(format!(
            "bundle {what} is {} bytes, over the limit {max}",
            text.len()
        )));
    }
    Ok(())
}

/// Check that `name` is not empty, not longer than `max_bytes` bytes, and,
/// when `max_chars` is `Some`, not longer than that many characters.
fn check_name(
    name: &str,
    max_bytes: usize,
    max_chars: Option<usize>,
    what: &str,
) -> Result<(), BeamError> {
    if name.is_empty() {
        return Err(error(format!("bundle {what} is empty")));
    }
    if name.len() > max_bytes {
        return Err(error(format!(
            "bundle {what} is {} bytes, over the limit {max_bytes}",
            name.len()
        )));
    }
    if let Some(limit) = max_chars {
        let characters = name.chars().count();
        if characters > limit {
            return Err(error(format!(
                "bundle {what} is {characters} characters, over the limit {limit}"
            )));
        }
    }
    Ok(())
}

/// Check that an atom name is not empty, not longer than
/// [`MAX_ATOM_NAME_BYTES`] bytes, and not longer than
/// [`MAX_ATOM_NAME_CHARACTERS`] characters.
fn check_atom_name(name: &str, what: &str) -> Result<(), BeamError> {
    if name.len() > MAX_ATOM_NAME_BYTES {
        return Err(error(format!(
            "bundle {what} is {} bytes, over the limit {MAX_ATOM_NAME_BYTES}",
            name.len()
        )));
    }
    let characters = name.chars().count();
    if characters > MAX_ATOM_NAME_CHARACTERS {
        return Err(error(format!(
            "bundle {what} is {characters} characters, over the limit {MAX_ATOM_NAME_CHARACTERS}"
        )));
    }
    Ok(())
}

/// Read one table: a count, then one name, one length, and one payload for
/// each entry. The name set rejects a duplicate, and each length is checked
/// against the remaining bytes before the payload is borrowed, so a declared
/// payload that is too long allocates nothing. An entry of the module table
/// has an atom name, which may be empty; an entry of the resource table has a
/// name that is not empty.
fn read_table<'a>(
    cursor: &mut Cursor<'a>,
    max_count: u32,
    max_name: usize,
    kind: &str,
    atom: bool,
) -> Result<Vec<Blob<'a>>, BeamError> {
    let count = cursor.u32(&format!("{kind} count"))?;
    if count > max_count {
        return Err(error(format!(
            "bundle {kind} count {count} is over the limit {max_count}"
        )));
    }
    let mut blobs = Vec::new();
    let mut names = HashSet::new();
    for index in 0..count {
        let name = if atom {
            cursor.atom_name(&format!("{kind} {index} name"))?
        } else {
            cursor.name(max_name, &format!("{kind} {index} name"))?
        };
        if !names.insert(name) {
            return Err(error(format!("duplicate bundle {kind} name {name}")));
        }
        let length = cursor.u64(&format!("{kind} {index} length"))?;
        let bytes = cursor.take_u64(length, &format!("{kind} {index} payload"))?;
        blobs.push(Blob { name, bytes });
    }
    Ok(blobs)
}

/// Write the header, the entry MFA, and the two tables.
fn write_bundle(
    out: &mut Vec<u8>,
    otp_release: &str,
    compiler: &str,
    entry: Entry<'_>,
    modules: &[Blob<'_>],
    resources: &[Blob<'_>],
) {
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    write_text(out, otp_release);
    write_text(out, compiler);
    write_text(out, entry.module);
    write_text(out, entry.function);
    out.extend_from_slice(&entry.arity.to_le_bytes());
    write_table(out, modules);
    write_table(out, resources);
}

/// Write a length-prefixed text.
fn write_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u16).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

/// Write a table: its count, then one name, one length, and one payload for
/// each entry.
fn write_table(out: &mut Vec<u8>, blobs: &[Blob<'_>]) {
    out.extend_from_slice(&(blobs.len() as u32).to_le_bytes());
    for blob in blobs {
        write_text(out, blob.name);
        out.extend_from_slice(&(blob.bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(blob.bytes);
    }
}

/// A read position over the bundle bytes. Each read checks the remaining
/// length first, so a short field gives an error and a declared payload that
/// is longer than the rest of the bundle borrows nothing.
struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    /// Start at the first byte.
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    /// Return the bytes that this cursor does not read yet.
    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// Return true when the cursor read every byte.
    fn is_empty(&self) -> bool {
        self.position == self.bytes.len()
    }

    /// Read `count` bytes.
    ///
    /// # Errors
    ///
    /// A `count` larger than the remaining bytes.
    fn take(&mut self, count: usize, what: &str) -> Result<&'a [u8], BeamError> {
        if count > self.remaining() {
            return Err(error(format!(
                "bundle is truncated: {what} needs {count} bytes at offset {}, but {} remain",
                self.position,
                self.remaining()
            )));
        }
        let start = self.position;
        self.position += count;
        Ok(&self.bytes[start..self.position])
    }

    /// Read `count` bytes, with a length that the caller read as a `u64`.
    ///
    /// # Errors
    ///
    /// A `count` larger than the remaining bytes.
    fn take_u64(&mut self, count: u64, what: &str) -> Result<&'a [u8], BeamError> {
        let remaining = self.remaining();
        if count > remaining as u64 {
            return Err(error(format!(
                "bundle is truncated: {what} needs {count} bytes at offset {}, but {remaining} remain",
                self.position
            )));
        }
        // `count <= remaining`, and `remaining` is a `usize`, so the cast is
        // exact on every target.
        self.take(count as usize, what)
    }

    /// Read a little-endian `u16`.
    ///
    /// # Errors
    ///
    /// Fewer than two bytes remain.
    fn u16(&mut self, what: &str) -> Result<u16, BeamError> {
        let bytes = self.take(2, what)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// Read a little-endian `u32`.
    ///
    /// # Errors
    ///
    /// Fewer than four bytes remain.
    fn u32(&mut self, what: &str) -> Result<u32, BeamError> {
        let bytes = self.take(4, what)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a little-endian `u64`.
    ///
    /// # Errors
    ///
    /// Fewer than eight bytes remain.
    fn u64(&mut self, what: &str) -> Result<u64, BeamError> {
        let bytes = self.take(8, what)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    /// Read a length-prefixed text of at most `max` bytes.
    ///
    /// # Errors
    ///
    /// A length over `max`, fewer bytes than the length, or bytes that are
    /// not UTF-8.
    fn text(&mut self, max: usize, what: &str) -> Result<&'a str, BeamError> {
        let length = usize::from(self.u16(&format!("{what} length"))?);
        if length > max {
            return Err(error(format!(
                "bundle {what} is {length} bytes, over the limit {max}"
            )));
        }
        let bytes = self.take(length, what)?;
        std::str::from_utf8(bytes).map_err(|_| error(format!("bundle {what} is not UTF-8")))
    }

    /// Read a length-prefixed non-empty text of at most `max` bytes.
    ///
    /// # Errors
    ///
    /// The errors of [`Cursor::text`], and an empty text.
    fn name(&mut self, max: usize, what: &str) -> Result<&'a str, BeamError> {
        let name = self.text(max, what)?;
        if name.is_empty() {
            return Err(error(format!("bundle {what} is empty")));
        }
        Ok(name)
    }

    /// Read a length-prefixed atom name: at most [`MAX_ATOM_NAME_BYTES`]
    /// bytes and at most [`MAX_ATOM_NAME_CHARACTERS`] characters. An empty
    /// atom name is valid, as the atom `''` is.
    ///
    /// # Errors
    ///
    /// The errors of [`Cursor::text`], a name over the byte limit, and a name
    /// over the character limit.
    fn atom_name(&mut self, what: &str) -> Result<&'a str, BeamError> {
        let name = self.text(MAX_ATOM_NAME_BYTES, what)?;
        let characters = name.chars().count();
        if characters > MAX_ATOM_NAME_CHARACTERS {
            return Err(error(format!(
                "bundle {what} is {characters} characters, over the limit {MAX_ATOM_NAME_CHARACTERS}"
            )));
        }
        Ok(name)
    }
}

#[cfg(test)]
#[path = "../tests/bundle.rs"]
mod tests;
