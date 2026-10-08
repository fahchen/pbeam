# Effect inventory

Status: reviewed inventory for transition 1 of the [capability security migration](capability-security-migration.md). This document adds no security mechanism. It does not change runtime behavior. The [security design](capability-security-design.md) defines the terms that this inventory uses.

This inventory lists each effect entry that PBeam executes on the current branch: each BIF, native MFA, instruction path, VM operation, host operation, startup operation, and linked native library. For each entry, it records the authority requirement, the determinism, the execution class, the observable effects, the cost, and the owner. The owner is the Rust module, as a file path, that executes the entry.

A classification in this document is reviewed metadata. It is not a proof about the Rust or C code. The authority labels name the protected domain of each entry. They are not a grant syntax. Transition 2 defines grants.

## Maintenance

Update this inventory in the PR that adds, removes, or changes an entry. The unit test `effect_inventory_lists_each_bif_and_native_function` in `src/runtime/interpreter/native.rs` reads this file. The test fails in these conditions:

- A BIF that has a body in `BIFS` (`src/runtime/interpreter/bif/table.rs`) has no row in the section "BIFs".
- An `unsupported` entry of `BIFS` is not in the section "Unsupported BIFs".
- A native binding of the static registry has no row in the section "Native bindings".
- A function of a static NIF library that has no binding is not in the section "Native functions without a binding".
- One of these sections lists an entry that the code does not have.

The test does not check the other sections. Review them when a PR changes an instruction, a VM operation, a host operation, startup, or a linked library. Tasks #7, #221, and #9 change VM operations. Their PRs update this inventory.

An entry key is `module:name/arity`. The function name is written as it is, also an operator name, for example `erlang:+/2` and `erlang:=/=/2`.

## Classification terms

### Authority

The authority column gives the protected domains that a request to the entry uses. One entry can use more than one domain.

| Label | Protected domain |
| --- | --- |
| `none` | No protected state and no host resource. The result depends only on the arguments, or the entry only raises. |
| `process.self` | The state of the calling process only: its dictionary, flags, mailbox, and receive state. |
| `process.spawn` | Creation of a process. |
| `process.send` | A message to another process. |
| `process.exit` | An exit signal to a process. |
| `process.link` | A link between two processes. |
| `process.monitor` | A monitor of a process or a port name. |
| `process.alias` | An alias reference and the messages and signals that go through it. |
| `process.group_leader` | The group leader of a process. |
| `process.name` | The registered names of the node. |
| `process.inspect` | Information about another process. |
| `runtime.info` | Node and system information, and node-wide flags. |
| `runtime.code` | The loaded code and the NIF library state of the run. |
| `runtime.ets` | ETS tables. |
| `runtime.persistent_term` | Persistent terms. |
| `runtime.atom` | New atoms in the atom table of the run. Atoms are never released. |
| `runtime.timer` | Timers of the run. |
| `runtime.resource` | Node resources in the typed store: compiled regular expressions and TTY records. |
| `runtime.service` | Runtime work that no guest request starts: scheduling, timer expiry, and exit cleanup. |
| `launcher` | The trusted launcher before the root process starts. |
| `host.clock.monotonic` | The host monotonic clock. |
| `host.clock.system` | The host system clock. |
| `host.timezone` | The host time zone data. |
| `host.locale` | The C library locale of the OS process. |
| `host.environment` | The OS environment of the PBeam process. |
| `host.os_info` | OS identity information from `uname`. |
| `host.signal` | The signal dispositions of the OS process. |
| `host.filesystem.read` | Reads of host files. |
| `host.stderr` | Writes to fd 2 by the runtime, not by guest code. |
| `terminal.query` | Reads of terminal state: `isatty`, `tcgetattr`, and the window size. |
| `terminal.setup` | The terminfo setup of the curses library. |
| `terminal.text_write` | Bytes to the terminal output fd. |
| `terminal.tui_control` | Terminal control sequences in the output bytes. PBeam does not filter them. |
| `terminal.raw` | A change of the terminal mode, for example raw mode. |
| `terminal.input` | Terminal input selection. |
| `native.load` | A request to load a NIF library. |

### Determinism

| Value | Meaning |
| --- | --- |
| `pure` | The result depends only on the arguments. |
| `run` | The result depends on the arguments and the state of the run. The same program, input, and schedule give the same result. |
| `host` | The result or the effect depends on host state outside the run, for example a clock, the environment, or the terminal. |
| `unknown` | This inventory cannot classify the determinism. |

### Execution class

| Value | Meaning |
| --- | --- |
| `inline` | The entry runs to completion on the scheduler thread and makes no host call. |
| `host-call` | The entry runs on the scheduler thread and calls the host or a linked C library. The call does not wait for an external event. |
| `host-blocking` | The entry runs on the scheduler thread, and its host call can wait for an external event, for example a full pipe or a file read. All processes stop until the call returns. |
| `trap` | The entry can continue in BEAM code as a call to an exported function, or in the export of a BIF. The process then switches out, and it continues in the export in its next turn. |
| `scheduler` | The scheduler loop runs the entry. No guest call requests it. |
| `startup` | The entry runs once before the root process starts, or after the run ends. |
| `build` | The entry runs at build time. |

### Cost

`O(1)` is constant. `O(n)` is linear in the size of the argument terms. `O(p)` is linear in the number of live processes. `O(t)` is linear in the number of entries in the shared table that the entry uses. `O(q)` is linear in the length of the signal queue of the process. `O(l)` is linear in the number of links and monitors of the process. `O(d)` is linear in the size of the process dictionary. `O(e)` is linear in the size of the OS environment. A copy of a term costs `O(n)`. `unknown` means that this inventory cannot give a bound.

### Common BIF behavior

Each BIF runs on the scheduler thread. A call that matches no pattern of the BIF table gives the runtime error `unresolved external MFA`, and the run stops. A BIF result can be a value, an exception, a trap into BEAM code, or the exit of the caller. A runtime error for a part that PBeam does not implement yet stops the run. [ERTS differences](erts-differences.md) records each such part.

## Startup initialization

These entries run before the root process starts, or after the run ends. No guest request starts them. Classify them apart from guest requests.

| Entry | Authority | Determinism | Execution | Effects | Cost | Owner |
| --- | --- | --- | --- | --- | --- | --- |
| Dynamic library load | `launcher` | `host` | `startup` | The OS loader links `libc` and the curses library (`libncurses` on macOS, `libtinfo.so.6` on Linux). Loader environment variables can select another library file. | `O(1)` | `src/runtime/host/terminal.rs`, `Cargo.toml` |
| Rust runtime start | `launcher`, `host.signal` | `host` | `startup` | The Rust standard library sets `SIGPIPE` to `SIG_IGN` before `main`. On Unix it opens `/dev/null` for each closed fd 0, 1, or 2. The signal disposition applies to the OS process. | `O(1)` | `src/bin/pbeam.rs` |
| Command line parse | `launcher`, `host.stderr` | `host` | `startup` | `clap` reads the process arguments: the module, the function, the argument atoms, and the BEAM file paths. For `--help`, `--version`, or an argument error, it writes text to fd 1 or fd 2 and ends the process. | `O(n)` | `src/bin/pbeam.rs` |
| BEAM file read | `launcher`, `host.filesystem.read` | `host` | `startup` | `std::fs::read` reads each BEAM file that the command line names. | `O(n)` | `src/bin/pbeam.rs` |
| Program load | `launcher` | `pure` | `startup` | `BeamProgram::load` decodes the BEAM files and builds the atom table of the program. It makes no host call. | `O(n)` | `src/runtime/interpreter.rs`, `src/beam/module.rs` |
| Root argument atoms | `launcher` | `pure` | `startup` | The launcher finds each argument atom in the program atom table. A name that is not an atom of the program stops the start. | `O(n)` | `src/bin/pbeam.rs` |
| Scheduler creation | `launcher`, `host.clock.monotonic` | `host` | `startup` | `Scheduler::new` copies the atom table, makes the empty process, ETS, persistent term, and alias tables, puts `RegexCodes` and `Ttys` into the resource store, and sets `system_logger` to `logger`. `Timers::new` reads the monotonic clock and starts the run clock. | `O(t)` | `src/runtime/scheduler.rs`, `src/runtime/timer.rs` |
| Root process start | `launcher` | `pure` | `startup` | `Scheduler::start` makes process 0 with the entry frame and the arguments. | `O(n)` | `src/runtime/scheduler.rs` |
| Run end report | `runtime.service`, `host.stderr` | `run` | `startup` | After the root process exits, `execute` returns the result. On an error, the binary writes `pbeam: <message>` to fd 2 and exits with status 1. | `O(n)` | `src/runtime/interpreter.rs`, `src/bin/pbeam.rs` |

## Scheduler operations

The scheduler loop runs these entries. No guest call requests them. A scheduler clock read is not a guest-visible clock access: the guest does not get the value.

| Entry | Authority | Determinism | Execution | Effects | Cost | Owner |
| --- | --- | --- | --- | --- | --- | --- |
| Scheduler clock read | `runtime.service`, `host.clock.monotonic` | `host` | `scheduler` | Before each scheduling step, `execute` reads the run clock with `Timers::now` to expire timers and to find the sleep time. | `O(1)` | `src/runtime/interpreter.rs`, `src/runtime/timer.rs` |
| Timer expiry | `runtime.service`, `runtime.timer`, `process.send` | `host` | `scheduler` | `Scheduler::expire_timers` removes each due timer. A receive timer makes its process runnable. A BIF timer delivers `{timeout, Ref, Msg}` to its pid, or to the pid that has its registered name at expiry. | `O(t)` | `src/runtime/scheduler.rs`, `src/runtime/timer.rs` |
| Scheduler sleep | `runtime.service`, `host.clock.monotonic` | `host` | `scheduler` | When no process can run, `execute` calls `std::thread::sleep` until the first timer end. | `O(1)` | `src/runtime/interpreter.rs` |
| Process selection | `runtime.service` | `run` | `scheduler` | `Scheduler::schedule_next` takes the next process from the run queue and gives it a budget of `CONTEXT_REDS` (4000) reductions. An exiting process runs its exit cleanup. A process with signals to handle first handles them (row "Signal handling"). A process that then has no budget left or no Erlang work switches out. A process with an expired receive timer continues at its timeout label. | `O(q)` | `src/runtime/scheduler.rs` |
| Process switch | `runtime.service` | `run` | `scheduler` | `Scheduler::yield_current` puts the current process at the end of the run queue when it switches out at an ERTS switch point, for example at the end of its budget, at `erlang:yield/0`, or at a BIF trap. The process continues where it stopped. Each switch out that is not an exit first sends the pending monitor signal. A process with signals to handle stays in the run queue. Each switch out, also to wait, to hibernate, or to exit, adds the reductions of the turn, except the virtual reductions, to the reduction total of the process. | `O(1)` | `src/runtime/scheduler.rs`, `src/runtime/interpreter.rs` |
| Process park | `runtime.service` | `run` | `scheduler` | `Scheduler::park_current` and `Scheduler::hibernate_current` make the current process wait. A hibernating process fetches its messages, and it stays active when its inner queue part has messages. A waiting process with signals to handle stays in the run queue. | `O(q)` | `src/runtime/scheduler.rs`, `src/runtime/process.rs` |
| Exit cleanup | `runtime.service`, `process.send`, `process.exit`, `process.monitor`, `process.link`, `terminal.raw` | `run` | `scheduler`, `host-call` | `Scheduler::exit_process` removes the process, cancels its receive timer and the BIF timers to its pid, deletes the ETS tables that it owns, and removes its registered name. It fetches its signal queue. It sends a link exit signal to each linked process, except through an unlinking link, and a demonitor signal to the target of each monitor that it holds. `Ttys::process_down` sets fd 0 to the first mode of each TTY record that the process owns, with `tcsetattr`. Each process that monitors it gets a monitor down signal. Then it answers the signals that it did not handle (row "Signal handling"), ends its aliases, and sends its pending monitor signal. The exit of process 0 sets the root result. | `O(q)`, `O(l)` | `src/runtime/scheduler.rs`, `src/runtime/scheduler/signals.rs`, `src/runtime/tty.rs`, `src/runtime/host/terminal.rs` |
| Signal handling | `runtime.service`, `process.self`, `process.send`, `process.exit`, `process.link`, `process.monitor`, `process.alias`, `process.group_leader`, `process.inspect` | `run` | `scheduler`, `inline` | `Scheduler::handle_signals` applies the signals of the middle queue part of a process in queue order: at schedule-in, in `loop_rec`, and in the BIFs that handle the signals of the caller up to a flush signal. A signal changes the links, the monitors, or the group leader of the process, puts an `EXIT`, `DOWN`, or alias message in its place, sends a reply message or an unlink acknowledgment, or ends the process. Each signal counts 1 and the extra count of its kind. The handling uses the count divided by 4, plus 1, reductions, and stops at its limit. An abnormal link exit signal to a process that does not trap exits gives a runtime error. `Scheduler::handle_exit_signals` answers the signals of an exiting process: `noproc` for a link or a monitor, an unlink acknowledgment, and the replies `badarg`, `false`, and `undefined`. | `O(q)` | `src/runtime/scheduler/signals.rs`, `src/runtime/mailbox.rs`, `src/runtime/signal.rs` |

## Instruction paths

These entries are BEAM instructions that have an effect or that dispatch to an effect entry. The loader turns each BEAM opcode into one `Instruction` variant (`src/beam/code/instruction.rs`).

| Entry | Authority | Determinism | Execution | Effects | Cost | Owner |
| --- | --- | --- | --- | --- | --- | --- |
| `send` | `process.send`, `process.name`, `process.alias` | `run` | `inline` | The fast path of `erlang:send/2`. It costs one reduction, and the process switches out before the send when the budget is then 0 or less. It sends `x1` to `x0` as `erlang:send/2` does and writes the message to `x0`. A local send then uses 4 reductions, and the process switches out after the send when the budget is then 0. | `O(n)` | `src/runtime/interpreter.rs`, `src/runtime/interpreter/bif/erlang.rs` |
| `loop_rec`, `loop_rec_end` | `process.self` and the authority of "Signal handling" | `run` | `inline` | Reads the current message of the inner queue part of the own process, or moves to the next message. The process switches out before `loop_rec` when the budget is 0 or less. At the end of the inner part, `loop_rec` fetches for 4 reductions when the middle part is empty, else it handles the signals of the process (row "Signal handling") with a limit of 100 reductions. A signal that ends the process makes it switch out, and it exits in its next turn. When the reductions are used up, `loop_rec` runs again in the next turn. `loop_rec_end` uses 1 reduction. | `O(q)` | `src/runtime/interpreter.rs`, `src/runtime/scheduler/signals.rs`, `src/runtime/mailbox.rs` |
| `remove_message` | `process.self`, `runtime.timer` | `run` | `inline` | Removes the current message and cancels the receive timer of the process. | `O(1)` | `src/runtime/interpreter.rs`, `src/runtime/mailbox.rs` |
| `wait` | `process.self` | `run` | `inline` | Parks the process until a message arrives. | `O(1)` | `src/runtime/interpreter.rs` |
| `wait_timeout` | `process.self`, `runtime.timer`, `host.clock.monotonic` | `host` | `host-call` | Reads the run clock and starts a receive timer. `infinity` parks with no timer, and `0` continues. A value that is not valid raises `timeout_value`. The guest observes the clock through the timeout order. | `O(t)` | `src/runtime/interpreter.rs`, `src/runtime/scheduler.rs`, `src/runtime/timer.rs` |
| `timeout` | `process.self` | `run` | `inline` | Clears the receive timer state and moves the save pointer to the first message. | `O(1)` | `src/runtime/interpreter.rs` |
| `recv_marker_reserve`, `recv_marker_bind`, `recv_marker_use`, `recv_marker_clear` | `process.self` | `run` | `inline` | Changes the receive markers of the own queue. `recv_marker_reserve` first fetches. A marker goes to the middle queue part when that part is not empty, and `recv_marker_use` of such a marker moves the save pointer when the signal handling moves the marker into the inner part. | `O(q)` | `src/runtime/interpreter.rs`, `src/runtime/mailbox.rs` |
| `nif_start` | The authority of the binding | The binding | The binding | Runs the native binding of the function. A function without a binding runs its BEAM stub body. The function entry before it costs the reduction of the NIF call. | The binding | `src/runtime/interpreter.rs`, `src/runtime/interpreter/native.rs` |
| `call_ext`, `call_ext_last`, `call_ext_only` | The authority of the target | The target | `inline` | Calls a BIF entry first, then the module code. A light BIF costs one reduction first, and the process switches out before the call when the budget is then 0 or less. A heavy BIF costs no reduction. A tail call of a BIF continues at the `deallocate` and the `return` after it. A missing function calls `$handle_undefined_function/2` or raises `undef`. | `O(1)` | `src/runtime/interpreter.rs` |
| `call_ext` of `erlang:load_nif/2` | `native.load`, `runtime.code`, `runtime.atom`, `host.locale` | `host` | `inline` | Runs as the ERTS `i_load_nif` instruction from module code. It checks the path with the host file name encoding, checks the library functions against the module, and records the library as loaded. The result is `ok` or `{error, {Kind, Text}}`. A module without bindings gives a runtime error. PBeam opens no library file. | `O(n)` | `src/runtime/interpreter/native.rs` |
| `apply`, `apply_last` | The authority of the target | The target | `inline` | Calls `Module:Function` through its export. A BIF costs no reduction, and a tail call of it returns with no reduction. `erlang:apply/2` and `erlang:apply/3` continue with the inner call. `erlang:yield/0` runs its body in `erlang.beam`, which yields. | `O(n)` | `src/runtime/interpreter.rs`, `src/runtime/interpreter/bif.rs` |
| `call_fun`, `call_fun2` | The authority of the target | The target | `inline` | Calls a fun, or raises `badfun` or `badarity`. | `O(n)` | `src/runtime/interpreter.rs` |
| `bif0`, `bif1`, `bif2`, `gc_bif1`, `gc_bif2` | The authority of the BIF | The BIF | The BIF | Runs a BIF entry and writes the result, or jumps to the fail label. A BIF result that traps or ends the caller gives a runtime error. | The BIF | `src/runtime/interpreter.rs` |
| `call_ext` of `erlang:hibernate/0` | `process.self` | `run` | `inline` | Writes `ok` to `x0`, fetches the messages that arrived, and parks the process. The process stays active when its inner queue part has messages, and it stays in the run queue when it has signals to handle. | `O(q)` | `src/runtime/interpreter.rs`, `src/runtime/scheduler.rs` |
| `call_ext` of `erlang:yield/0` | `runtime.service` | `run` | `inline` | Runs as `Instruction::Yield`: writes `true` to `x0` and switches out the process with no reduction cost. The process goes to the end of the run queue. | `O(1)` | `src/runtime/interpreter.rs`, `src/runtime/scheduler.rs` |
| `is_function2` | `none` | `pure` | `inline` | The fast path of `erlang:is_function/2`. | `O(1)` | `src/runtime/interpreter.rs` |
| `bs_start_match3` | `none` | `pure` | `inline` | Jumps for a term that is not a binary. A binary gives a runtime error. | `O(1)` | `src/runtime/interpreter.rs` |
| `fconv` | `none` | `pure` | `inline` | Raises `badarith` for a term that is not a number. An integer gives a runtime error. | `O(1)` | `src/runtime/interpreter.rs` |
| Term and control instructions | `none` | `pure` | `inline` | `label`, `func_info`, `line`, `move`, `allocate`, `allocate_heap`, `deallocate`, `trim`, `test_heap`, `init_yregs`, `call`, `call_last`, `call_only`, `jump`, `try`, `catch`, `catch_end`, `try_end`, `try_case`, `build_stacktrace`, the type tests, `test_arity`, `is_tagged_tuple`, `is_eq_exact`, `is_ne_exact`, `is_eq`, `is_ne`, `is_lt`, `is_ge`, `select_val`, `select_tuple_arity`, `get_map_elements`, `has_map_fields`, `put_map_assoc`, `put_map_exact`, `swap`, `put_list`, `put_tuple2`, `update_record`, `get_list`, `get_hd`, `get_tl`, `get_tuple_element`, and `make_fun3`. They change only the registers, the stack, and the frames of the process, or raise. | `O(n)` | `src/runtime/interpreter.rs` |
| Function entry and `return` | `none` | `run` | `inline` | The loader puts `TestYield` after the entry label of each function. It costs one reduction, and the process switches out when the budget is then 0 or less. `return` returns from the frame and costs one reduction, and the process switches out when the budget is then less than 0. After a switch at the `return` of its last frame, the process exits in its next turn. | `O(1)` | `src/runtime/interpreter.rs`, `src/beam/code/instruction.rs` |
| Unsupported opcode | `none` | `pure` | `inline` | Each other opcode that the loader accepts, for example `gc_bif3`, `raise`, the float instructions, and most binary instructions, gives the runtime error `unsupported BEAM opcode` when it runs. Execution of `func_info` gives a `function clause` runtime error. | `O(1)` | `src/runtime/interpreter.rs` |

## VM operations

These entries change or read the shared state of the run. BIFs, instructions, and scheduler operations call them.

| Entry | Authority | Determinism | Execution | Effects | Cost | Owner |
| --- | --- | --- | --- | --- | --- | --- |
| Process creation | `process.spawn`, `process.link`, `process.monitor` | `run` | `inline` | `Scheduler::spawn` gives the next pid, copies the group leader of the parent, and puts the child at the end of the run queue. The `link` and `monitor` options add the link or the monitor to both processes, with no signal. Pid overflow gives a runtime error. | `O(n)` | `src/runtime/scheduler.rs`, `src/runtime/interpreter.rs` |
| Message delivery | `process.send` | `run` | `inline` | `Scheduler::send_message` puts the message of a process at the end of the queue of a live process, after the pending monitor signal of the sender to the same process, and makes a waiting process active. `Scheduler::deliver` does the same for a message of the runtime, for example a timer message or a reply to a request signal. A message to a process that does not exist or exits is dropped. | `O(1)` | `src/runtime/scheduler.rs`, `src/runtime/mailbox.rs` |
| Exit signal | `process.exit` | `run` | `inline` | `Scheduler::send_signal` puts an exit signal of `erlang:exit/2` or of exit cleanup at the end of the queue of its receiver. When the receiver handles it (row "Signal handling"), the signal gives `{'EXIT', From, Reason}` to a process that traps exits, is ignored for `normal` to another process, or ends the receiver. `kill` of `erlang:exit/2` becomes `killed`. | `O(1)` | `src/runtime/scheduler/signals.rs` |
| Links | `process.link` | `run` | `inline` | `Scheduler::link` and `Scheduler::unlink` change the links of the caller and send a link or an unlink signal. The target changes its side of the link when it handles the signal, and its unlink acknowledgment removes the unlinking link of the caller. A link to a process that does not exist or exits gives a runtime error. | `O(l)` | `src/runtime/scheduler.rs`, `src/runtime/scheduler/signals.rs` |
| Monitors | `process.monitor`, `process.name` | `run` | `inline` | `Scheduler::monitor` adds the monitor to the monitor tree of the caller and sends a monitor signal through the pending signal slot, or a `noproc` monitor down signal to the caller. `Scheduler::demonitor` removes the monitor from the tree and sends a demonitor signal. The target adds or removes its side when it handles the signal. | `O(l)` | `src/runtime/scheduler.rs`, `src/runtime/scheduler/signals.rs` |
| Aliases | `process.alias` | `run` | `inline` | `Scheduler::monitor` makes an alias for the `alias` option: the alias mode goes into the monitor tree of the caller, and `Scheduler::alias_owner` gives the caller for the reference. `Scheduler::send_alias_message` and `erlang:exit/2` send a signal through an alias to its owner, which checks the alias when it handles the signal. A `reply_demonitor` alias ends with its first message, which also removes its monitor. The references of a process lose their owner when it exits. | `O(l)` | `src/runtime/scheduler.rs`, `src/runtime/scheduler/signals.rs` |
| Group leader | `process.group_leader` | `run` | `inline` | `erts_internal:group_leader/2` sets the group leader of a process with no incoming signals at once. A group-leader signal sets it when the target handles the signal. | `O(1)` | `src/runtime/interpreter/bif/erts_internal.rs`, `src/runtime/scheduler/signals.rs` |
| Registered names | `process.name` | `run` | `inline` | `Scheduler::register` adds a name to the node registry. `Scheduler::whereis` reads it. Exit cleanup removes the name. | `O(1)` | `src/runtime/scheduler.rs` |
| Process inspection | `process.inspect` | `run` | `inline` | `Scheduler::process` reads a process of the process table, also an exiting process, and `Scheduler::live_process` reads a process that does not exit. `ProcessState::info` gives the registered name, the message count, and the reductions of a process. | `O(q)` | `src/runtime/scheduler.rs`, `src/runtime/process.rs` |
| Process state | `process.self` | `run` | `inline` | `ProcessState` keeps the dictionary, `trap_exit`, `message_queue_data`, the group leader, the receive timer, the links, the monitor tree, the monitors of the process, the run-queue and exit state, and the signal queue of each process. | `O(d)` | `src/runtime/process.rs` |
| Reduction budget | `runtime.service` | `run` | `inline` | `ProcessState::fcalls` keeps the reductions that remain in the turn of a process. Function entries, returns, light BIF calls, sends, and the BIFs whose ERTS cost depends on their input, for example the list BIFs, `maps:merge/2`, and `iolist_size/1`, use it, and the process switches out at the ERTS switch points. `erlang:bump_reductions/1` uses it down to 0. A local send uses virtual reductions, which `Scheduler::virtual_reductions` keeps for the turn. `ProcessState::reductions` keeps the reduction total. | `O(1)` | `src/runtime/process.rs`, `src/runtime/scheduler.rs`, `src/runtime/interpreter.rs` |
| References | `none` | `run` | `inline` | `Scheduler::make_ref` gives the next reference of the run. Reference overflow gives a runtime error. | `O(1)` | `src/runtime/scheduler.rs` |
| ETS tables | `runtime.ets` | `run` | `inline` | `EtsTables` keeps each table, its owner, its access mode, and its name. Exit cleanup deletes the tables of the owner. A table keeps copies of its objects. A key search reads each object of the table. | `O(t)` | `src/runtime/ets.rs`, `src/runtime/interpreter/bif/ets.rs` |
| Persistent terms | `runtime.persistent_term` | `run` | `inline` | `PersistentTerms` keeps each key and value of the node until the run ends. There is no erase. | `O(t)` | `src/runtime/persistent_term.rs` |
| Atoms | `runtime.atom` | `run` | `inline` | `AtomTable::put` adds an atom to the atom table of the run. An atom is never released. A full table of 1048576 atoms gives a runtime error. | `O(1)` | `src/term/atom.rs` |
| Mailboxes | `process.self`, `process.send` | `run` | `inline` | `Mailbox` is the signal queue of one process: its messages, receive markers, and signals in arrival order, in an inner, a middle, and an outer part. A sent message is a copy of the term, and it goes to the outer part with the signals. A fetch moves the outer part into the inner part, or into the middle part behind a signal. A receive reads only the inner part. There is no queue limit. | `O(q)` | `src/runtime/mailbox.rs` |
| Binary storage | `none` | `run` | `inline` | PBeam has no shared or reference-counted binary store. Each binary term owns its bytes, and a send, an ETS insert, or a persistent term put copies them. | `O(n)` | `src/term.rs` |
| Resource store | `runtime.resource` | `run` | `inline` | `Resources` keeps one value of each Rust type for the node. It holds `RegexCodes` and `Ttys`. A reference locates a record but is not an authority. | `O(1)` | `src/runtime/resources.rs` |
| Compiled regular expressions | `runtime.resource` | `run` | `inline` | `RegexCodes` keeps each compiled PCRE2 code by its reference until the run ends. | `O(t)` | `src/runtime/regex_code.rs` |
| TTY records | `runtime.resource`, `terminal.raw` | `run` | `inline` | `Ttys` keeps each TTY record by its reference, with its output fd, state, modes, owner, and monitors. A record is never removed. | `O(t)` | `src/runtime/tty.rs` |
| NIF library state | `runtime.code` | `run` | `inline` | `Scheduler::nif_libraries` records each module whose library `erlang:load_nif/2` loaded in the run. | `O(1)` | `src/runtime/scheduler.rs` |
| System logger | `runtime.info` | `run` | `inline` | `Scheduler::system_logger` keeps the node value of `system_logger`. PBeam writes no error report with it. | `O(1)` | `src/runtime/scheduler.rs` |
| Timers | `runtime.timer` | `run` | `inline` | `Timers` starts, cancels, and orders the receive timers and the BIF timers. Exit cleanup cancels the timers of the process. | `O(t)` | `src/runtime/timer.rs` |

## Host operations

These entries call the host or a linked C library. A process-wide effect changes the OS process, not one BEAM process.

| Entry | Authority | Determinism | Execution | Effects | Cost | Owner |
| --- | --- | --- | --- | --- | --- | --- |
| `host::os_flavor` | `host.os_info` | `host` | `host-call` | Calls `uname` and gives the lowercase system name. | `O(1)` | `src/runtime/host.rs` |
| `host::filename_encoding` | `host.locale` | `host` | `host-call` | On Linux, the first call reads the codeset of the `LC_CTYPE` locale from the environment with `newlocale` and `nl_langinfo_l`, and keeps the result for the process. On macOS, the result is always `utf8` with no host call. The global locale does not change. | `O(1)` | `src/runtime/host.rs` |
| `host::clock::system_time` | `host.clock.system` | `host` | `host-call` | Reads `CLOCK_REALTIME`. | `O(1)` | `src/runtime/host/clock.rs` |
| `host::clock::local_time` | `host.timezone` | `host` | `host-call` | Calls `tzset`, then `localtime_r`. On macOS it first calls `posix2time`. `tzset` changes the time zone state of the C library for the OS process. | `unknown` | `src/runtime/host/clock.rs` |
| `host::environment::variable` | `host.environment` | `host` | `host-call` | Reads the `environ` array of the OS process. PBeam never changes its environment. | `O(e)` | `src/runtime/host/environment.rs` |
| `host::errno` | `none` | `host` | `host-call` | `errno` reads the thread-local `errno`. `errno_id` maps a number to its ERTS atom name with no host call. | `O(1)` | `src/runtime/host/errno.rs` |
| `host::output::write_all` | `terminal.text_write`, `terminal.tui_control` | `host` | `host-blocking` | Calls `writev` on an fd until it writes all bytes. It repeats the call after `EINTR` and `EAGAIN`, so a non-blocking fd that stays full keeps the scheduler thread busy. | `O(n)` | `src/runtime/host/output.rs` |
| `host::signal::set_signal` | `host.signal` | `pure` | `host-call` | Calls `sigaction` with `SIG_IGN`, `SIG_DFL`, or a handler that does nothing. The disposition applies to the OS process. | `O(1)` | `src/runtime/host/signal.rs` |
| `host::terminal::check` | `terminal.query` | `host` | `host-call` | Calls `isatty` on an fd. | `O(1)` | `src/runtime/host/terminal.rs` |
| `host::terminal::setlocale_utf8` | `host.locale`, `host.environment` | `host` | `host-call` | Calls `setlocale(LC_CTYPE, "")` and reads the codeset. This sets the C library locale of the OS process from the environment. | `unknown` | `src/runtime/host/terminal.rs` |
| `host::terminal::setupterm` | `terminal.setup`, `host.environment`, `host.filesystem.read` | `host` | `host-blocking` | Calls the curses `setupterm` for `TERM` and fd -1, which reads the terminfo database. It keeps the new `cur_term` of the curses library and frees the earlier one. | `unknown` | `src/runtime/host/terminal.rs` |
| `host::terminal::window_size` | `terminal.query` | `host` | `host-call` | Calls `ioctl(TIOCGWINSZ)` on fd 0, then on the output fd. | `O(1)` | `src/runtime/host/terminal.rs` |
| `host::terminal::get_mode` | `terminal.query` | `host` | `host-call` | Calls `tcgetattr` on an fd. | `O(1)` | `src/runtime/host/terminal.rs` |
| `host::terminal::set_mode` | `terminal.raw` | `host` | `host-call` | Calls `tcsetattr(TCSANOW)` on an fd. The terminal mode is shared with each other process that uses the terminal. | `O(1)` | `src/runtime/host/terminal.rs` |
| `host::terminal::raw_mode`, `host::terminal::same_mode` | `none` | `pure` | `inline` | Compute and compare `termios` values with no host call. | `O(1)` | `src/runtime/host/terminal.rs` |
| Run clock | `host.clock.monotonic` | `host` | `host-call` | `Timers::now` reads the Rust `Instant` clock: `CLOCK_UPTIME_RAW` on macOS and `CLOCK_MONOTONIC` on other Unix hosts. The scheduler and the guest-visible time BIFs read the same run clock. | `O(1)` | `src/runtime/timer.rs` |
| PCRE2 stack limit | `none` | `host` | `host-call` | `pcre2::compile` reads the stack bounds of the scheduler thread with `pthread` calls for the PCRE2 stack guard. | `O(1)` | `src/runtime/pcre2.rs` |

## Guest-visible clock access

These entries give a clock value, or a value that depends on a clock, to guest code. They are apart from the scheduler clock read.

| Entry | Clock |
| --- | --- |
| `erlang:monotonic_time/1` | The run clock, which the scheduler also reads. |
| `erlang:start_timer/3` | The run clock, for the timer end. The timer message order shows it. |
| `erlang:cancel_timer/2` | The run clock, for the time left. |
| `wait_timeout` | The run clock, for the receive timer end. |
| `os:system_time/1` | `CLOCK_REALTIME`. |
| `erlang:universaltime_to_localtime/1` | The host time zone data. |

## BIFs

Each row is a BIF entry of `BIFS` that has a body.

| Entry | Authority | Determinism | Execution | Effects | Cost | Owner |
| --- | --- | --- | --- | --- | --- | --- |
| `erlang:*/2` | `none` | `pure` | `inline` | Gives the product, or raises `badarith`. An `i64` overflow gives a runtime error. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:+/2` | `none` | `pure` | `inline` | Gives the sum, or raises `badarith`. An `i64` overflow gives a runtime error. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:++/2` | `process.self` | `run` | `inline`, `trap` | Appends two lists. Uses one reduction of the budget of the caller for each 60 elements of the left list, or for each 32 when the right list is `[]`. At the end of the budget, it traps to its export and continues in the next turn. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:-/2` | `none` | `pure` | `inline` | Gives the difference, or raises `badarith`. An `i64` overflow gives a runtime error. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:=/=/2` | `none` | `pure` | `inline` | Compares two terms for exact inequality. | `O(n)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:=:=/2` | `none` | `pure` | `inline` | Compares two terms for exact equality. | `O(n)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:>=/2` | `none` | `pure` | `inline` | Compares two terms in the term order. | `O(n)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:atom_to_list/1` | `none` | `pure` | `inline` | Gives the characters of an atom. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:bump_reductions/1` | `process.self` | `run` | `inline` | Uses the given number of reductions, but not more than 4000, of the budget of the caller, and gives `true`. A value that is not a small integer, or a negative value, raises `badarg`. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:cancel_timer/2` | `runtime.timer`, `process.self`, `host.clock.monotonic` | `host` | `host-call` | Cancels a BIF timer of the run and gives the time left, `false`, or `ok`. With `{async, true}`, it sends `{cancel_timer, Ref, Result}` to the caller. | `O(t)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:demonitor/1` | `process.monitor`, `process.alias` | `run` | `inline` | Removes a monitor of the caller from its monitor tree, ends its alias unless the alias is `explicit_unalias`, and sends a demonitor signal to the target. | `O(l)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/scheduler.rs` |
| `erlang:demonitor/2` | `process.monitor`, `process.alias`, `process.self` | `run` | `inline`, `trap` | As `erlang:demonitor/1`. With `flush` and no monitor removed, it traps to `erts_internal:flush_monitor_messages/3`, which removes `DOWN` messages from the own queue. | `O(l)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:div/2` | `none` | `pure` | `inline` | Gives the integer quotient, or raises `badarith`. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:element/2` | `none` | `pure` | `inline` | Gives a copy of a tuple element, or raises `badarg`. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:error/1` | `none` | `pure` | `inline` | Raises an exception of class `error` with a copy of the reason. | `O(n)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:error/3` | `none` | `pure` | `inline` | Raises an exception of class `error` with a copy of the reason. | `O(n)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:exit/1` | `none` | `pure` | `inline` | Raises an exception of class `exit` with a copy of the reason. | `O(n)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:exit/2` | `process.exit`, `process.alias`, `process.self` | `run` | `inline`, `trap` | Sends an exit signal to a pid or through an alias. The receiver acts on it when it handles the signal. A signal that ends the caller traps to `erts_internal:await_exit/0`, which handles the signals of the caller until the caller exits. | `O(q)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/interpreter/bif.rs` |
| `erlang:function_exported/3` | `runtime.code` | `run` | `inline` | Tells whether a loaded module exports the function or the function is a BIF. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:get/1` | `process.self` | `run` | `inline` | Reads a value of the own dictionary. | `O(d)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:group_leader/0` | `process.self` | `run` | `inline` | Gives the group leader of the caller. | `O(1)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:hd/1` | `none` | `pure` | `inline` | Gives a copy of the head of a list, or raises `badarg`. | `O(n)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:integer_to_list/1` | `process.self` | `run` | `inline` | Gives the decimal characters of an integer. A bignum uses one reduction of the budget of the caller. A bignum when no reductions remain gives a runtime error. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:iolist_size/1` | `process.self` | `run` | `inline`, `trap` | Gives the byte size of iodata. Uses one reduction of the budget of the caller for each 8 cells and list ends. At the end of the budget, it traps to an internal export and continues in the next turn. | `O(n)` | `src/runtime/interpreter/bif/iolist.rs` |
| `erlang:iolist_to_iovec/1` | `process.self` | `run` | `inline`, `trap` | Gives a list of binaries with copies of the bytes. Uses one reduction of the budget of the caller for each 8 cells and list ends of the size walk and of the copy walk, and for each 256 copied bytes. At the end of the budget, it traps to an internal export and continues in the next turn. | `O(n)` | `src/runtime/interpreter/bif/iolist.rs` |
| `erlang:is_function/1` | `none` | `pure` | `inline` | Tells whether a term is a fun. | `O(1)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:is_function/2` | `none` | `pure` | `inline` | Tells whether a term is a fun of an arity. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:is_map_key/2` | `none` | `pure` | `inline` | Tells whether a map has a key. | `O(n)` | `src/runtime/interpreter/bif/maps.rs` |
| `erlang:is_process_alive/1` | `process.inspect`, `process.self` | `run` | `inline`, `trap` | Tells whether a process is alive. A process that exits or has incoming signals gets an is-alive request through `erts_internal:is_process_alive/1`. A result traps to `erlang:bif_handle_signals_return/2` when the caller has incoming signals. | `O(q)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:length/1` | `process.self` | `run` | `inline`, `trap` | Gives the length of a proper list. Uses one reduction of the budget of the caller for each 16 elements. At the end of the budget, it traps to its export and continues in the next turn. The guard BIF keeps its trap state in the process and runs the instruction again. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/interpreter.rs` |
| `erlang:link/1` | `process.link` | `run` | `inline` | Links the caller and a pid, and sends a link signal for a new link. | `O(l)` | `src/runtime/interpreter/bif/table.rs`, `src/runtime/scheduler.rs` |
| `erlang:list_to_atom/1` | `runtime.atom` | `run` | `inline` | Gives the atom of a character list and adds a new atom to the atom table. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:load_nif/2` | `runtime.atom` | `pure` | `inline` | A call through `apply` gives `{error, {bad_lib, Text}}` and loads no library. A call from module code is the `i_load_nif` instruction path. | `O(1)` | `src/runtime/interpreter/native.rs` |
| `erlang:make_fun/3` | `none` | `pure` | `inline` | Gives an external fun. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:make_ref/0` | `none` | `run` | `inline` | Gives the next reference of the run. | `O(1)` | `src/runtime/interpreter/bif/table.rs`, `src/runtime/scheduler.rs` |
| `erlang:map_get/2` | `none` | `pure` | `inline` | Gives the value of a map key, or raises. | `O(n)` | `src/runtime/interpreter/bif/maps.rs` |
| `erlang:min/2` | `none` | `pure` | `inline` | Gives the smaller term in the term order. | `O(n)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:monitor/2` | `process.monitor`, `process.name` | `run` | `inline` | Adds a monitor of a process or a name to the caller and sends a monitor signal, or sends a `noproc` monitor down signal to the caller. | `O(l)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/scheduler.rs` |
| `erlang:monitor/3` | `process.monitor`, `process.name`, `process.alias` | `run` | `inline` | As `erlang:monitor/2`. The `alias` option makes an alias, and the `tag` option sets the `DOWN` tag. The `priority` option gives a runtime error. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/scheduler.rs` |
| `erlang:monotonic_time/1` | `host.clock.monotonic` | `host` | `host-call` | Gives the run clock in a time unit. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/timer.rs` |
| `erlang:nif_error/1` | `none` | `pure` | `inline` | Raises an exception of class `error` with a copy of the reason. | `O(n)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:node/0` | `runtime.info` | `pure` | `inline` | Gives `nonode@nohost`. | `O(1)` | `src/runtime/mod.rs` |
| `erlang:node/1` | `runtime.info` | `pure` | `inline` | Gives `nonode@nohost` for a pid. Another term gives a runtime error. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:not/1` | `none` | `pure` | `inline` | Gives the boolean negation, or raises `badarg`. | `O(1)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:posixtime_to_universaltime/1` | `none` | `pure` | `inline` | Converts POSIX seconds to a UTC date and time. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/time.rs` |
| `erlang:process_flag/2` | `process.self` | `run` | `inline` | Sets `trap_exit` or `message_queue_data` of the caller and gives the old value. Other valid flags give a runtime error. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:process_info/2` | `process.inspect`, `process.self` | `run` | `inline`, `trap` | Gives `registered_name`, `message_queue_len`, or `reductions` of a process, or `current_stacktrace` of the caller as `[]`. For the caller, it first handles the signals that the caller sent to itself. A process that exits or has incoming signals gets a process-info request, and the caller traps to `erts_internal:await_result/1`. A result traps to `erlang:bif_handle_signals_return/2` when the caller has incoming signals. The reductions of the caller include its current turn. Each read item costs the caller one reduction. Other items give a runtime error. | `O(q)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/process.rs` |
| `erlang:put/2` | `process.self` | `run` | `inline` | Writes a value to the own dictionary and gives the old value. | `O(d)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:raise/3` | `none` | `pure` | `inline` | Raises an exception of class `error` or `exit`, or gives `badarg`. The class `throw` gives a runtime error. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:register/2` | `process.name` | `run` | `inline` | Registers a name for a live process. | `O(1)` | `src/runtime/interpreter/bif/table.rs`, `src/runtime/scheduler.rs` |
| `erlang:rem/2` | `none` | `pure` | `inline` | Gives the integer remainder, or raises `badarith`. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:self/0` | `process.self` | `run` | `inline` | Gives the pid of the caller. | `O(1)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:send/2` | `process.send`, `process.name`, `process.alias` | `run` | `inline` | Sends a copy of a message to a pid, a registered name, or `{Name, nonode@nohost}`, or as a signal through an alias. A message to another node is dropped. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/scheduler.rs` |
| `erlang:send/3` | `process.send`, `process.name`, `process.alias` | `run` | `inline` | As `erlang:send/2`, with the options `noconnect`, `nosuspend`, and `priority`. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/scheduler.rs` |
| `erlang:spawn/3` | `process.spawn`, `runtime.code` | `run` | `inline` | Makes a process that runs `M:F(Args)`. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/interpreter.rs` |
| `erlang:spawn_link/3` | `process.spawn`, `process.link`, `runtime.code` | `run` | `inline` | As `erlang:spawn/3`, and links the child. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/interpreter.rs` |
| `erlang:spawn_opt/4` | `process.spawn`, `process.link`, `process.monitor`, `runtime.code` | `run` | `inline` | As `erlang:spawn/3`, with the options `link`, `monitor`, and `{message_queue_data, Mode}`. Other options give a runtime error. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/interpreter.rs` |
| `erlang:start_timer/3` | `runtime.timer`, `process.send`, `process.name`, `host.clock.monotonic` | `host` | `host-call` | Reads the run clock and starts a BIF timer that sends `{timeout, Ref, Msg}`. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/timer.rs` |
| `erlang:system_flag/2` | `runtime.info` | `run` | `inline` | Sets `system_logger` for the node and gives the old value. `debug_flags` gives `true`. Other valid items give a runtime error. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:system_info/1` | `runtime.info`, `host.os_info`, `runtime.atom` | `host` | `host-call` | Gives `{unix, Flavor}` for `os_type` from `uname`, and adds the atoms. Other items give a runtime error. | `O(1)` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/host.rs` |
| `erlang:tl/1` | `none` | `pure` | `inline` | Gives the tail of a list, or raises `badarg`. | `O(n)` | `src/runtime/interpreter/bif/erlang.rs` |
| `erlang:tuple_size/1` | `none` | `pure` | `inline` | Gives the size of a tuple. | `O(1)` | `src/runtime/interpreter/bif/table.rs` |
| `erlang:universaltime_to_localtime/1` | `host.timezone` | `host` | `host-call` | Converts a UTC date and time to the host time zone. | `unknown` | `src/runtime/interpreter/bif/erlang.rs`, `src/runtime/host/clock.rs` |
| `erlang:unlink/1` | `process.link` | `run` | `inline` | Unlinks the caller and a pid, and sends an unlink signal for an active link. | `O(l)` | `src/runtime/interpreter/bif/table.rs`, `src/runtime/scheduler.rs` |
| `erlang:whereis/1` | `process.name` | `run` | `inline` | Gives the pid of a registered name, or `undefined`. | `O(1)` | `src/runtime/interpreter/bif/table.rs`, `src/runtime/scheduler.rs` |
| `erts_internal:group_leader/2` | `process.group_leader` | `run` | `inline` | Sets the group leader of a process with no incoming signals and gives `true`. Gives `false` for another process with incoming signals, and `badarg` for an argument that is not a pid, a pid that no process has, and an exiting process. | `O(1)` | `src/runtime/interpreter/bif/erts_internal.rs` |
| `erts_internal:group_leader/3` | `process.group_leader`, `process.self` | `run` | `inline` | Sends a group-leader signal to a pid. The target replies `{Ref, true}`, or `{Ref, badarg}` when it exits first. A pid that no process has gives `{Ref, badarg}` to the caller at once. | `O(1)` | `src/runtime/interpreter/bif/erts_internal.rs` |
| `erts_internal:is_process_alive/2` | `process.inspect`, `process.self` | `run` | `inline`, `trap` | Sends an is-alive request to a pid. The process replies `{Ref, true}`, or `{Ref, false}` when it exits first. A pid that no process has gives `{Ref, false}` to the caller at once, and the result then traps to `erlang:bif_handle_signals_return/2` when the caller has incoming signals. | `O(1)` | `src/runtime/interpreter/bif/erts_internal.rs` |
| `erts_internal:map_next/3` | `process.self` | `run` | `inline` | Gives the next part of a map iterator. The budget of the caller sets the number of pairs of a hashmap and of keys of an ordered iterator path. The call uses 4 reductions for each pair, and 32 for each key or all reductions for an ordered iterator path. | `O(n)` | `src/runtime/interpreter/bif/maps.rs`, `src/term/map.rs` |
| `erts_internal:perf_counter_unit/0` | `none` | `pure` | `inline` | Gives the constant unit of the performance counter. | `O(1)` | `src/runtime/time.rs` |
| `ets:delete/2` | `runtime.ets` | `run` | `inline` | Removes the object of a key from a table that the caller can write. | `O(t)` | `src/runtime/interpreter/bif/ets.rs` |
| `ets:insert/2` | `runtime.ets`, `process.self` | `run` | `inline`, `trap` | Inserts copies of objects into a table that the caller can write. A list of two or more objects uses one reduction of the budget of the caller for each 8 units of the ERTS insert: 3 or 4 units for each object. At the end of the budget, it traps to an internal export and continues in the next turn. | `O(t)` | `src/runtime/interpreter/bif/ets.rs` |
| `ets:lookup/2` | `runtime.ets` | `run` | `inline` | Gives a copy of the object of a key from a table that the caller can read. | `O(t)` | `src/runtime/interpreter/bif/ets.rs` |
| `ets:lookup_element/4` | `runtime.ets` | `run` | `inline` | Gives an element of the object of a key, or the default. | `O(t)` | `src/runtime/interpreter/bif/ets.rs` |
| `ets:member/2` | `runtime.ets` | `run` | `inline` | Tells whether a table has a key. | `O(t)` | `src/runtime/interpreter/bif/ets.rs` |
| `ets:new/2` | `runtime.ets` | `run` | `inline` | Makes a `set` table that the caller owns. A named table registers its name for the node. Other table types and a live heir give a runtime error. | `O(n)` | `src/runtime/interpreter/bif/ets.rs`, `src/runtime/ets.rs` |
| `ets:whereis/1` | `runtime.ets` | `run` | `inline` | Gives the reference of a named table, or `undefined`. | `O(1)` | `src/runtime/interpreter/bif/table.rs`, `src/runtime/ets.rs` |
| `io:printable_range/0` | `none` | `pure` | `inline` | Gives `latin1`. | `O(1)` | `src/runtime/interpreter/bif/table.rs` |
| `lists:keyfind/3` | `process.self` | `run` | `inline`, `trap` | Finds the first tuple with a key. Costs no reduction. After 40000 elements, it uses the budget of the caller, traps to its export, and continues in the next turn. | `O(n)` | `src/runtime/interpreter/bif/lists.rs` |
| `lists:keymember/3` | `process.self` | `run` | `inline`, `trap` | Tells whether a tuple has a key. Costs and traps as `lists:keyfind/3`. | `O(n)` | `src/runtime/interpreter/bif/lists.rs` |
| `lists:member/2` | `process.self` | `run` | `inline`, `trap` | Tells whether a list has an element. Uses one reduction of the budget of the caller for each 16 compared elements. At the end of the budget, it traps to its export and continues in the next turn. | `O(n)` | `src/runtime/interpreter/bif/lists.rs` |
| `lists:reverse/2` | `process.self` | `run` | `inline`, `trap` | Reverses a list onto a tail. Uses one reduction of the budget of the caller for each 60 elements. At the end of the budget, it traps to its export and continues in the next turn. | `O(n)` | `src/runtime/interpreter/bif/lists.rs` |
| `maps:find/2` | `none` | `pure` | `inline` | Gives `{ok, Value}` or `error`. | `O(n)` | `src/runtime/interpreter/bif/maps.rs` |
| `maps:from_list/1` | `none` | `pure` | `inline` | Makes a map from a list of pairs. | `O(n)` | `src/runtime/interpreter/bif/maps.rs` |
| `maps:keys/1` | `none` | `pure` | `inline` | Gives the keys of a map. | `O(n)` | `src/runtime/interpreter/bif/maps.rs` |
| `maps:merge/2` | `process.self` | `run` | `inline`, `trap` | Merges two maps. Unless both maps are flatmaps, uses one reduction of the budget of the caller for each 8 steps of the ERTS hashmap merge. At the end of the budget, it traps to an internal export and continues in the next turn. | `O(n)` | `src/runtime/interpreter/bif/maps.rs`, `src/term/map.rs` |
| `maps:remove/2` | `none` | `pure` | `inline` | Removes a key from a map. | `O(n)` | `src/runtime/interpreter/bif/maps.rs` |
| `net_kernel:dflag_unicode_io/1` | `none` | `pure` | `inline` | Gives `true` for a pid, or raises `badarg`. | `O(1)` | `src/runtime/interpreter/bif/table.rs` |
| `os:getenv/1` | `host.environment`, `host.locale` | `host` | `host-call` | Gives the value of an environment variable, or `false`. The name and the value use the host file name encoding. | `O(e)` | `src/runtime/interpreter/bif/os.rs`, `src/runtime/host/environment.rs` |
| `os:set_signal/2` | `host.signal` | `pure` | `host-call` | Sets the disposition of a signal for the OS process and gives `ok`. The `handle` handler sends no message. | `O(1)` | `src/runtime/interpreter/bif/os.rs`, `src/runtime/host/signal.rs` |
| `os:system_time/1` | `host.clock.system` | `host` | `host-call`, `trap` | Gives `CLOCK_REALTIME` in a time unit. A time before 1970 traps for the unit conversion. | `O(1)` | `src/runtime/interpreter/bif/os.rs`, `src/runtime/host/clock.rs` |
| `persistent_term:get/0` | `runtime.persistent_term`, `process.self` | `run` | `inline`, `trap` | Gives each key and value of the node. Uses one reduction of the budget of the caller for each pair. At the end of the budget, it traps to an internal export and continues in the next turn with the same result. | `O(t)` | `src/runtime/persistent_term.rs` |
| `persistent_term:get/1` | `runtime.persistent_term` | `run` | `inline` | Gives the value of a key, or raises `badarg`. | `O(n)` | `src/runtime/persistent_term.rs` |
| `persistent_term:get/2` | `runtime.persistent_term` | `run` | `inline` | Gives the value of a key, or the default. | `O(n)` | `src/runtime/persistent_term.rs` |
| `persistent_term:put/2` | `runtime.persistent_term` | `run` | `inline` | Stores a copy of a value for the node until the run ends. | `O(n)` | `src/runtime/persistent_term.rs` |
| `re:compile/2` | `runtime.resource` | `host` | `host-call`, `trap` | Compiles a pattern with the linked PCRE2 C code and keeps the code in `RegexCodes` until the run ends. A `unicode` pattern that is not a binary traps to `re:ucompile/2`. The result depends on the free stack of the scheduler thread: the PCRE2 stack guard rejects a deeply nested pattern on a small stack. | `unknown` | `src/runtime/interpreter/bif/re.rs`, `src/runtime/pcre2.rs` |
| `unicode:characters_to_binary/2` | `none` | `pure` | `inline` | Converts chardata to a UTF-8 or Latin-1 binary. | `O(n)` | `src/runtime/interpreter/bif/unicode.rs`, `src/runtime/unicode.rs` |
| `unicode:characters_to_list/2` | `none` | `pure` | `inline` | Converts chardata to a list of characters. | `O(n)` | `src/runtime/interpreter/bif/unicode.rs`, `src/runtime/unicode.rs` |

## Unsupported BIFs

Each entry is a BIF entry of `BIFS` with the form `unsupported`. A call gives the runtime error `unresolved external MFA`, and the run stops. The BEAM stub of the entry never runs. These entries have no effect. Authority: `none`. Determinism: `pure`. Execution: `inline`. Cost: `O(1)`. Owner: `src/runtime/interpreter/bif/table.rs`. A PR that implements one of these BIFs moves it to the section "BIFs".

- `atomics`: `add/3`, `add_get/3`, `compare_exchange/4`, `exchange/3`, `get/2`, `info/1`, `put/3`
- `binary`: `at/2`, `compile_pattern/1`, `copy/1`, `copy/2`, `decode_unsigned/1`, `decode_unsigned/2`, `encode_unsigned/1`, `encode_unsigned/2`, `first/1`, `last/1`, `list_to_bin/1`, `longest_common_prefix/1`, `longest_common_suffix/1`, `match/2`, `match/3`, `matches/2`, `matches/3`, `part/2`, `part/3`, `referenced_byte_size/1`, `split/2`, `split/3`
- `code`: `coverage_support/0`, `get_coverage/2`, `get_coverage_mode/0`, `get_coverage_mode/1`, `get_debug_info/1`, `reset_coverage/1`, `set_coverage_mode/1`
- `erl_ddll`: `demonitor/1`, `format_error_int/1`, `info/2`, `loaded_drivers/0`, `monitor/2`, `try_load/3`, `try_unload/2`
- `erl_debugger`: `breakpoint/3`, `breakpoints/1`, `breakpoints/3`, `instrumentations/0`, `peek_stack_frame_slot/4`, `peek_xreg/3`, `register/1`, `stack_frames/2`, `supported/0`, `toggle_instrumentations/1`, `unregister/2`, `whereis/0`, `xregs_count/1`
- `erlang`: `!/2`, `+/1`, `-/1`, `--/2`, `//2`, `/=/2`, `</2`, `=</2`, `==/2`, `>/2`, `abs/1`, `adler32/1`, `adler32/2`, `adler32_combine/3`, `alias/1`, `and/2`, `append/2`, `append_element/2`, `atom_to_binary/2`, `band/2`, `binary_part/2`, `binary_part/3`, `binary_to_atom/2`, `binary_to_existing_atom/2`, `binary_to_float/1`, `binary_to_list/1`, `binary_to_list/3`, `binary_to_term/1`, `binary_to_term/2`, `bit_size/1`, `bitstring_to_list/1`, `bnot/1`, `bor/2`, `bsl/2`, `bsr/2`, `bxor/2`, `byte_size/1`, `call_on_load_function/1`, `cancel_timer/1`, `ceil/1`, `check_old_code/1`, `crc32/1`, `crc32/2`, `crc32_combine/3`, `date/0`, `decode_packet/3`, `delete_element/2`, `delete_module/1`, `display/1`, `display_string/2`, `dist_ctrl_get_data/1`, `dist_ctrl_get_data_notification/1`, `dist_ctrl_get_opt/2`, `dist_ctrl_input_handler/2`, `dist_ctrl_put_data/2`, `dist_ctrl_set_opt/3`, `dist_get_stat/1`, `dt_append_vm_tag_data/1`, `dt_get_tag/0`, `dt_get_tag_data/0`, `dt_prepend_vm_tag_data/1`, `dt_put_tag/1`, `dt_restore_tag/1`, `dt_spread_tag/1`, `erase/0`, `erase/1`, `error/2`, `exit/3`, `exit_signal/2`, `exit_signal/3`, `external_size/1`, `external_size/2`, `finish_after_on_load/2`, `finish_loading/1`, `float/1`, `float_to_binary/1`, `float_to_binary/2`, `float_to_list/1`, `float_to_list/2`, `floor/1`, `fun_info/2`, `fun_info_mfa/1`, `fun_to_list/1`, `get/0`, `get_keys/0`, `get_keys/1`, `get_module_info/1`, `get_module_info/2`, `halt/2`, `has_prepared_code_on_load/1`, `hibernate/3`, `insert_element/3`, `integer_to_binary/1`, `integer_to_binary/2`, `integer_to_list/2`, `iolist_to_binary/1`, `is_atom/1`, `is_binary/1`, `is_bitstring/1`, `is_boolean/1`, `is_builtin/3`, `is_float/1`, `is_integer/1`, `is_integer/3`, `is_list/1`, `is_map/1`, `is_number/1`, `is_pid/1`, `is_port/1`, `is_record/1`, `is_record/2`, `is_record/3`, `is_reference/1`, `is_tuple/1`, `link/2`, `list_to_binary/1`, `list_to_bitstring/1`, `list_to_existing_atom/1`, `list_to_float/1`, `list_to_pid/1`, `list_to_port/1`, `list_to_ref/1`, `list_to_tuple/1`, `loaded/0`, `localtime/0`, `localtime_to_universaltime/2`, `make_tuple/2`, `make_tuple/3`, `map_size/1`, `match_spec_test/3`, `max/2`, `md5/1`, `md5_final/1`, `md5_init/0`, `md5_update/2`, `module_loaded/1`, `monitor_node/2`, `monitor_node/3`, `monotonic_time/0`, `nif_error/2`, `nodes/0`, `nodes/1`, `nodes/2`, `now/0`, `or/2`, `phash/2`, `phash2/1`, `phash2/2`, `pid_to_list/1`, `port_get_data/1`, `port_set_data/2`, `port_to_list/1`, `ports/0`, `pre_loaded/0`, `process_info/1`, `processes/0`, `read_timer/1`, `read_timer/2`, `ref_to_list/1`, `registered/0`, `resume_process/1`, `round/1`, `send_after/3`, `send_after/4`, `seq_trace/2`, `seq_trace_info/1`, `seq_trace_print/1`, `seq_trace_print/2`, `setelement/3`, `setnode/2`, `size/1`, `spawn_request_abandon/1`, `split_binary/2`, `start_timer/4`, `statistics/1`, `subtract/2`, `system_profile/0`, `system_profile/2`, `system_time/0`, `system_time/1`, `term_to_binary/1`, `term_to_binary/2`, `term_to_iovec/1`, `term_to_iovec/2`, `throw/1`, `time/0`, `time_offset/0`, `time_offset/1`, `timestamp/0`, `trace_delivered/1`, `trace_info/2`, `trunc/1`, `tuple_to_list/1`, `unalias/1`, `unique_integer/0`, `unique_integer/1`, `universaltime/0`, `universaltime_to_posixtime/1`, `unregister/1`, `xor/2`
- `error_logger`: `warning_map/0`
- `erts_debug`: `breakpoint/2`, `copy_shared/2`, `dirty/3`, `dirty_cpu/2`, `dirty_io/2`, `disassemble/1`, `dist_ext_to_term/2`, `flat_size/1`, `get_internal_state/1`, `instructions/0`, `interpreter_size/0`, `lcnt_clear/0`, `lcnt_collect/0`, `lcnt_control/1`, `lcnt_control/2`, `map_info/1`, `same/2`, `set_internal_state/2`, `size_shared/1`, `unaligned_bitstring/2`
- `erts_internal`: `abort_pending_connection/2`, `atomics_new/2`, `beamfile_chunk/2`, `beamfile_module_md5/1`, `binary_to_integer/2`, `check_dirty_process_code/2`, `check_process_code/1`, `cmp_term/2`, `counters_add/3`, `counters_get/2`, `counters_info/1`, `counters_new/1`, `counters_put/3`, `create_dist_channel/3`, `dirty_process_handle_signals/1`, `dist_spawn_request/4`, `erase_persistent_terms/0`, `ets_lookup_binary_info/2`, `ets_raw_first/1`, `ets_raw_next/2`, `ets_super_user/1`, `garbage_collect/1`, `gather_alloc_histograms/1`, `gather_carrier_info/1`, `get_creation/0`, `get_dflags/0`, `is_process_executing_dirty/1`, `is_system_process/1`, `list_to_integer/2`, `map_hashmap_children/1`, `map_to_tuple_keys/1`, `new_connection/1`, `no_aux_work_threads/0`, `notify_breakpoint_hit/3`, `open_port/2`, `port_call/3`, `port_close/1`, `port_command/3`, `port_connect/2`, `port_control/3`, `port_info/1`, `port_info/2`, `prepare_loading/2`, `process_display/2`, `process_flag/3`, `processes_next/1`, `purge_module/2`, `request_system_task/3`, `request_system_task/4`, `scheduler_wall_time/1`, `spawn_request/4`, `spawn_system_process/3`, `suspend_process/2`, `system_check/1`, `system_monitor/1`, `system_monitor/3`, `term_to_string/2`, `term_type/1`, `time_unit/0`, `trace/3`, `trace/4`, `trace_info/3`, `trace_pattern/3`, `trace_pattern/4`, `trace_session_create/3`, `trace_session_destroy/1`
- `erts_literal_area_collector`: `release_area_switch/0`, `send_copy_request/3`
- `erts_trace_cleaner`: `check/0`, `send_trace_clean_signal/1`
- `ets`: `delete/1`, `delete_object/2`, `first/1`, `first_lookup/1`, `give_away/3`, `info/1`, `info/2`, `insert_new/2`, `internal_delete_all/2`, `internal_request_all/0`, `internal_select_delete/2`, `is_compiled_ms/1`, `last/1`, `last_lookup/1`, `lookup_element/3`, `match/1`, `match/2`, `match/3`, `match_object/1`, `match_object/2`, `match_object/3`, `match_spec_compile/1`, `match_spec_run_r/3`, `next/2`, `next_lookup/2`, `prev/2`, `prev_lookup/2`, `rename/2`, `safe_fixtable/2`, `select/1`, `select/2`, `select/3`, `select_count/2`, `select_replace/2`, `select_reverse/1`, `select_reverse/2`, `select_reverse/3`, `setopts/2`, `slot/2`, `take/2`, `update_counter/3`, `update_counter/4`, `update_element/3`, `update_element/4`
- `file`: `native_name_encoding/0`
- `lists`: `keysearch/3`
- `maps`: `from_keys/2`, `get/2`, `is_key/2`, `put/3`, `take/2`, `update/3`, `values/1`
- `math`: `acos/1`, `acosh/1`, `asin/1`, `asinh/1`, `atan/1`, `atan2/2`, `atanh/1`, `ceil/1`, `cos/1`, `cosh/1`, `erf/1`, `erfc/1`, `exp/1`, `floor/1`, `fmod/2`, `log/1`, `log10/1`, `log2/1`, `pow/2`, `sin/1`, `sinh/1`, `sqrt/1`, `tan/1`, `tanh/1`
- `os`: `env/0`, `getpid/0`, `perf_counter/0`, `putenv/2`, `system_time/0`, `timestamp/0`, `unsetenv/1`
- `persistent_term`: `erase/1`, `info/0`, `put_new/2`
- `prim_file`: `internal_name2native/1`, `internal_native2name/1`, `internal_normalize_utf8/1`, `is_translatable/1`
- `re`: `compile/1`, `import/1`, `inspect/2`, `internal_run/4`, `run/2`, `run/3`, `version/0`
- `records`: `create/4`, `get/2`, `get_definition/2`, `get_field_names/1`, `get_module/1`, `get_name/1`, `is_exported/1`, `update/4`
- `string`: `list_to_float/1`
- `unicode`: `bin_is_7bit/1`

## Native bindings

Each row is a binding of the static NIF registry. The `nif_start` instruction runs the binding.

| Entry | Authority | Determinism | Execution | Effects | Cost | Owner |
| --- | --- | --- | --- | --- | --- | --- |
| `prim_tty:isatty/1` | `terminal.query`, `runtime.atom` | `host` | `host-call` | Calls `isatty` on fd 0, 1, or 2 and gives `true`, `false`, or `ebadf`. | `O(1)` | `src/runtime/interpreter/native/prim_tty.rs`, `src/runtime/host/terminal.rs` |
| `prim_tty:setlocale/1` | `host.locale`, `host.environment` | `host` | `host-call` | Sets the `LC_CTYPE` locale of the OS process from the environment and tells whether its codeset is UTF-8. | `unknown` | `src/runtime/interpreter/native/prim_tty.rs`, `src/runtime/host/terminal.rs` |
| `prim_tty:setupterm_nif/0` | `terminal.setup`, `host.environment`, `host.filesystem.read`, `runtime.atom` | `host` | `host-blocking` | Reads the terminfo entry of `TERM` into the curses library, or gives `{error, {setupterm, Errno}}`. | `unknown` | `src/runtime/interpreter/native/prim_tty.rs`, `src/runtime/host/terminal.rs` |
| `prim_tty:tty_create/1` | `terminal.query`, `runtime.resource` | `host` | `host-call` | Reads the mode of fd 1 or 2 with `tcgetattr` and makes a TTY record for it. | `O(1)` | `src/runtime/interpreter/native/prim_tty.rs`, `src/runtime/tty.rs` |
| `prim_tty:tty_encoding/1` | `none` | `pure` | `inline` | Gives `utf8`. | `O(1)` | `src/runtime/interpreter/native/prim_tty.rs` |
| `prim_tty:tty_init/2` | `terminal.raw`, `runtime.resource`, `runtime.atom` | `host` | `host-call` | Sets the output fd to the first mode or to its raw form with `tcsetattr`. Then the caller becomes the owner of the TTY record, and the record monitors the caller. At the owner exit, exit cleanup sets fd 0 to the first mode. | `O(n)` | `src/runtime/interpreter/native/prim_tty.rs`, `src/runtime/tty.rs` |
| `prim_tty:tty_select/2` | `terminal.input`, `runtime.resource` | `run` | `inline` | The TTY record monitors the caller. PBeam selects no fd for read. | `O(1)` | `src/runtime/interpreter/native/prim_tty.rs`, `src/runtime/tty.rs` |
| `prim_tty:tty_window_size/1` | `terminal.query`, `runtime.atom` | `host` | `host-call` | Reads the window size from fd 0, or else from the output fd. | `O(1)` | `src/runtime/interpreter/native/prim_tty.rs`, `src/runtime/host/terminal.rs` |
| `prim_tty:write_nif/2` | `terminal.text_write`, `terminal.tui_control`, `runtime.atom` | `host` | `host-blocking` | Writes the binaries of an iovec to the output fd in groups of `MAXIOV` elements. PBeam does not filter control sequences. A failed write gives `{error, Errno}`. ERTS runs this NIF on a dirty IO scheduler. | `O(n)` | `src/runtime/interpreter/native/prim_tty.rs`, `src/runtime/host/output.rs` |

## Native functions without a binding

Each entry is a function of a static NIF library that has no PBeam binding. The `nif_start` instruction runs the BEAM stub body, which calls `erlang:nif_error(undef)` and raises `error:undef`. These entries have no effect. Authority: `none`. Determinism: `pure`. Execution: `inline`. Cost: `O(1)`. Owner: `src/runtime/interpreter/native/prim_tty.rs`.

- `prim_tty`: `tty_is_open/2`, `read_nif/3`, `isprint/1`, `wcwidth/1`, `wcswidth/1`, `sizeof_wchar/0`, `tigetnum_nif/1`, `tigetflag_nif/1`, `tinfo_nif/0`, `tigetstr_nif/1`, `tputs_nif/2`

## Linked native code and dependencies

All in-process native code is part of the TCB. A classification above does not limit what this code can do.

| Component | Use | Linked by | Notes |
| --- | --- | --- | --- |
| `libc` (libSystem on macOS, glibc on Linux) | Each host operation, and the PCRE2 allocation | `Cargo.toml` (`libc` crate) | The Rust and C code calls it directly. |
| Curses library | `setupterm` and `del_curterm` | `src/runtime/host/terminal.rs` (`libncurses` on macOS, `libtinfo.so.6` on Linux) | The dynamic loader selects the file at startup. |
| PCRE2 10.47 | `re:compile/2` | `build.rs` (`vendor/pcre2`, static) | C code from OTP 29.1.1. It runs on the scheduler thread with the stack guard of `src/runtime/pcre2.rs`. |
| Rust standard library | Startup, file read, clock, sleep, and stderr output | The Rust toolchain of `mise.toml` | Startup sets `SIGPIPE` and opens `/dev/null` for a closed standard fd. |
| `clap` | Command line parse at startup | `Cargo.toml` | Its derive procedural macros run at build time. |
| `cc` | Compile of PCRE2 | `build.rs` | `build` class: it runs at build time only. |

## Unknown entries

This inventory cannot classify the entries in this section. Each unknown entry prevents secure-profile acceptance until a review classifies it.

| Entry | Unknown part | Reason |
| --- | --- | --- |
| `host::terminal::setupterm` and `prim_tty:setupterm_nif/0` | Cost, and the complete set of environment variables and files that the curses library reads | The linked curses build selects the terminfo search path. This inventory did not review the curses source. |
| `host::terminal::setlocale_utf8` and `prim_tty:setlocale/1` | Cost, and the locale files and environment variables that the C library reads | The C library selects the locale data. This inventory did not review the C library source. |
| `host::clock::local_time` and `erlang:universaltime_to_localtime/1` | Cost, and the time zone files and environment variables that the C library reads | `tzset`, `localtime_r`, and `posix2time` read the time zone data of the C library. This inventory did not review the C library source. |
| `re:compile/2` | Cost: compile time and memory | PCRE2 has no reviewed bound other than the stack guard. Compiled code stays in `RegexCodes` until the run ends. |
