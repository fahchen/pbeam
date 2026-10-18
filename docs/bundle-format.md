# Bundle format

This document defines the byte format of the application bundle that
issue #15 of milestone M8 introduces. The runtime reads and writes this
format; the Rust types are [`Bundle`](../src/bundle.rs) and
[`BundleWriter`](../src/bundle.rs).

The format is a single contiguous byte string. Every integer is
little-endian. There is no compression in this version. Compression stays
out until a measured need justifies it.

## Magic and version

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | Magic: the ASCII bytes `PBEAMBND`. |
| 8 | 2 | `format_version`, a `u16`. This version is `1`. |

A reader rejects any other magic and any other version.

## Compatibility metadata

| Size | Field |
|---|---|
| 2 + n | `otp_release`, a length-prefixed UTF-8 string. The length is a `u16`. |
| 2 + m | `compiler`, a length-prefixed UTF-8 string. The length is a `u16`. |

`otp_release` is the OTP release that the bundle was built for, as
`erlang:system_info(otp_release)` gives it. The runtime rejects a bundle
whose value differs from its own pinned release, because the runtime
interprets the BEAM code and the OTP modules of exactly that release.

`compiler` names the toolchain that compiled the modules, for example
`elixir 1.20.4-otp-29`. The runtime records it for diagnosis and does not
reject a bundle by it. The OTP release is the compatibility gate.

## Entry MFA

| Size | Field |
|---|---|
| 2 + n | `entry_module`, a length-prefixed UTF-8 string. |
| 2 + m | `entry_function`, a length-prefixed UTF-8 string. |
| 4 | `entry_arity`, a `u32`. |

The module and the function are atoms of the loaded modules, so each is at
most 255 characters, and up to 1020 bytes in UTF-8. The arity may be any
`u32`. The bundle format does not validate the entry MFA against a module
export table; that validation is launcher work that lands with task #16.

## Module table

| Size | Field |
|---|---|
| 4 | `module_count`, a `u32`. |
| for each module: | |
| 2 + n | `module_name`, a length-prefixed UTF-8 string. |
| 8 | `payload_length`, a `u64`. |
| `payload_length` | the BEAM module bytes. |

## Resource table

| Size | Field |
|---|---|
| 4 | `resource_count`, a `u32`. |
| for each resource: | |
| 2 + n | `resource_name`, a length-prefixed UTF-8 string. |
| 8 | `payload_length`, a `u64`. |
| `payload_length` | the resource bytes. |

## Limits

| Field | Limit |
|---|---|
| `otp_release` | 16 bytes. |
| `compiler` | 64 bytes. |
| `entry_module`, `entry_function`, `module_name` | 1020 bytes and 255 characters. The character limit is `MAX_ATOM_CHARACTERS` of ERTS (`erts/emulator/beam/atom.h:29`); the byte limit is four times that, because one character takes at most four bytes in UTF-8. |
| `resource_name` | 1024 bytes. |
| `module_count`, `resource_count` | 65536. |
| `payload_length` | The remaining bytes of the bundle. A declared payload that is longer than the bytes that follow is rejected before any byte is borrowed. |

## Duplicate rules

A name appears at most once in the module table, and at most once in the
resource table. A module name may also appear as a resource name, because
the tables are separate. The writer and the reader enforce both rules.

## Trailing bytes

A bundle ends with the last payload. Any byte after it is an error.

## Resource lookup semantics

`Bundle::resource(name)` returns the payload of the entry whose name equals
`name` byte for byte, or `None` when no entry has that name. The lookup
applies no path rule, no case folding, and no normalization. A valid bundle
holds each name once, so the result is one payload or none.

`Bundle::module(name)` has the same semantics for the module table.

## Payload boundaries

Each payload is bounded by its `payload_length`. The reader borrows the
payload from the bundle bytes and copies nothing to read it. A declared
payload that is longer than the remaining bytes is rejected before the
reader borrows a byte.

## Appended executable

The `pbeam_bundle` tool writes one executable: the bytes of the `pbeam` binary, a bundle, and a trailer. The trailer is 16 bytes: the magic `PBEAMEMB`, then the length of the bundle as a little-endian `u64`. A `pbeam` run with no argument finds the trailer at the end of its own file, parses the bundle that the trailer names, extracts each resource to a new temporary directory, and starts `pbeam_boot start` with the extracted directory as the OTP root. The bundle carries the release of the application and the modules of the start set, so the run needs no other file.

The resource `meta/start-modules` names the modules of the start set, separated by ASCII whitespace. The run loads `pbeam/<entry_module>.beam` and `pbeam/pbeam_boot.beam` first, then each module of the resource, each resolved in `lib/*/ebin` or `erts-*/ebin` of the extracted release.
