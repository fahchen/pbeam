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

The `pbeam_bundle` tool has four commands. `pbeam_bundle bundle` writes the bundle of a release directory with the start set. `pbeam_bundle fixture` writes the bundle of a fixture example, which has no release directory. `pbeam_bundle executable` writes one executable: the bytes of the `pbeam` binary, the bundle, and a trailer. The trailer is 16 bytes: the magic `PBEAMEMB`, then the length of the bundle as a little-endian `u64`. A `pbeam` run with no argument finds the trailer at the end of its own file, parses the bundle that the trailer names, extracts each resource to a new temporary directory, and replaces the process with the boot of the extracted release, whose directory is its OTP root. A release with the boot script `releases/<Vsn>/start.boot`, which `mix release` writes, boots with `pbeam erl --root <dir> --bindir <dir>/erts-<Erts>/bin --preloaded <dir>/lib/erts-<Erts>/ebin`, with `<Erts>` and `<Vsn>` from `releases/start_erl.data`. The `erl` arguments are those of `bin/<app> start` with `RELEASE_DISTRIBUTION=none`, with `-s <entry_module> <entry_function>` of the entry MFA and without `-setcookie` and `-args_file`: `-noshell -s elixir start_cli -mode embedded -config <dir>/releases/<Vsn>/sys -boot <dir>/releases/<Vsn>/start -boot_var RELEASE_LIB <dir>/lib -extra --no-halt` for the entry `elixir:start_cli` of `mix pbeam.bundle`. The run gets `RELEASE_ROOT` and `RELEASE_SYS_CONFIG`. The bundle keeps no file mode, so the launcher gives mode `0755` to each file of `erts-<Erts>/bin/`, from which the boot starts port programs such as `inet_gethost`. The `bundle` command writes each file of the release directory, so the bundle ships each NIF shared library of the release, `lib/<app>-<vsn>/priv/<file>.so`, at its relative path. The launcher writes it to the same path under the extracted directory, so `code:priv_dir/1` and `erlang:load_nif/2` find it as on the pinned ERTS. `dlopen` needs only read access to the file, so the file keeps the mode of a new file. The boot removes it with the extracted directory at the exit. A shipped `.so` holds native code of the build host, so a bundle that ships one runs only on the OS and the CPU of the build host, with the shared libraries that the `.so` links there, such as the C library. Each other bundle, such as a fixture bundle, boots with `pbeam_boot start` of the entry. The replacement keeps the process identifier, so a signal reaches the boot as in a normal run. The boot removes the extracted directory at its exit; the launcher gives it the directory with the flag `--extracted-root`, so a program that the run starts does not inherit it. The launcher creates the directory with mode `0700`. The bundle carries the release of the application and the modules of the start set, so the run needs no other file.

A fixture example builds with `scripts/example.sh`, not with `mix release`. The `fixture` command reads the start set, the beam list, and the consolidated directory of that build. It writes the example modules and `pbeam_boot.beam` as `pbeam/<file>`, and the consolidated protocols as `releases/pbeam/consolidated/<file>`. A module of an OTP or Elixir lib directory keeps its path from the lib directory. The command writes all files of each such ebin directory, because the run loads each other module on demand, as a release carries its applications. The resource `meta/start-modules` names each start module except the entry module and `pbeam_boot`, which the launcher adds. The resource `releases/start_erl.data` is synthetic and names the version `pbeam`, so the launcher finds the same layout as the layout of a release.

The root Mix project `pbeam` has the tasks `mix pbeam.bundle` and `mix pbeam.build`. A Mix project that has `pbeam` as a dependency, such as `examples/plug_hello` with `{:pbeam, path: "../..", runtime: false}`, runs them. The tasks find the PBeam source from the path of the dependency, and Cargo builds `pbeam` and `pbeam_bundle` in that source. `mix pbeam.bundle` writes the start list, the BEAM file of each module of `erlang:pre_loaded()` in the `ebin` directory of `erts`, and runs the `bundle` command. `mix pbeam.build` runs `mix release` in the `MIX_ENV=prod` environment first, unless `--release` names a built release directory. It stops with an error when the ERTS version of the release, the first field of `releases/start_erl.data`, is not the ERTS version of the Erlang that runs the command: the pinned OTP of the PBeam source, whose modules PBeam runs. It also runs the `executable` command, and writes `<app>.bundle` next to the release directory, or the path of `--output`. The mise task `mise run example-executable <name>` writes the one executable of a release example or a fixture example. The build targets the host: macOS arm64 on the development host and Linux x86_64 in CI. The build needs Cargo and the pinned OTP; the run needs neither. On macOS the linker signs the `pbeam` binary ad hoc, and `codesign --verify --strict` passes for it. The appended bundle lies outside the signed bytes: the appended executable keeps the ad-hoc code directory and runs, and `codesign --verify --strict` stops with `main executable failed strict validation`.

The NIF manifest gate is a build check, not a runtime check. It loads each BEAM file and compares its NIF references with the NIF libraries of PBeam: the static libraries and the registered extension libraries. PBeam selects the library by the module of the caller, as ERTS does for a static NIF library (`beam/erl_nif.c:4743-4757`), so a call of `erlang:load_nif/2` must be in a module that has a library. In a module with a library, the path of the call can be any term; thus `crypto`, which builds its path at run time (`crypto.erl:4479`, `4499`), passes. In a module without a library and without a shipped library, the path must be a literal string, so that the gate can name the library; the compiler folds `atom_to_list(?MODULE)` to such a literal. The module `pubkey_os_cacerts` has no PBeam library, and its `on_load` calls `erlang:load_nif/2` only when `os:type()` is `{win32, _}` (`pubkey_os_cacerts.erl:265-275`). PBeam builds a bundle only for the host, and does not support Windows, so the gate reports no item for that call on a host that is not Windows. For a module without a PBeam library, the runtime opens `Path.so` with the NIF ABI loader. The shipped libraries of a BEAM file `<dir>/ebin/<module>.beam` are the `.so` files under `<dir>/priv/`, the directory that `code:priv_dir/1` gives for the application. The gate opens them with the same loader, in the order of the names, until one has the name of the module in its `ErlNifEntry`, because `erlang:load_nif/2` requires that name (`beam/erl_nif.c:4820-4823`). The open runs the constructors and the init function of the library on the build host, which built the library. The module passes when the library has the vm variant `beam.vanilla`, the functions of the library pass the checks of `erlang:load_nif/2` against the module (`beam/erl_nif.c:4814-4819`, `4845-4900`), and the module has `nif_start`, which PBeam needs. The path of the call can be any term, because the gate does not resolve it: `Elixir.Bcrypt.Base` of `bcrypt_elixir` builds it with `code:priv_dir/1`. A shipped library with the name of the module that fails a check is a build error, as `nif_add: NIF library "<file>" does not load: Function not found nif_add:sub/2`. A module without a PBeam library and without a shipped library that calls `erlang:load_nif/2` is a build error: the call fails after deployment. Then each shipped library of the module that the loader cannot open is a build error too, with the `erlang:load_nif/2` text, because it can be the library of the module. Each function that starts with `nif_start`, is in the library of its module, and has no binding is a report item, not a build error: a call of it gives a clear result at run time. The item names the library, the function, the arity, and the result of a call: the runtime error `M:F/A NIF is not supported` for a library whose unbound functions fail, such as `zstd`, or the BEAM stub body for another library, such as `prim_tty`. Each item is one line, such as `prim_tty:isprint/1: NIF function has no PBeam binding; a call runs the BEAM stub body`.

The `pbeam_bundle check-nifs <directory>` command runs the gate on each BEAM file under the directory. The `pbeam_bundle bundle` command runs the gate on each BEAM file of the bundle before it writes the output. Both commands print each report item on stdout, as `pbeam_bundle: the NIF manifest gate reports prim_tty:isprint/1: NIF function has no PBeam binding; a call runs the BEAM stub body`. A library without bindings, a path that is not a literal string, and a shipped library that does not load stop the command with status 1, and the `bundle` command then writes no output. The `fixture` command does not run the gate.

A trailer with a bundle length that does not fit before the trailer is an error, and the run stops with status 1. Without the magic at the end of the file, the binary has no bundle, and its command line runs as usual.

The resource `meta/start-modules` names the modules of the start set, separated by ASCII whitespace. For a release, the start set is the preloaded modules of `erlang:pre_loaded/0`, which `scripts/example.sh --write-start` lists, and the release boot reads them from `lib/erts-<Erts>/ebin`. For a fixture bundle, the run loads `pbeam/<entry_module>.beam` and `pbeam/pbeam_boot.beam` first, then each module of the resource. A module resolves in `pbeam/`, in `releases/*/consolidated/`, in `erts-*/ebin`, or in `lib/*/ebin` of the extracted release, in this order. Thus a consolidated protocol comes before the unconsolidated protocol of the same name in a lib directory.
