//! BIFs: the ERTS `bif.tab` table, the BIF dispatch, and BIF results.

use super::*;

/// What a BIF call does to the calling process.
#[derive(Debug)]
pub(super) enum BifResult {
    /// The BIF returns this value.
    Value(Term),
    /// The BIF raises an exception in the calling process.
    Raise {
        /// Class of the exception.
        class: ExceptionClass,
        /// Reason of the exception, without a stack trace.
        reason: Term,
    },
    /// The BIF continues in BEAM code, as `BIF_TRAP` does in ERTS: the call
    /// becomes a call to this exported function, and its return value is the
    /// value of the BIF call.
    Trap {
        /// Module of the trap target, for example `erts_internal`.
        module: &'static str,
        /// Exported function of the trap target.
        function: &'static str,
        /// Arguments for the trap target, in `x0` onwards.
        arguments: Vec<Term>,
    },
}

impl BifResult {
    /// Return the value of a BIF called through a `bif` instruction with
    /// fail label 0. An exception or a trap from such a BIF is not supported.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let value = BifResult::Value(Term::Integer(1)).into_bif_value("erlang", "self", 0)?;
    /// assert_eq!(value, Term::Integer(1));
    /// let raise = BifResult::Raise { class: ExceptionClass::Exit, reason: Term::static_atom("badarg") };
    /// assert!(raise.into_bif_value("erlang", "node", 1).is_err());
    /// ```
    pub(super) fn into_bif_value(
        self,
        module: &str,
        name: &str,
        arity: usize,
    ) -> Result<Term, BeamError> {
        match self {
            BifResult::Value(value) => Ok(value),
            // ponytail: no BIF that a `bif` instruction reaches raises or traps; add it when one does.
            other => Err(error(format!(
                "unsupported {module}:{name}/{arity} result {other:?}"
            ))),
        }
    }
}

impl BeamProgram {
    /// Run a BIF that the observed process path calls. The result
    /// is a value, an exception, or a trap into BEAM code.
    ///
    /// BIFs are native functions listed in the ERTS `bif.tab`, mostly in the
    /// `erlang` module. The compiler emits guard BIFs such as `self/0` as
    /// `bif` instructions and other BIFs as `call_ext` instructions. Both
    /// forms come here.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let value = program.bif(&mut scheduler, "erlang", "make_ref", vec![])?;
    /// assert!(matches!(value, BifResult::Value(Term::Reference(_))));
    /// ```
    pub(super) fn bif(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        name: &str,
        arguments: Vec<Term>,
    ) -> Result<BifResult, BeamError> {
        let value = match (module, name, arguments.as_slice()) {
            ("erlang", "process_flag", [Term::Atom(flag), Term::Atom(value)])
                if flag.name() == "trap_exit"
                    && (value.name() == "true" || value.name() == "false") =>
            {
                let process = scheduler.current_process_mut();
                let old = Term::static_atom(&process.trap_exit.to_string());
                process.trap_exit = value.name() == "true";
                old
            }
            ("erlang", "self", []) => Term::Pid(scheduler.current_process_mut().id),
            // PBeam is a local node that is not distributed, so every pid is
            // local and its node is `nonode@nohost`, as on ERTS (`dist.c`).
            ("erlang", "node", [] | [Term::Pid(_)]) => Term::static_atom("nonode@nohost"),
            ("erlang", "node", [argument]) => {
                return Err(error(format!(
                    "unsupported erlang:node/1 argument {argument:?}: not a pid"
                )));
            }
            ("erlang", "make_ref", []) => Term::Reference(scheduler.make_ref()?),
            ("erlang", "link", [Term::Pid(target)]) => scheduler.link(*target)?,
            ("erlang", "group_leader", []) => {
                Term::Pid(scheduler.current_process_mut().group_leader)
            }
            ("erlang", "group_leader", [Term::Pid(leader), Term::Pid(target)]) => {
                scheduler.set_group_leader(*target, *leader)?;
                Term::static_atom("true")
            }
            ("erlang", "monitor", [Term::Atom(kind), Term::Pid(target)])
                if kind.name() == "process" =>
            {
                Term::Reference(scheduler.monitor(*target)?)
            }
            ("erlang", "demonitor", [Term::Reference(reference), Term::List(options)]) => {
                let flush = match options.as_slice() {
                    [] => false,
                    [Term::Atom(option)] if option.name() == "flush" => true,
                    _ => return Err(error(format!("unsupported demonitor options {options:?}"))),
                };
                // ERTS `demonitor_2` (bif.c) flushes only when no active
                // monitor was removed: the DOWN message can then be queued.
                let removed = scheduler.demonitor(*reference);
                if removed || !flush {
                    Term::static_atom("true")
                } else {
                    let arguments = vec![
                        Term::Reference(*reference),
                        Term::static_atom("false"),
                        Term::static_atom("true"),
                    ];
                    let module = "erts_internal";
                    let function = "flush_monitor_messages";
                    return Ok(BifResult::Trap {
                        module,
                        function,
                        arguments,
                    });
                }
            }
            ("erlang", "exit", [reason]) => {
                return Ok(BifResult::Raise {
                    class: ExceptionClass::Exit,
                    reason: reason.clone(),
                });
            }
            ("erlang", "nif_error", [reason]) => {
                return Ok(BifResult::Raise {
                    class: ExceptionClass::Error,
                    reason: reason.clone(),
                });
            }
            (
                "erlang",
                "spawn" | "spawn_monitor" | "spawn_link",
                [Term::Atom(spawn_module), Term::Atom(function), list],
            ) => {
                let Term::List(arguments) = list else {
                    return Err(error("spawn arguments are not a proper list"));
                };
                let arguments = arguments.clone();
                let arity = u32::try_from(arguments.len())
                    .map_err(|_| error("arity exceeds BEAM limit"))?;
                let frame = self.mfa_frame(spawn_module.name(), function.name(), arity)?;
                let monitor = name == "spawn_monitor";
                let link = name == "spawn_link";
                let (pid, reference) = scheduler.spawn(frame, arguments, monitor, link)?;
                match reference {
                    Some(reference) => {
                        Term::Tuple(vec![Term::Pid(pid), Term::Reference(reference)])
                    }
                    None => Term::Pid(pid),
                }
            }
            ("maps", "keys", [Term::Map(map)]) => {
                Term::List(map.iter().map(|(key, _)| key.clone()).collect())
            }
            // ponytail: ERTS raises error {badmap, Map}; see "Exception raise sites" in docs/erts-differences.md.
            ("maps", "keys", [_]) => {
                return Err(error("unsupported maps:keys/1 argument: not a map"));
            }
            ("unicode", "characters_to_binary", [chardata, Term::Atom(encoding)])
                if encoding.name() == "unicode" =>
            {
                Term::Binary(characters_to_utf8(chardata)?)
            }
            // ponytail: ERTS also converts `utf8`, `latin1`, and other encodings; see "Chardata conversion" in docs/erts-differences.md.
            ("unicode", "characters_to_binary", [_, encoding]) => {
                return Err(error(format!(
                    "unsupported unicode:characters_to_binary/2 encoding {encoding:?}"
                )));
            }
            _ => {
                return Err(error(format!(
                    "unresolved external MFA {module}:{name}/{}",
                    arguments.len()
                )));
            }
        };
        Ok(BifResult::Value(value))
    }
}

/// The BIFs in modules other than `erlang`, as `(module, name, arity)`,
/// sorted for a binary search. The table holds each non-`erlang` entry of the
/// ERTS 29.1.1 `bif.tab`, and `erlang:is_builtin/3` returns `true` for each
/// entry on the pinned ERTS. ERTS binds each import of a BIF to the BIF and
/// replaces the body of each exported BIF stub with a call to the BIF
/// (`emu_load.c`), so a stub body such as `erlang:nif_error(undef)` never runs
/// for an external call. PBeam implements only some entries in
/// [`BeamProgram::bif`]. Each other entry gives an unresolved MFA error.
pub(super) const BIFS: &[(&str, &str, u32)] = &[
    ("atomics", "add", 3),
    ("atomics", "add_get", 3),
    ("atomics", "compare_exchange", 4),
    ("atomics", "exchange", 3),
    ("atomics", "get", 2),
    ("atomics", "info", 1),
    ("atomics", "put", 3),
    ("binary", "at", 2),
    ("binary", "compile_pattern", 1),
    ("binary", "copy", 1),
    ("binary", "copy", 2),
    ("binary", "decode_unsigned", 1),
    ("binary", "decode_unsigned", 2),
    ("binary", "encode_unsigned", 1),
    ("binary", "encode_unsigned", 2),
    ("binary", "first", 1),
    ("binary", "last", 1),
    ("binary", "list_to_bin", 1),
    ("binary", "longest_common_prefix", 1),
    ("binary", "longest_common_suffix", 1),
    ("binary", "match", 2),
    ("binary", "match", 3),
    ("binary", "matches", 2),
    ("binary", "matches", 3),
    ("binary", "part", 2),
    ("binary", "part", 3),
    ("binary", "referenced_byte_size", 1),
    ("binary", "split", 2),
    ("binary", "split", 3),
    ("code", "coverage_support", 0),
    ("code", "get_coverage", 2),
    ("code", "get_coverage_mode", 0),
    ("code", "get_coverage_mode", 1),
    ("code", "get_debug_info", 1),
    ("code", "reset_coverage", 1),
    ("code", "set_coverage_mode", 1),
    ("erl_ddll", "demonitor", 1),
    ("erl_ddll", "format_error_int", 1),
    ("erl_ddll", "info", 2),
    ("erl_ddll", "loaded_drivers", 0),
    ("erl_ddll", "monitor", 2),
    ("erl_ddll", "try_load", 3),
    ("erl_ddll", "try_unload", 2),
    ("erl_debugger", "breakpoint", 3),
    ("erl_debugger", "breakpoints", 1),
    ("erl_debugger", "breakpoints", 3),
    ("erl_debugger", "instrumentations", 0),
    ("erl_debugger", "peek_stack_frame_slot", 4),
    ("erl_debugger", "peek_xreg", 3),
    ("erl_debugger", "register", 1),
    ("erl_debugger", "stack_frames", 2),
    ("erl_debugger", "supported", 0),
    ("erl_debugger", "toggle_instrumentations", 1),
    ("erl_debugger", "unregister", 2),
    ("erl_debugger", "whereis", 0),
    ("erl_debugger", "xregs_count", 1),
    ("error_logger", "warning_map", 0),
    ("erts_debug", "breakpoint", 2),
    ("erts_debug", "copy_shared", 2),
    ("erts_debug", "dirty", 3),
    ("erts_debug", "dirty_cpu", 2),
    ("erts_debug", "dirty_io", 2),
    ("erts_debug", "disassemble", 1),
    ("erts_debug", "dist_ext_to_term", 2),
    ("erts_debug", "flat_size", 1),
    ("erts_debug", "get_internal_state", 1),
    ("erts_debug", "instructions", 0),
    ("erts_debug", "interpreter_size", 0),
    ("erts_debug", "lcnt_clear", 0),
    ("erts_debug", "lcnt_collect", 0),
    ("erts_debug", "lcnt_control", 1),
    ("erts_debug", "lcnt_control", 2),
    ("erts_debug", "map_info", 1),
    ("erts_debug", "same", 2),
    ("erts_debug", "set_internal_state", 2),
    ("erts_debug", "size_shared", 1),
    ("erts_debug", "unaligned_bitstring", 2),
    ("erts_internal", "abort_pending_connection", 2),
    ("erts_internal", "atomics_new", 2),
    ("erts_internal", "beamfile_chunk", 2),
    ("erts_internal", "beamfile_module_md5", 1),
    ("erts_internal", "binary_to_integer", 2),
    ("erts_internal", "check_dirty_process_code", 2),
    ("erts_internal", "check_process_code", 1),
    ("erts_internal", "cmp_term", 2),
    ("erts_internal", "counters_add", 3),
    ("erts_internal", "counters_get", 2),
    ("erts_internal", "counters_info", 1),
    ("erts_internal", "counters_new", 1),
    ("erts_internal", "counters_put", 3),
    ("erts_internal", "create_dist_channel", 3),
    ("erts_internal", "dirty_process_handle_signals", 1),
    ("erts_internal", "dist_spawn_request", 4),
    ("erts_internal", "erase_persistent_terms", 0),
    ("erts_internal", "ets_lookup_binary_info", 2),
    ("erts_internal", "ets_raw_first", 1),
    ("erts_internal", "ets_raw_next", 2),
    ("erts_internal", "ets_super_user", 1),
    ("erts_internal", "garbage_collect", 1),
    ("erts_internal", "gather_alloc_histograms", 1),
    ("erts_internal", "gather_carrier_info", 1),
    ("erts_internal", "get_creation", 0),
    ("erts_internal", "get_dflags", 0),
    ("erts_internal", "group_leader", 2),
    ("erts_internal", "group_leader", 3),
    ("erts_internal", "is_process_alive", 2),
    ("erts_internal", "is_process_executing_dirty", 1),
    ("erts_internal", "is_system_process", 1),
    ("erts_internal", "list_to_integer", 2),
    ("erts_internal", "map_hashmap_children", 1),
    ("erts_internal", "map_next", 3),
    ("erts_internal", "map_to_tuple_keys", 1),
    ("erts_internal", "new_connection", 1),
    ("erts_internal", "no_aux_work_threads", 0),
    ("erts_internal", "notify_breakpoint_hit", 3),
    ("erts_internal", "open_port", 2),
    ("erts_internal", "perf_counter_unit", 0),
    ("erts_internal", "port_call", 3),
    ("erts_internal", "port_close", 1),
    ("erts_internal", "port_command", 3),
    ("erts_internal", "port_connect", 2),
    ("erts_internal", "port_control", 3),
    ("erts_internal", "port_info", 1),
    ("erts_internal", "port_info", 2),
    ("erts_internal", "prepare_loading", 2),
    ("erts_internal", "process_display", 2),
    ("erts_internal", "process_flag", 3),
    ("erts_internal", "processes_next", 1),
    ("erts_internal", "purge_module", 2),
    ("erts_internal", "request_system_task", 3),
    ("erts_internal", "request_system_task", 4),
    ("erts_internal", "scheduler_wall_time", 1),
    ("erts_internal", "spawn_request", 4),
    ("erts_internal", "spawn_system_process", 3),
    ("erts_internal", "suspend_process", 2),
    ("erts_internal", "system_check", 1),
    ("erts_internal", "system_monitor", 1),
    ("erts_internal", "system_monitor", 3),
    ("erts_internal", "term_to_string", 2),
    ("erts_internal", "term_type", 1),
    ("erts_internal", "time_unit", 0),
    ("erts_internal", "trace", 3),
    ("erts_internal", "trace", 4),
    ("erts_internal", "trace_info", 3),
    ("erts_internal", "trace_pattern", 3),
    ("erts_internal", "trace_pattern", 4),
    ("erts_internal", "trace_session_create", 3),
    ("erts_internal", "trace_session_destroy", 1),
    ("erts_literal_area_collector", "release_area_switch", 0),
    ("erts_literal_area_collector", "send_copy_request", 3),
    ("erts_trace_cleaner", "check", 0),
    ("erts_trace_cleaner", "send_trace_clean_signal", 1),
    ("ets", "delete", 1),
    ("ets", "delete", 2),
    ("ets", "delete_object", 2),
    ("ets", "first", 1),
    ("ets", "first_lookup", 1),
    ("ets", "give_away", 3),
    ("ets", "info", 1),
    ("ets", "info", 2),
    ("ets", "insert", 2),
    ("ets", "insert_new", 2),
    ("ets", "internal_delete_all", 2),
    ("ets", "internal_request_all", 0),
    ("ets", "internal_select_delete", 2),
    ("ets", "is_compiled_ms", 1),
    ("ets", "last", 1),
    ("ets", "last_lookup", 1),
    ("ets", "lookup", 2),
    ("ets", "lookup_element", 3),
    ("ets", "lookup_element", 4),
    ("ets", "match", 1),
    ("ets", "match", 2),
    ("ets", "match", 3),
    ("ets", "match_object", 1),
    ("ets", "match_object", 2),
    ("ets", "match_object", 3),
    ("ets", "match_spec_compile", 1),
    ("ets", "match_spec_run_r", 3),
    ("ets", "member", 2),
    ("ets", "new", 2),
    ("ets", "next", 2),
    ("ets", "next_lookup", 2),
    ("ets", "prev", 2),
    ("ets", "prev_lookup", 2),
    ("ets", "rename", 2),
    ("ets", "safe_fixtable", 2),
    ("ets", "select", 1),
    ("ets", "select", 2),
    ("ets", "select", 3),
    ("ets", "select_count", 2),
    ("ets", "select_replace", 2),
    ("ets", "select_reverse", 1),
    ("ets", "select_reverse", 2),
    ("ets", "select_reverse", 3),
    ("ets", "setopts", 2),
    ("ets", "slot", 2),
    ("ets", "take", 2),
    ("ets", "update_counter", 3),
    ("ets", "update_counter", 4),
    ("ets", "update_element", 3),
    ("ets", "update_element", 4),
    ("ets", "whereis", 1),
    ("file", "native_name_encoding", 0),
    ("io", "printable_range", 0),
    ("lists", "keyfind", 3),
    ("lists", "keymember", 3),
    ("lists", "keysearch", 3),
    ("lists", "member", 2),
    ("lists", "reverse", 2),
    ("maps", "find", 2),
    ("maps", "from_keys", 2),
    ("maps", "from_list", 1),
    ("maps", "get", 2),
    ("maps", "is_key", 2),
    ("maps", "keys", 1),
    ("maps", "merge", 2),
    ("maps", "put", 3),
    ("maps", "remove", 2),
    ("maps", "take", 2),
    ("maps", "update", 3),
    ("maps", "values", 1),
    ("math", "acos", 1),
    ("math", "acosh", 1),
    ("math", "asin", 1),
    ("math", "asinh", 1),
    ("math", "atan", 1),
    ("math", "atan2", 2),
    ("math", "atanh", 1),
    ("math", "ceil", 1),
    ("math", "cos", 1),
    ("math", "cosh", 1),
    ("math", "erf", 1),
    ("math", "erfc", 1),
    ("math", "exp", 1),
    ("math", "floor", 1),
    ("math", "fmod", 2),
    ("math", "log", 1),
    ("math", "log10", 1),
    ("math", "log2", 1),
    ("math", "pow", 2),
    ("math", "sin", 1),
    ("math", "sinh", 1),
    ("math", "sqrt", 1),
    ("math", "tan", 1),
    ("math", "tanh", 1),
    ("net_kernel", "dflag_unicode_io", 1),
    ("os", "env", 0),
    ("os", "getenv", 1),
    ("os", "getpid", 0),
    ("os", "perf_counter", 0),
    ("os", "putenv", 2),
    ("os", "set_signal", 2),
    ("os", "system_time", 0),
    ("os", "system_time", 1),
    ("os", "timestamp", 0),
    ("os", "unsetenv", 1),
    ("persistent_term", "erase", 1),
    ("persistent_term", "get", 0),
    ("persistent_term", "get", 1),
    ("persistent_term", "get", 2),
    ("persistent_term", "info", 0),
    ("persistent_term", "put", 2),
    ("persistent_term", "put_new", 2),
    ("prim_file", "internal_name2native", 1),
    ("prim_file", "internal_native2name", 1),
    ("prim_file", "internal_normalize_utf8", 1),
    ("prim_file", "is_translatable", 1),
    ("re", "compile", 1),
    ("re", "compile", 2),
    ("re", "import", 1),
    ("re", "inspect", 2),
    ("re", "internal_run", 4),
    ("re", "run", 2),
    ("re", "run", 3),
    ("re", "version", 0),
    ("records", "create", 4),
    ("records", "get", 2),
    ("records", "get_definition", 2),
    ("records", "get_field_names", 1),
    ("records", "get_module", 1),
    ("records", "get_name", 1),
    ("records", "is_exported", 1),
    ("records", "update", 4),
    ("string", "list_to_float", 1),
    ("unicode", "bin_is_7bit", 1),
    ("unicode", "characters_to_binary", 2),
    ("unicode", "characters_to_list", 2),
];

/// Return whether an external call or a `bif` instruction goes to a BIF. An
/// entry of `BIFS` runs before the module code, as in ERTS. Every `erlang`
/// call goes to the BIF dispatch, which reports an unresolved MFA.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_bif("erlang", "self", 0));
/// assert!(is_bif("maps", "keys", 1));
/// assert!(is_bif("lists", "keyfind", 3));
/// assert!(!is_bif("maps", "keys", 2));
/// assert!(!is_bif("io", "put_chars", 2));
/// ```
pub(super) fn is_bif(module: &str, name: &str, arity: u32) -> bool {
    module == "erlang" || BIFS.binary_search(&(module, name, arity)).is_ok()
}
