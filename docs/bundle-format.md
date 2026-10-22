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

The `pbeam_bundle` tool has four commands. `pbeam_bundle bundle` writes the bundle of a release directory with the start set. `pbeam_bundle fixture` writes the bundle of a fixture example, which has no release directory. `pbeam_bundle executable` writes one executable: the bytes of the `pbeam` binary, the bundle, and a trailer. The trailer is 16 bytes: the magic `PBEAMEMB`, then the length of the bundle as a little-endian `u64`. A `pbeam` run with no argument finds the trailer at the end of its own file, parses the bundle that the trailer names, extracts each resource to a new temporary directory, and replaces the process with the boot of the extracted release, whose directory is its OTP root. A release with the boot script `releases/<Vsn>/start.boot`, which `mix release` writes, boots with `pbeam erl --root <dir> --bindir <dir>/erts-<Erts>/bin --preloaded <dir>/lib/erts-<Erts>/ebin`, with `<Erts>` and `<Vsn>` from `releases/start_erl.data`. The `erl` arguments are those of `bin/<app> start` with `RELEASE_DISTRIBUTION=none`, with `-s <entry_module> <entry_function>` of the entry MFA and without `-setcookie` and `-args_file`: `-noshell -s elixir start_cli -mode embedded -config <dir>/releases/<Vsn>/sys -boot <dir>/releases/<Vsn>/start -boot_var RELEASE_LIB <dir>/lib -extra --no-halt` for the entry `elixir:start_cli` of `mix pbeam.bundle`. The run gets `RELEASE_ROOT` and `RELEASE_SYS_CONFIG`. The bundle keeps no file mode, so the launcher gives mode `0755` to each file of `erts-<Erts>/bin/`, from which the boot starts port programs such as `inet_gethost`. Each other bundle, such as a fixture bundle, boots with `pbeam_boot start` of the entry. The replacement keeps the process identifier, so a signal reaches the boot as in a normal run. The boot removes the extracted directory at its exit; the launcher gives it the directory with the flag `--extracted-root`, so a program that the run starts does not inherit it. The launcher creates the directory with mode `0700`. The bundle carries the release of the application and the modules of the start set, so the run needs no other file.

A fixture example builds with `scripts/example.sh`, not with `mix release`. The `fixture` command reads the start set, the beam list, and the consolidated directory of that build. It writes the example modules and `pbeam_boot.beam` as `pbeam/<file>`, and the consolidated protocols as `releases/pbeam/consolidated/<file>`. A module of an OTP or Elixir lib directory keeps its path from the lib directory. The command writes all files of each such ebin directory, because the run loads each other module on demand, as a release carries its applications. The resource `meta/start-modules` names each start module except the entry module and `pbeam_boot`, which the launcher adds. The resource `releases/start_erl.data` is synthetic and names the version `pbeam`, so the launcher finds the same layout as the layout of a release.

The `mix pbeam.build` task of `examples/plug_hello` runs the `bundle` and `executable` commands with the release binaries that Cargo builds, and writes `<app>.bundle` under `target/examples/`. The mise task `mise run example-executable <name>` writes the one executable of a release example or a fixture example. The build targets the host: macOS arm64 on the development host and Linux x86_64 in CI. The build needs Cargo and the pinned OTP; the run needs neither. On macOS the linker signs the `pbeam` binary ad hoc, and `codesign --verify --strict` passes for it. The appended bundle lies outside the signed bytes: the appended executable keeps the ad-hoc code directory and runs, and `codesign --verify --strict` stops with `main executable failed strict validation`.

The NIF manifest gate is a build check, not a runtime check. It loads each BEAM file and compares its NIF references with the NIF libraries of PBeam: the static libraries and the registered extension libraries. PBeam selects the library by the module of the caller, as ERTS does for a static NIF library (`beam/erl_nif.c:4743-4757`), so a call of `erlang:load_nif/2` must be in a module that has a library. In a module with a library, the path of the call can be any term; thus `crypto`, which builds its path at run time (`crypto.erl:4479`, `4499`), passes. In a module without a library, the path must be a literal string, so that the gate can name the library; the compiler folds `atom_to_list(?MODULE)` to such a literal. The module `pubkey_os_cacerts` has no PBeam library, and its `on_load` calls `erlang:load_nif/2` only when `os:type()` is `{win32, _}` (`pubkey_os_cacerts.erl:265-275`). PBeam builds a bundle only for the host, and does not support Windows, so the gate reports no item for that call on a host that is not Windows. Each function that starts with `nif_start` and is in the library of its module must have a binding. Each missing item is one line, such as `prim_tty:isprint/1: NIF function has no PBeam binding`.

The `pbeam_bundle check-nifs <directory>` command runs the gate on each BEAM file under the directory, prints each item, and stops with status 1 when there is an item. The `pbeam_bundle bundle` command runs the gate on each BEAM file of the bundle before it writes the output. Its optional last argument is a NIF allowlist: a text file of the unbound `library:function/arity` items that the build accepts, each with a reason. A line `reason: <text>` gives the reason of each item that follows it, up to the next reason line. Each other line that is not empty is one item. An item before a reason with text is an error. The build prints each accepted item with its reason, as `pbeam_bundle: the NIF manifest gate accepts crypto:info_fips/0: <reason>`. An item that is not in the list, a library without bindings, a path that is not a literal string, and an item of the list that the bundle does not have stop the build with status 1, and the build writes no output. The `plug_hello` example keeps its list in `examples/plug_hello/nif-allowlist.txt`, which `mix pbeam.bundle`, `mix pbeam.build`, and `mise run example-executable plug_hello` give to the build. The `fixture` command does not run the gate.

A trailer with a bundle length that does not fit before the trailer is an error, and the run stops with status 1. Without the magic at the end of the file, the binary has no bundle, and its command line runs as usual.

The resource `meta/start-modules` names the modules of the start set, separated by ASCII whitespace. For a release, the start set is the preloaded modules of `erlang:pre_loaded/0`, which `scripts/example.sh --write-start` lists, and the release boot reads them from `lib/erts-<Erts>/ebin`. For a fixture bundle, the run loads `pbeam/<entry_module>.beam` and `pbeam/pbeam_boot.beam` first, then each module of the resource. A module resolves in `pbeam/`, in `releases/*/consolidated/`, in `erts-*/ebin`, or in `lib/*/ebin` of the extracted release, in this order. Thus a consolidated protocol comes before the unconsolidated protocol of the same name in a lib directory.
