# On-demand code loading

This document records the design of on-demand code loading and the owner decisions for it. The epic [[m5-e3] On-demand code loading](https://github.com/fahchen/pbeam/issues/440) owns the work, and [task #441](https://github.com/fahchen/pbeam/issues/441) records this design. The owner accepted the design on 2026-10-11 in Suikou review 01a1282e (design `code-loading.md`, round 2 conclusion, and `open-questions.md`, round 3). This document describes planned work. Each section that describes the current branch says so.

## 1. Goal and scope

Goal: PBeam runs the pinned OTP code loading path as ERTS does in interactive mode. The pinned `erl_prim_loader` process reads the code files. The pinned `code_server` process holds the code path. The pinned `error_handler.beam` loads a module at its first call. `erlang:prepare_loading/2` and `erlang:finish_loading/1` (and `erlang:load_module/2`) add the module to the code table of the run. The CLI and the library API take a code path and a small start set, not the full list of BEAM files.

The work removes these observable differences of the current branch:

- `code:ensure_loaded/1` of a module that is not loaded stops the run with the runtime error `unresolved external MFA code_server:get_mode/0`. ERTS returns `{module, Module}` or `{error, nofile}`. #445 removes this difference for a run with `--root`.
- A call to a module that is not loaded stops the run with the runtime error `unresolved external MFA M:F/A`. ERTS loads the module through `error_handler`, or raises `undef`.
- The output order of processes is not the order of interactive mode. In Terra Counter ([#428](https://github.com/fahchen/pbeam/issues/428)), the ERTS `main` process waits for 11 module loads, but the PBeam `main` process writes first.
- Each example needs a build-time closure script. A call through a module name in a variable, for example a supervisor child, must be listed by hand.

Out of scope: hot code upgrade (old code, `purge`, `check_old_code`), `-on_load`, archives, the `inet` loader, and a code index for more than one scheduler.

## 2. Current state

This table describes the current branch.

| Location | Fact |
| --- | --- |
| `src/runtime/interpreter.rs:44-50` | `BeamProgram` holds `modules: HashMap<String, BeamModule>` and `atoms`. A run has only `&self`, so the code table cannot change during a run. |
| `src/runtime/interpreter.rs:145-159` | `BeamProgram::load` decodes all BEAM bytes at one time. A duplicate module name is an error. |
| `src/runtime/interpreter.rs:336-337` | Each run makes its `Scheduler` with `self.atoms.clone()`. Thus each run already has its own atom table, which can hold the atoms of a module that the run loads. |
| `src/runtime/process.rs:46-55` | A `Frame` refers to code by module **name** and instruction index. It does not borrow a `BeamModule`. Thus a module that a run adds does not make a frame invalid. |
| `src/runtime/interpreter.rs:509-525`, `580-584` | `export_frame` and `module` find a module in `self.modules`. A module that is not there gives `unresolved external MFA M:F/A` or `missing BEAM module M`. |
| `src/runtime/interpreter.rs` (`undefined_function`) | Since #447, `undefined_function` runs the pinned `error_handler:undefined_function/3` of the process, which `process_flag(error_handler, Module)` sets. It puts the module, the function, and the argument list in `x0`, `x1`, and `x2` and enters the export (`call_error_handler`, `beam_common.c:1294-1342`). A run without a code server and without the init process of `pbeam_boot` cannot answer the load call of the default handler, so a call of a module that the run does not load with the default handler keeps the runtime error `unresolved external MFA`. A custom handler answers the call itself. |
| `src/runtime/interpreter/bif/erlang.rs:2609-2615` | `erlang:module_loaded/1` reads only `program.modules`. |
| `src/runtime/interpreter/bif/table.rs` | #442 added `erlang:finish_loading/1` and `erts_internal:prepare_loading/2`. #445 added `erlang:get_module_info/2` (key `module`) and `erlang:pre_loaded/0`, and the BIFs that `code_server` calls: `erlang:'--'/2`, `erlang:unique_integer/0`, `erts_internal:list_to_integer/2`, and `maps:from_keys/2`. `erlang:get_module_info/1` stays `unsupported`. The compiler-generated `M:module_info/1` calls `erlang:get_module_info/2`. |
| `src/runtime/interpreter/native/prim_file.rs:29-63`, `72-98` | `prim_file` registers 33 NIF names. `get_cwd_nif/0`, `list_dir_nif/1`, `read_file_nif/1`, `read_info_nif/2`, and `read_link_nif/1` have a binding. The other names have no binding. |
| `src/bin/pbeam.rs:12-52` | The CLI is `pbeam MODULE FUNCTION [ATOM]... [--policy MODULE] [--no-capability-checks] [--boot] [--pa DIR]... -- BEAM_FILE...`. Each argument must be an atom of the loaded modules. #444 added `--pa DIR`: with it, the root function gets the boot flag list as one more argument. #445 added `--root DIR` and `--mode interactive|embedded` (section 7). |
| `examples/hello_world/pbeam_boot.erl:4-42`, `86-106`, `194-210` | The boot registers an `init` request process with the flags `[{noshell, []} | Flags]`. Since #444, it starts the pinned `erl_prim_loader` with the path `Pa ++ Pz ++ ["."]`. It starts `logger` and `application_controller` (with the kernel application spec of the pinned `start_clean.boot`) in the boot process, and a kernel process starts the `kernel_sup` children by hand. It does not start the `kernel` application. Since #445, with a root flag, the boot sets the path of the last `path` command of `start_clean.boot`, and the kernel process starts `code_server` as the first `kernel_sup` child. |
| `scripts/example_beams.exs:15-28` | `@not_loaded` lists 10 modules that the PBeam loader cannot decode. Since #445, the build gives `code_server` to the run, and `scripts/example.sh` gives `--root`. |
| `docs/erts-differences.md`, rows "Undefined functions", "Code loading", and "Boot" | These rows record the differences above. |
| `docs/backlog.md` | The entry "Give caller frames and line locations in stack traces" records the remaining work on the frames of `erlang:error/1`. #445 removed the entry "Run `code:ensure_loaded/1` as ERTS does", #446 removed "Keep the frames that `erlang:raise/3` receives", #447 removed "Run `error_handler.beam` for an undefined function", and #447 updated "Raise `undef` for an entry module that the run does not load" with its remaining work. |
| `src/runtime/capability.rs:25-46` | The capability kinds have no `code` kind and no `file` kind. `docs/capability-security-migration.md` marks `file` (`read`, `write`, `list`) as later. [#437](https://github.com/fahchen/pbeam/issues/437) keeps `erlang:module_loaded/1` at the inventory value `later` until the code loading capability exists. |

## 3. ERTS flow

These references are at the `OTP-29.1.1` tag, the pin of `mise.toml`.

1. **Start of the loader.** `init` starts `erl_prim_loader` and sets its path to `Pa ++ Pz ++ ["."]` (`erts/preloaded/src/init.erl:1164-1176`). `erl_prim_loader` reads `init:get_argument(loader)`, with the default `"efile"`, and checks `prim_file:get_cwd()` (`erts/preloaded/src/erl_prim_loader.erl:153-214`). It reads files with `prim_file:read_file/1`, `prim_file:list_dir/1`, `prim_file:read_file_info/1`, and `prim_file:read_link/1` (`erl_prim_loader.erl:721`, `1112`, `1140`, `1195`, `1712`).
2. **Mandatory modules.** The boot script loads `error_handler`, `code`, `code_server`, and the other mandatory modules with `primLoad` (`lib/sasl/src/systools_make.erl:1541-1577`). `error_handler` loads each other module on demand.
3. **Code server start.** The first child of `kernel_sup` is `{code, start_link, []}` (`lib/kernel/src/kernel.erl:110-115`, `222`). `code:do_start/0` loads the code server prerequisites with `M:module_info(module)`, reads the `root` flag with `{ok,[[Root0]]} = init:get_argument(root)`, and starts `code_server:start_link([Root, Mode])` (`lib/kernel/src/code.erl:1682-1712`). Thus a boot that starts `code_server` needs the `root` flag. `code_server:start_link/1` spawns and links the server process and waits for its reply (`lib/kernel/src/code_server.erl:66-73`). `code_server:init/3` registers the name, traps exits, makes the `ets` module table, inserts `erlang:pre_loaded()` and `init:fetch_loaded()`, and, in interactive mode, lists `Root/lib` to make the path. It then adds `erl_prim_loader:get_path()` and the `pa` and `pz` flags, and calls `persistent_term:put(code_server, Mode)` (`code_server.erl:91-125`, `513-525`). The server loop receives `{code_call, Pid, Req}` messages (`code_server.erl:162-180`).
4. **`code:ensure_loaded/1`.** When `erlang:module_loaded/1` returns `true`, the call returns `{module, Mod}`. Else it reads the mode. In interactive mode, it calls the server with `{get_object_code_for_loading, Mod}`, runs `erlang:prepare_loading/2` in the client, and then calls the server with `{load_ok, ...}`, so that the server runs `erlang:finish_loading/1`. In embedded mode, it returns `{error, embedded}` (`code.erl:581-612`). The server finds the file in the code path (`code_server.erl:1112-1144`). It puts concurrent requests for the same module in a queue (`code_server.erl:1283-1384`).
5. **Undefined function.** The export entry calls `error_handler:undefined_function/3` (`erts/emulator/beam/beam_common.c:1294-1342`). This function calls `ensure_loaded/1`. When the caller is `code_server`, it halts the node. When `code_server` is registered, it calls `code:ensure_loaded/1`. Else it calls `init:ensure_loaded/1` (`lib/kernel/src/error_handler.erl:85-98`, `182-198`). For a failure, `crash/1` uses `tl(Stacktrace)` and `erlang:raise/3` (`error_handler.erl:172-178`). Before the code server starts, `init` loads a module with `erl_prim_loader:get_file/1` (`init.erl:722-750`).
6. **BIFs.** `erlang:prepare_loading/2` calls `erts_internal:prepare_loading/2`. It returns a magic reference or `{error, badfile}` (`erts/preloaded/src/erlang.erl:5436-5463`; `erts/emulator/beam/beam_bif_load.c:232-266`). Before that, a binary that does not start with `FOR1` goes through `zlib:gunzip/1` (`erlang.erl:5438-5447`). A file that the loader rejects sends an error report to the system logger process, with the group leader of the caller as the `gl` metadata of the report (`erts/emulator/beam/beam_load.c:146`, `414`; `erts/emulator/beam/utils.c:849-857`; `erts/emulator/beam/erl_trace.c:2494-2496`). The report does not go to the group leader. `erlang:finish_loading/1` raises `badarg` for an argument that is not a proper list of prepared code references (`beam_bif_load.c:341-394`). For a list that names a module two times, it returns `{duplicated, [Mod]}` (`beam_bif_load.c:429-441`). For a module that has current code and old code, it returns `{not_purged, [Mod]}` (`beam_bif_load.c:443-455`). These two results install no code and keep the prepared code, so a later call can use the same references. For a module that has only current code, it makes the current code old and loads the new code. `erlang:load_module/2` is `prepare_loading/2` and then `finish_loading/1` (`erlang.erl:7513-7528`). `erlang:module_loaded/1` returns `true` for a module with current code and no pending `-on_load` function (`beam_bif_load.c:738-755`).

Measured on the pinned ERTS:

- `mise run erts-eval '{code:get_mode(), code:ensure_loaded(nomod_x), erlang:load_module(queue_x, <<"bad">>), (catch erlang:finish_loading([a])), length(erlang:pre_loaded()), erlang:module_loaded(nomod_x)}'` gives `interactive`, `{error,nofile}`, `{error,badfile}`, `{'EXIT',{badarg,[{erlang,finish_loading,[[a]],[{error_info,#{module => erl_erts_errors}}]}|_]}}`, `22`, and `false`. The `load_module/2` call also writes the report `beam/beam_load.c(146): Error loading module queue_x:` and `corrupt file header`.
- `mise run erts-eval '{queue, B, _} = code:get_object_code(queue), R1 = erlang:load_module(queue, B), R2 = erlang:load_module(queue, B), {R1, R2}'` gives `{{module,queue},{error,not_purged}}`. The first call replaces the loaded code. The second call finds old code.
- `mise run erts-eval '{queue, B, _} = code:get_object_code(queue), R = erlang:prepare_loading(queue, B), A1 = erlang:finish_loading([R,R]), A2 = erlang:finish_loading([R]), A3 = (catch erlang:finish_loading([R])), {A1, A2, element(1, element(2, A3))}'` gives `{{duplicated,[queue]},ok,badarg}`. The failed first call keeps the prepared code. The second call installs it. The third call finds no prepared code.
- `mise exec -- erl +S 1 -noshell -eval 'io:format("~p~n", [length(code:all_loaded())]), halt().'` writes `112`.
- `mise exec -- erl +S 1 -noshell -mode embedded -eval 'io:format("~p~n", [{code:get_mode(), code:ensure_loaded(nomod_x)}]), halt().'` writes `{embedded,{error,embedded}}`.

## 4. Options

| Option | Content | Advantages | Disadvantages | Cost | Risk |
| --- | --- | --- | --- | --- | --- |
| A. Run the pinned OTP path | PBeam adds only BIFs and NIFs: `prepare_loading`, `finish_loading`, `module_loaded`, `get_module_info`, `pre_loaded`, and the `prim_file` file reads. `erl_prim_loader`, `code_server`, and `error_handler` run as pinned `.beam` code. The code path is BEAM state that the boot flags give. | It obeys the rule "run the pinned OTP module code". The messages, the queue, `{error, nofile}`, and the process order are the same as on ERTS, so #428 is fixed. `code:get_path/0`, `code:which/1`, and the other `code` functions also work. Embedded mode comes with it. | The start set must include the code server prerequisites. `error_handler.beam` needs stack-trace work first. | Medium: about 6 PRs. | `code_server` can call BIFs that PBeam does not have yet. Each one needs a task. Decoding BEAM files during a run takes time. |
| B. A code server in Rust | Rust holds a `Vec<PathBuf>`. When the export entry does not find a module, Rust reads and decodes the file. A Rust override replaces `code:ensure_loaded/1`. | Fast to implement. No BEAM start dependency. | It replaces pinned OTP code with a Rust shortcut, which the project rules do not permit. There is no `code_server` process and no message, so the order is not the ERTS order and #428 is not fixed. The other `code` functions still do not work. | Low to medium. | Option A replaces it later, so the work is done two times. |
| C. Keep the start load and run embedded mode only | The build-time closure stays. `code_server` starts in embedded mode, `code:ensure_loaded/1` returns `{error, embedded}`, and a module that is not loaded gives `undef`. | It agrees with the old spec text "startup-loaded set". It needs no `prim_file` file read. | It is not interactive mode, which is the ERTS default and the mode of the reference run. The #428 order does not change. The closure script stays. | Medium: it still needs the code server start and the stack-trace work. | The reference run must change to `-mode embedded` for a comparison, which is not how Terra runs. |

## 5. Selected option: A with part of C

The owner selected option A with part of option C (round 2 conclusion, 2026-10-11).

- `erl_prim_loader`, `code_server`, and `error_handler` run as pinned BEAM code. On ERTS, `code_server` is an ordinary BEAM process: `code:start_link/0` calls `code_server:start_link/1`, which spawns and links a process with a `receive` loop (`code.erl:1669-1689`; `code_server.erl:66-73`, `176-180`). `kernel_sup` starts it as its first child. `erl_prim_loader` is also a preloaded BEAM process (`erl_prim_loader.erl:153-161`). `erl_prim_loader` and `code_server` are processes of the kernel tree. The kernel tree has no capability check, so these processes need no Rust code server.
- A start set loads before the run: the preloaded modules (`erlang`, `init`, `erl_prim_loader`, and others) and the modules that the kernel start needs. Other modules load on demand. With `--mode embedded`, all modules load at the start (option C).
- Option B is rejected. It replaces pinned OTP code with a Rust shortcut, and it does not give the ERTS message order.

Reasons:

- The project rules require PBeam to run the pinned OTP module code and to match the observable result. Only option A does this.
- A `Frame` refers to code by name (`src/runtime/process.rs:46-55`), and each run has its own atom table (`src/runtime/interpreter.rs:336-337`). Thus the Rust change that adds a module during a run is small.
- Option A includes option C: `--mode embedded` is only a boot flag. A bundle can use embedded mode (no file read after the start) or interactive mode (the bundle is a code path).

## 6. Code table and prepared code

- **One code table for each run.** The module table moves to a `CodeTable` that each run owns. Its elements are `Rc<BeamModule>`, and the start set of `BeamProgram` makes it. Each module lookup uses this table. Alternative: an overlay, which keeps the start set in `&self` and puts new modules in the scheduler. It keeps the current borrow path, but each lookup reads two tables, and a later `delete_module` is harder. The owner rejected the overlay. The PR of #442 measures the instruction fetch cost before and after the change. A run clones each `Rc` one time, not at each instruction.
- **Prepared code.** A prepared module is a `Term::Reference` with an entry in a prepared table of the run. `erlang:is_reference/1` returns `true`, as for the ERTS magic reference. `erlang:finish_loading/1` removes the entry only when it installs the code. A call that returns `{duplicated, [Mod]}` or `{not_purged, [Mod]}`, or that raises `badarg` or `system_limit`, keeps the entry (section 3). A later call with a reference whose code is installed raises `badarg`. An entry that does not finish stays until the run ends. ERTS frees it at garbage collection, so this is only a memory difference, and `docs/erts-differences.md` records it.
- **No cache.** PBeam does not keep a cache of decoded modules. The examples task (#448) measures the start time of Terra Counter. If the start is slower, `docs/backlog.md` gets an entry for a cache.

## 7. CLI arguments and boot flags

- The CLI uses the `erl` names: `--pa DIR` (repeatable), `--root DIR`, and `--mode interactive|embedded`. The default mode is `interactive`, as for `erl`.
- `-- BEAM_FILE...` stays. It gives the start set (the preloaded modules and the modules of the `primLoad` step), not the full list of modules.
- `--root` has no default. Without `--root`, the `erl_prim_loader` path is only the `--pa` directories and `"."`, as `init.erl:1166` does. The example script gives the pinned OTP root that `code:root_dir/0` of the pinned `erl` gives (#445). The binary does not hold a host OTP path. The pinned `code:do_start/0` reads the `root` flag (`code.erl:1686`), so a run that starts `code_server` must give `--root`.
- The boot gets its flags as a `[{Flag, [Value]}]` list, as `init:parse_boot_args/1` gives them (`init.erl:1668-1709`). The library API gets an entry that takes `Term` arguments. Alternative: `pbeam_boot` reads the OS environment, which needs the `env` capability. The owner selected the flag list.
- #444 implements the flag list. Each `--pa DIR` gives `{pa, [Dir]}`, with `Dir` as a binary of its bytes, as the emulator gives each boot argument to `init` (`erl_init.c:373-384`). With one or more flags, the `pbeam` binary gives the list to the root function as one more argument after the atoms, so `pbeam_boot:start/3` gets it. Without a flag, the binary gives no list, and `pbeam_boot:start/2` calls `start/3` with `[]`. The library entry is `BeamProgram::execute` and the other `execute` functions, which already take `&[Term]`: a host passes the flag list as a list term, for example from `Term::list_from`.
- #445 adds `--root DIR` and `--mode MODE`. The flag list starts with `{root, [Root]}`, then has the `pa` flags, and ends with `{mode, [Mode]}`, as `erlexec` gives `-root` before the user flags. A flag that the command line does not have gives no tuple, and the binary gives the list when it has `--root` or `--pa`. `--mode` accepts only `interactive` and `embedded`, and needs `--root`, because only `code_server` reads it.
- #445: with a root flag, `pbeam_boot` sets the path to `Pa ++ ["$ROOT/lib/kernel-11.0.4/ebin", "$ROOT/lib/stdlib-8.1/ebin"] ++ Pz`, as the last `path` command of the pinned `start_clean.boot` does (`init.erl:1323-1332`, `1417-1442`). Then the kernel process runs `code:start_link/0` before its other children, as the first `kernel_sup` child (`kernel.erl:110-115`). Without a root flag, the boot starts no code server: ERTS cannot boot without `-root` (`init.erl:1226-1232`), and each run that does not need `code_server` keeps its start. The `init` request process answers `fetch_loaded` with `[]`, because the boot loads no module with `primLoad` (`init.erl:720-722`).
- #445: `erlang:pre_loaded/0` gives the fixed ERTS list of 22 modules, also when the run did not load one of them. `code_server` marks each of them `preloaded` (`code_server.erl:96-99`). A start module that is not in this list is not in the module database of `code_server`, so `code:which/1` searches the path for it, and `code:is_loaded/1` and `code:all_loaded/0` do not give it. `docs/erts-differences.md` records this in the row "Code loading".

## 8. Unsupported cases

These cases stop the run with a clear runtime error. `docs/erts-differences.md` records each one when its task lands:

- `erlang:finish_loading/1` of a module that is loaded (code replacement). ERTS makes the current code old and loads the new code, and the next replacement returns `{not_purged, [Mod]}` (section 3).
- A module with an `-on_load` function. ERTS returns `{on_load, [Mod]}` from `erlang:finish_loading/1`, and `code_server` runs the function (`code_server.erl:1348-1352`).
- A module with an instruction that the PBeam loader cannot decode. This is not `{error, badfile}`, because ERTS loads the module.

## 9. Authorization

- `erlang:prepare_loading/2` has no capability check. It only decodes the module and makes atoms, as `erlang:list_to_atom/1` does (`runtime.atom`).
- The authorization points are `erlang:finish_loading/1` (`code:load`) and the read of a code file (`file:read`).
- [#465](https://github.com/fahchen/pbeam/issues/465) of [m6] Capability security added the `code` kind (section 9 of `docs/capability-security-migration.md`). The kind has the operations `load` and `inspect` and the constraint `modules`. `erlang:finish_loading/1` checks `load` for each module of the list; `erlang:module_loaded/1` and `erlang:get_module_info/2` check `inspect` for the module name. The read of a code file waits for the `file` kind.
- The processes of the kernel tree, which include `erl_prim_loader` and `code_server`, have no check.

## 10. Specification change

The bundle defines the loadable module set. Interactive mode loads a module on demand. Embedded mode loads all modules at the start. Loaded code does not change: there is no code replacement. This keeps the security intent of a fixed code set. `docs/portable-beam-spec.md` states this text in the "Code loading" row of section 2, in the exclusions of section 3, in section 5, and in the startup text of section 9.

The default mode of a bundle stays embedded mode, with interactive mode as an option. Embedded mode reads no file after the start. [#15](https://github.com/fahchen/pbeam/issues/15) of [m8] Embedded application confirms this default.

## 11. Task order of this epic

Each task has one PR. The GitHub issues are the source of truth for scope, acceptance, and blockers.

1. [#441](https://github.com/fahchen/pbeam/issues/441) Record the on-demand code loading design (this document).
2. [#442](https://github.com/fahchen/pbeam/issues/442) Load a module at run time with `erlang:load_module/2`. The `CodeTable`, `erts_internal:prepare_loading/2`, and `erlang:finish_loading/1` for a new module. `erlang:module_loaded/1` and `erlang:function_exported/3` read the code table. Blocked by #441.
3. [#443](https://github.com/fahchen/pbeam/issues/443) Read code files through `prim_file` for `erl_prim_loader`. `read_file_nif/1`, `list_dir_nif/1`, `get_cwd_nif/0`, and `read_link_nif/1`. It can run in parallel with #442. Blocked by #441.
4. [#444](https://github.com/fahchen/pbeam/issues/444) Start `erl_prim_loader` with a code path from the command line. `--pa DIR` goes to `pbeam_boot` as a boot flag, and `pbeam_boot` starts the pinned `erl_prim_loader` as `init.erl:1164-1176` does. Blocked by #443.
5. [#445](https://github.com/fahchen/pbeam/issues/445) Start `code_server` and load a module in `code:ensure_loaded/1`. `--root DIR` and `--mode`, `erlang:pre_loaded/0`, and `erlang:get_module_info/1,2`. Blocked by #442 and #444.
6. [#446](https://github.com/fahchen/pbeam/issues/446) Keep the stack trace frames that `error_handler.beam` reads. A sub-issue of #447: `erlang:error/1` gives the caller frames, and `erlang:raise/3` keeps its frames. Without them, `tl([])` in `crash/1` raises `badarg`.
7. [#447](https://github.com/fahchen/pbeam/issues/447) Run `error_handler.beam` for an undefined function. The export entry calls the pinned `error_handler:undefined_function/3`, `process_flag(error_handler, Module)` works, and `pbeam_boot` answers `{ensure_loaded, Module}`. Blocked by #445 and #446. In review: [PR #490](https://github.com/fahchen/pbeam/pull/490).
8. [#448](https://github.com/fahchen/pbeam/issues/448) Run the examples from a code path. `scripts/example.sh` gives only the start set, `--pa`, and `--root`. Terra Counter ends in the interactive-mode order of #428. Blocked by #447.
9. [#449](https://github.com/fahchen/pbeam/issues/449) Start the `kernel` application with the pinned `application_controller` (section 12). Blocked by #443, #444, and #445.

Later work outside this epic: [#465](https://github.com/fahchen/pbeam/issues/465) adds the `code` capability kind (section 9). The independent application root of [m6] ([#464](https://github.com/fahchen/pbeam/issues/464)) waits for #449.

## 12. Kernel start

The owner asked on 2026-10-11 to start the `kernel` application as OTP does. The last task of this epic (#449) does this work.

- Current branch: `pbeam_boot` starts `logger_server` and `application_controller` in the boot process, with the kernel application spec of the pinned `start_clean.boot` written in `pbeam_boot.erl`. It does not call `application:start(kernel)`, so there is no `application_master` and no `kernel_sup`. A kernel process starts `standard_error`, `file_server_2`, `erl_signal_server`, `user_sup`, and `logger_sup` by hand, in the `kernel_sup` order (`examples/hello_world/pbeam_boot.erl:27-42`, `194-210`). Early PBeam could not read an `.app` file and had no code server, so the boot added the processes one at a time.
- ERTS: `init` runs the boot script, starts `application_controller`, and runs `application:start_boot/2` for `kernel`. `kernel:start/2` starts `kernel_sup` (`kernel.erl:35-45`), and `kernel_sup` starts the children that `kernel.app` and the configuration give (`kernel.erl:91-115`, `188-226`). Its first child is `code_server`. Without `-mode minimal`, its children also include `inet_db` (`kernel.erl:211-226`), which the PBeam loader cannot decode yet (`scripts/example_beams.exs:21`).
- Design: after on-demand loading lands, `pbeam_boot` runs the pinned `application_controller` and the `kernel` application, so the start is the same as in OTP. A new kernel child then needs no change of `pbeam_boot`. One possible method: give the content of `kernel.app` to `application:load/1` at build time, as the OTP boot script does, so the early start reads no file. The children of `kernel_sup` include `code_server`, so the code loading tasks must merge first.
- All processes that the kernel start makes belong to the boot authority (kernel tree).

## 13. Bundle

The bundle work of [m8] gets no task now. `docs/backlog.md` has the entry "Read code files from the bundle as a read-only code path root": a bundle is a read-only code path root, and `prim_file` reads a path under this root from the bundle. The trigger is the bundle format of #15.
