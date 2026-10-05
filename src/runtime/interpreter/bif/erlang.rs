//! Bodies of the `erlang` BIFs that do not fit on one table line.

use super::BifResult::{Raise, Trap, Value};
use super::*;
use crate::runtime::host;
use crate::runtime::scheduler::{AliasMode, MonitorOptions, MonitorTarget};
use crate::runtime::timer::{Receiver, Timer, Timers};
use crate::term::Fun;

/// Return the node of a pid or of the calling process.
///
/// PBeam is a local node that is not distributed, so every pid is local and
/// its node is `nonode@nohost`, as on ERTS (`dist.c`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(local_node(), Term::static_atom("nonode@nohost"));
/// ```
pub(super) fn local_node() -> Term {
    Term::static_atom("nonode@nohost")
}

/// Run `erlang:node/1`: return the node of a pid. A term that is not a pid
/// gives a runtime error.
///
/// # Examples
///
/// ```rust,ignore
/// let result = node(&Term::Pid(pid))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "nonode@nohost"));
/// assert!(node(&Term::Integer(1)).is_err());
/// ```
pub(super) fn node(term: &Term) -> Result<BifResult, BeamError> {
    match term {
        Term::Pid(_) => Ok(Value(local_node())),
        _ => Err(error(format!(
            "unsupported erlang:node/1 argument {term:?}: not a pid"
        ))),
    }
}

/// Run `erlang:process_flag(trap_exit, Value)`: set the `trap_exit` flag of
/// the calling process and return the old flag. The table pattern accepts
/// only `true` and `false` as the value.
///
/// # Examples
///
/// ```rust,ignore
/// let old = trap_exit(&mut context, &true_atom)?;
/// assert!(matches!(old, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// ```
pub(super) fn trap_exit(
    context: &mut BifContext<'_>,
    value: &Atom,
) -> Result<BifResult, BeamError> {
    let process = context.scheduler.current_process_mut();
    let old = Term::static_atom(&process.trap_exit.to_string());
    process.trap_exit = value.name() == "true";
    Ok(Value(old))
}

/// Run `erlang:exit(Id, Reason)`: send an exit signal to a local pid, as
/// ERTS `send_exit_signal_bif` (`bif.c:1750-1772`) does, and return `true`.
/// A signal that ends the caller gives [`BifResult::Exited`] instead.
///
/// A reference sends the signal to the owner of its active alias, with the
/// caller as the sender (`erl_proc_sig_queue.c:1650-1658`). A reference that
/// is not an active alias gives `true` and sends nothing: the owner ignores
/// the signal (`erl_proc_sig_queue.c:4659-4672`). A reference is never
/// `exit2_suicide`, so `normal` through the alias of the caller is ignored
/// without `trap_exit`. The reference of a NIF resource also gives `true`:
/// in ERTS it is a magic reference, and `is_internal_ref` is true for it
/// (`erl_term.h:1064-1065`).
///
/// Each other term raises `error:badarg` (`bif.c:1864-1866`). PBeam cannot
/// represent a port or a remote pid.
///
/// # Examples
///
/// ```rust,ignore
/// let result = exit(&mut context, &Term::Reference(reference), &boom)?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let result = exit(&mut context, &Term::Pid(caller), &kill)?;
/// assert!(matches!(result, BifResult::Exited(Term::Atom(atom)) if atom.name() == "killed"));
/// assert!(matches!(exit(&mut context, &foo, &boom)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// An abnormal exit of the receiver to a linked process that does not trap
/// exits is not supported. An exit signal through an alias of the caller
/// that ends the caller is not supported: ERTS ends the caller only when
/// it next handles signals, after `exit/2` returns `true`.
pub(super) fn exit(
    context: &mut BifContext<'_>,
    id: &Term,
    reason: &Term,
) -> Result<BifResult, BeamError> {
    let target = match id {
        Term::Pid(target) => *target,
        Term::Reference(reference) => {
            let Some(owner) = context.scheduler.alias_owner(*reference) else {
                return Ok(Value(Term::static_atom("true")));
            };
            let caller = context.scheduler.current_process_mut();
            let trapped = caller.trap_exit && *reason != Term::static_atom("kill");
            if owner == caller.id && !trapped {
                if !caller.trap_exit && *reason == Term::static_atom("normal") {
                    return Ok(Value(Term::static_atom("true")));
                }
                // ponytail: no signal queue yet (#10); ERTS ends the caller at its next signal handling.
                return Err(error(
                    "unsupported exit signal through an alias of the caller",
                ));
            }
            owner
        }
        _ => return Ok(BifResult::badarg()),
    };
    Ok(match context.scheduler.send_exit(target, reason.clone())? {
        Some(reason) => BifResult::Exited(reason),
        None => Value(Term::static_atom("true")),
    })
}

/// Run `erlang:demonitor(Reference, Options)` as ERTS `demonitor_2`
/// (`bif.c:613-654`) does. The options are a proper list of `flush` and
/// `info`; each other element, and a tail that is not `[]`, raise `badarg`
/// before the monitor is removed. After the options, a `Reference` that is
/// not a reference raises `badarg`. The result is `true`, or `false` with
/// `info` when no active monitor was removed.
///
/// With `flush`, ERTS flushes only when no active monitor was removed: the
/// DOWN message can then be queued. The flush traps into
/// `erts_internal:flush_monitor_messages/3`, which returns its third
/// argument, the result.
///
/// # Examples
///
/// ```rust,ignore
/// let result = demonitor(&mut context, &reference, &Term::List(vec![]));
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let result = demonitor(&mut context, &gone, &Term::List(vec![Term::static_atom("info")]));
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// let result = demonitor(&mut context, &gone, &Term::List(vec![Term::static_atom("flush")]));
/// assert!(matches!(result, BifResult::Trap { function: "flush_monitor_messages", .. }));
/// assert!(matches!(demonitor(&mut context, &gone, &Term::static_atom("x")), BifResult::Raise { .. }));
/// assert!(matches!(demonitor(&mut context, &Term::static_atom("x"), &Term::List(vec![])), BifResult::Raise { .. }));
/// ```
pub(super) fn demonitor(
    context: &mut BifContext<'_>,
    reference: &Term,
    options: &Term,
) -> BifResult {
    let Term::List(options) = options else {
        return BifResult::badarg();
    };
    let (mut flush, mut info) = (false, false);
    for option in options {
        match option {
            Term::Atom(option) if option.name() == "flush" => flush = true,
            Term::Atom(option) if option.name() == "info" => info = true,
            _ => return BifResult::badarg(),
        }
    }
    let Term::Reference(reference) = *reference else {
        return BifResult::badarg();
    };
    let removed = context.scheduler.demonitor(reference);
    let result = Term::static_atom(if removed || !info { "true" } else { "false" });
    if removed || !flush {
        return Value(result);
    }
    Trap {
        module: "erts_internal",
        function: "flush_monitor_messages",
        arguments: vec![
            Term::Reference(reference),
            Term::static_atom("false"),
            result,
        ],
    }
}

/// Run `erlang:monitor(Type, Target, Options)` as ERTS `monitor_3`
/// (`bif.c:988-997`) and `monitor` (`bif.c:758-981`) do;
/// `erlang:monitor/2` passes `[]` as `monitor_2` (`bif.c:983-986`) does.
/// The options come first (see [`monitor_options`]). The type `process`
/// accepts a pid, a registered name, and `{Name, nonode@nohost}`; the type
/// `port` accepts a registered name and `{Name, nonode@nohost}`. PBeam is a
/// node that is not alive, so `{Name, Node}` with another node raises
/// `badarg` (`bif.c:881-882`). Each other target, and each other type,
/// raises `badarg`. A monitor of the caller creates no monitor and no
/// alias, and each port name gives the `noproc` `DOWN` message (see
/// [`Scheduler::monitor`]).
///
/// # Examples
///
/// ```rust,ignore
/// let result = monitor(&mut context, &process, &Term::Pid(child), &Term::List(vec![]))?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// assert!(matches!(monitor(&mut context, &port, &Term::Pid(child), &Term::List(vec![]))?, BifResult::Raise { .. }));
/// assert!(matches!(monitor(&mut context, &process, &Term::Pid(child), &x)?, BifResult::Raise { .. }));
/// assert!(monitor(&mut context, &time_offset, &clock_service, &Term::List(vec![])).is_err());
/// ```
///
/// # Errors
///
/// A `time_offset` monitor of `clock_service`, and the `priority` option
/// of a monitor that is not of the caller, are not supported.
pub(super) fn monitor(
    context: &mut BifContext<'_>,
    kind: &Term,
    target: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Some(options) = monitor_options(options) else {
        return Ok(BifResult::badarg());
    };
    let Term::Atom(kind) = kind else {
        return Ok(BifResult::badarg());
    };
    let target = match (kind.name(), target) {
        ("process", Term::Pid(id)) => Some(MonitorTarget::Process(*id)),
        ("process", target) => local_name(target).map(MonitorTarget::ProcessName),
        ("port", target) => local_name(target).map(MonitorTarget::PortName),
        ("time_offset", Term::Atom(service)) if service.name() == "clock_service" => {
            // ponytail: no time offset service yet; add it when a measured path monitors it.
            return Err(error("unsupported time_offset monitor"));
        }
        _ => None,
    };
    let Some(target) = target else {
        return Ok(BifResult::badarg());
    };
    let reference = context.scheduler.monitor(target, options)?;
    Ok(Value(Term::Reference(reference)))
}

/// Read the options of `erlang:monitor/3` as ERTS `erts_monitor_opts`
/// (`bif.c:701-756`) does. The options are a proper list of `priority`,
/// `{alias, explicit_unalias | demonitor | reply_demonitor}`, and
/// `{tag, Tag}`; the last `alias` and the last `tag` option win. Return
/// `None` for each other element and for a tail that is not `[]`, which
/// `monitor_3` rejects with `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let alias = Term::Tuple(vec![Term::static_atom("alias"), Term::static_atom("demonitor")]);
/// let options = monitor_options(&Term::List(vec![alias])).unwrap();
/// assert_eq!(options.alias, Some(AliasMode::Demonitor));
/// assert!(monitor_options(&Term::static_atom("x")).is_none());
/// ```
fn monitor_options(options: &Term) -> Option<MonitorOptions> {
    let Term::List(options) = options else {
        return None;
    };
    let mut result = MonitorOptions {
        alias: None,
        tag: Term::static_atom("DOWN"),
        priority: false,
    };
    for option in options {
        match option {
            Term::Atom(option) if option.name() == "priority" => result.priority = true,
            Term::Tuple(pair) => match pair.as_slice() {
                [Term::Atom(key), Term::Atom(mode)] if key.name() == "alias" => {
                    result.alias = Some(match mode.name() {
                        "explicit_unalias" => AliasMode::ExplicitUnalias,
                        "demonitor" => AliasMode::Demonitor,
                        "reply_demonitor" => AliasMode::ReplyDemonitor,
                        _ => return None,
                    });
                }
                [Term::Atom(key), tag] if key.name() == "tag" => result.tag = tag.clone(),
                _ => return None,
            },
            _ => return None,
        }
    }
    Some(result)
}

/// Return the name of a local monitor target by name: an atom, or
/// `{Name, nonode@nohost}` with an atom name. Return `None` for each other
/// term, which `monitor` (`bif.c:875-896`, `938-950`) rejects with `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(local_name(&Term::Tuple(vec![name.clone(), local_node()])), Some(atom));
/// assert_eq!(local_name(&Term::Integer(1)), None);
/// ```
fn local_name(target: &Term) -> Option<Atom> {
    match target {
        Term::Atom(name) => Some(name.clone()),
        Term::Tuple(pair) => match pair.as_slice() {
            [Term::Atom(name), node] if *node == local_node() => Some(name.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Run `erlang:spawn/3`: start a process that runs `module:function` with the
/// elements of `arguments`, as ERTS `spawn_3` does with default spawn options.
/// A `module` or `function` that is not an atom, and `arguments` that are not
/// a proper list, raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = spawn(&mut context, &module, &function, &Term::List(vec![]))?;
/// assert!(matches!(result, BifResult::Value(Term::Pid(_))));
/// ```
pub(super) fn spawn(
    context: &mut BifContext<'_>,
    module: &Term,
    function: &Term,
    arguments: &Term,
) -> Result<BifResult, BeamError> {
    create_process(
        context.program,
        context.scheduler,
        module,
        function,
        arguments,
        SpawnOptions::default(),
    )
}

/// Run `erlang:spawn_link/3`: start a linked process, as ERTS `spawn_link_3`
/// does by adding `SPO_LINK` to the default spawn options.
///
/// # Examples
///
/// ```rust,ignore
/// let result = spawn_link(&mut context, &module, &function, &Term::List(vec![]))?;
/// assert!(matches!(result, BifResult::Value(Term::Pid(_))));
/// ```
pub(super) fn spawn_link(
    context: &mut BifContext<'_>,
    module: &Term,
    function: &Term,
    arguments: &Term,
) -> Result<BifResult, BeamError> {
    let options = SpawnOptions {
        link: true,
        ..SpawnOptions::default()
    };
    create_process(
        context.program,
        context.scheduler,
        module,
        function,
        arguments,
        options,
    )
}

/// Run `erlang:spawn_opt/4`, which `erlang:spawn_monitor/3` in `erlang.beam`
/// calls with the options `[monitor]`: start a process with the spawn
/// options in `options`, as ERTS `spawn_opt_4` (`bif.c`) does. The option
/// `link` links the new process and `monitor` monitors it. With `monitor`
/// the result is `{Pid, Reference}`, else `Pid`. `options` that are not a
/// proper list raise `badarg`, as `erts_parse_spawn_opts`
/// (`erl_process.c:12222-12434`) does. Any other option gives a runtime
/// error. After the options, a `module` or `function` that is not an atom
/// raises `badarg`, as in `spawn/3`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = spawn_opt(&mut context, &module, &function, &Term::List(vec![]), &Term::List(vec![monitor]))?;
/// assert!(matches!(result, BifResult::Value(Term::Tuple(_))));
/// assert!(matches!(spawn_opt(&mut context, &module, &function, &Term::List(vec![]), &link_x)?, BifResult::Raise { .. }));
/// assert!(spawn_opt(&mut context, &module, &function, &Term::List(vec![]), &Term::List(vec![priority])).is_err());
/// ```
pub(super) fn spawn_opt(
    context: &mut BifContext<'_>,
    module: &Term,
    function: &Term,
    arguments: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Term::List(options) = options else {
        return Ok(BifResult::badarg());
    };
    let mut spawn_options = SpawnOptions::default();
    for option in options {
        match option {
            Term::Atom(option) if option.name() == "link" => spawn_options.link = true,
            Term::Atom(option) if option.name() == "monitor" => spawn_options.monitor = true,
            // ponytail: ERTS accepts more options; see "Spawn options" in docs/erts-differences.md.
            _ => return Err(error(format!("unsupported spawn option {option:?}"))),
        }
    }
    create_process(
        context.program,
        context.scheduler,
        module,
        function,
        arguments,
        spawn_options,
    )
}

/// Run `erlang:put/2`: set the value of `key` in the process dictionary of
/// the calling process. Return the old value, or `undefined` for a new key,
/// as ERTS `pd_hash_put` (`erl_process_dict.c`) does. Keys compare with
/// exact equality.
///
/// # Examples
///
/// ```rust,ignore
/// let key = Term::static_atom("ok");
/// assert!(matches!(put(&mut context, &key, &Term::Integer(1)), BifResult::Value(Term::Atom(atom)) if atom.name() == "undefined"));
/// assert!(matches!(put(&mut context, &key, &Term::Integer(2)), BifResult::Value(Term::Integer(1))));
/// ```
pub(super) fn put(context: &mut BifContext<'_>, key: &Term, value: &Term) -> BifResult {
    let dictionary = &mut context.scheduler.current_process_mut().dictionary;
    let old = match dictionary.iter_mut().find(|(other, _)| other == key) {
        Some((_, old)) => std::mem::replace(old, value.clone()),
        None => {
            dictionary.push((key.clone(), value.clone()));
            Term::static_atom("undefined")
        }
    };
    Value(old)
}

/// Run `erlang:get/1`: return the value of `key` in the process dictionary
/// of the calling process, or `undefined` when the key has no value. Keys
/// compare with exact equality.
///
/// # Examples
///
/// ```rust,ignore
/// let key = Term::static_atom("ok");
/// assert!(matches!(get(&mut context, &key), BifResult::Value(Term::Atom(atom)) if atom.name() == "undefined"));
/// put(&mut context, &key, &Term::Integer(1));
/// assert!(matches!(get(&mut context, &key), BifResult::Value(Term::Integer(1))));
/// ```
pub(super) fn get(context: &mut BifContext<'_>, key: &Term) -> BifResult {
    let dictionary = &context.scheduler.current_process_mut().dictionary;
    Value(
        dictionary
            .iter()
            .find(|(other, _)| other == key)
            .map_or_else(
                || Term::static_atom("undefined"),
                |(_, value)| value.clone(),
            ),
    )
}

/// Run `erlang:'++'/2`: append `right` to the proper list `left`, as ERTS
/// `append` (`erl_bif_lists.c:262-338`) does. `right` can be any term, so
/// `[1] ++ x` gives the improper list `[1|x]`, and an empty `left` gives
/// `right` itself. A `left` that is not a proper list raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let x = Term::static_atom("x");
/// assert!(matches!(append(&Term::List(vec![]), &x), BifResult::Value(term) if term == x));
/// let list = append(&Term::List(vec![Term::Integer(1)]), &x);
/// assert!(matches!(list, BifResult::Value(Term::ImproperList(elements, _)) if elements.len() == 1));
/// assert!(matches!(append(&x, &Term::List(vec![])), BifResult::Raise { .. }));
/// ```
pub(super) fn append(left: &Term, right: &Term) -> BifResult {
    match left {
        Term::List(left) => Value(Term::list_with_tail(left.clone(), right.clone())),
        _ => BifResult::badarg(),
    }
}

/// The largest number of characters in an atom name (`MAX_ATOM_CHARACTERS`
/// in `atom.h`).
const MAX_ATOM_CHARACTERS: usize = 255;

/// Run `erlang:atom_to_list/1`: return the Unicode code points of the atom
/// name, as ERTS `atom_to_list_1` (`bif.c:3186-3210`) does. The atom `''`
/// gives `[]`. A term that is not an atom raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let list = atom_to_list(&Term::static_atom("ok"));
/// assert!(matches!(list, BifResult::Value(Term::List(codes)) if codes == [Term::Integer(111), Term::Integer(107)]));
/// assert!(matches!(atom_to_list(&Term::static_atom("")), BifResult::Value(Term::List(codes)) if codes.is_empty()));
/// assert!(matches!(atom_to_list(&Term::List(vec![])), BifResult::Raise { .. }));
/// ```
pub(super) fn atom_to_list(term: &Term) -> BifResult {
    match term {
        Term::Atom(atom) => Value(Term::List(
            atom.name()
                .chars()
                .map(|character| Term::Integer(i64::from(u32::from(character))))
                .collect(),
        )),
        _ => BifResult::badarg(),
    }
}

/// Run `erlang:list_to_atom/1`: return the atom whose name has the code
/// points of a list, and put a new atom into the atom table of the run, as
/// ERTS `list_to_atom_1` (`bif.c:3217-3236`) does with
/// `erts_unicode_list_to_buf` (`utils.c:2862-2942`). ERTS counts each
/// element before it checks it, so a 256th element raises `system_limit`,
/// also when it is not a character. ERTS checks the tail after the 255th
/// element, so an improper list of 300 characters raises `system_limit`.
/// An element that is not an integer in `0..=0x10FFFF` outside
/// `0xD800..=0xDFFF`, a tail that is not `[]`, and a term that is not a
/// list, raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let mut atoms = AtomTable::new();
/// let ok = list_to_atom(&mut atoms, &Term::List(vec![Term::Integer(111), Term::Integer(107)]))?;
/// assert!(matches!(ok, BifResult::Value(atom) if atom == Term::static_atom("ok")));
/// let surrogate = Term::List(vec![Term::Integer(0xD800)]);
/// assert!(matches!(list_to_atom(&mut atoms, &surrogate)?, BifResult::Raise { .. }));
/// let long = Term::List(vec![Term::Integer(97); 256]);
/// assert!(matches!(list_to_atom(&mut atoms, &long)?, BifResult::Raise { .. }));
/// let improper = Term::ImproperList(vec![Term::Integer(97)], Box::new(Term::static_atom("x")));
/// assert!(matches!(list_to_atom(&mut atoms, &improper)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A new atom in a full atom table gives the error of `AtomTable::put`,
/// where ERTS stops the emulator.
pub(super) fn list_to_atom(atoms: &mut AtomTable, term: &Term) -> Result<BifResult, BeamError> {
    let (elements, proper) = match term {
        Term::List(elements) => (elements, true),
        Term::ImproperList(elements, _) => (elements, false),
        _ => return Ok(BifResult::badarg()),
    };
    let mut name = String::new();
    for (count, element) in elements.iter().enumerate() {
        if count == MAX_ATOM_CHARACTERS {
            return Ok(Raise {
                class: ExceptionClass::Error,
                reason: Term::static_atom("system_limit"),
            });
        }
        let character = match element {
            Term::Integer(code) => u32::try_from(*code).ok().and_then(char::from_u32),
            _ => None,
        };
        let Some(character) = character else {
            return Ok(BifResult::badarg());
        };
        name.push(character);
    }
    if !proper {
        return Ok(BifResult::badarg());
    }
    Ok(Value(Term::Atom(atoms.put(&name)?)))
}

/// Run `erlang:system_info/1`. The item `os_type` returns `{unix, Flavor}`,
/// as ERTS `system_info_1` (`erl_bif_info.c:3026-3027`) does with `os_type`
/// `"unix"` (`sys/unix/sys.c:835`) and the OS flavor of the host. ERTS makes
/// both atoms at emulator start (`os_info_init` in
/// `erl_bif_info.c:6418-6440`); PBeam puts them into the atom table of the
/// run at this call.
///
/// # Examples
///
/// ```rust,ignore
/// let mut atoms = AtomTable::new();
/// let os_type = system_info(&mut atoms, &Term::static_atom("os_type"))?;
/// // macOS: {unix, darwin}.
/// assert!(matches!(os_type, BifResult::Value(Term::Tuple(_))));
/// assert!(system_info(&mut atoms, &Term::static_atom("otp_release")).is_err());
/// ```
///
/// # Errors
///
/// Each other item gives a runtime error, also an item that ERTS rejects
/// with `badarg`. A new atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn system_info(atoms: &mut AtomTable, item: &Term) -> Result<BifResult, BeamError> {
    match item {
        Term::Atom(atom) if atom.name() == "os_type" => Ok(Value(Term::Tuple(vec![
            Term::Atom(atoms.put("unix")?),
            Term::Atom(atoms.put(&host::os_flavor())?),
        ]))),
        _ => Err(error(format!(
            "unsupported erlang:system_info/1 argument {item:?}"
        ))),
    }
}

/// Run `erlang:'+'/2` or `erlang:'-'/2` with `operation`, the checked `i64`
/// sum or difference, as ERTS `erts_mixed_plus` and `erts_mixed_minus`
/// (`erl_arith.c:370-389`, `erl_arith.c:585-602`) do for two integers. An
/// operand that is not a number raises `badarith`. PBeam has no float term,
/// so each number is an integer. A result outside the `i64` range gives a
/// runtime error, where ERTS makes a bignum.
///
/// # Examples
///
/// ```rust,ignore
/// let sum = arith("+", &Term::Integer(1), &Term::Integer(2), i64::checked_add)?;
/// assert!(matches!(sum, BifResult::Value(Term::Integer(3))));
/// let atom = Term::static_atom("a");
/// assert!(matches!(arith("-", &atom, &Term::Integer(1), i64::checked_sub)?, BifResult::Raise { .. }));
/// assert!(arith("+", &Term::Integer(i64::MAX), &Term::Integer(1), i64::checked_add).is_err());
/// ```
pub(super) fn arith(
    name: &str,
    left: &Term,
    right: &Term,
    operation: fn(i64, i64) -> Option<i64>,
) -> Result<BifResult, BeamError> {
    let (Term::Integer(left), Term::Integer(right)) = (left, right) else {
        return Ok(BifResult::badarith());
    };
    // ponytail: a result outside i64 needs a bignum term; see "Integer range" in docs/erts-differences.md.
    let result = operation(*left, *right).ok_or_else(|| {
        error(format!(
            "unsupported erlang:{name}/2 result of {left} and {right}: needs a bignum"
        ))
    })?;
    Ok(Value(Term::Integer(result)))
}

/// Run `erlang:element/2`: return the element at the 1-based `index` of a
/// tuple. An `index` that is not an integer or is outside the tuple, and a
/// `tuple` that is not a tuple, raise `badarg`, as in ERTS `element_2`
/// (`bif.c`).
///
/// # Examples
///
/// ```rust,ignore
/// let tuple = Term::Tuple(vec![Term::static_atom("ok"), Term::Integer(1)]);
/// assert!(matches!(element(&Term::Integer(2), &tuple), BifResult::Value(Term::Integer(1))));
/// assert!(matches!(element(&Term::Integer(3), &tuple), BifResult::Raise { .. }));
/// assert!(matches!(element(&Term::Integer(1), &Term::List(vec![])), BifResult::Raise { .. }));
/// ```
pub(super) fn element(index: &Term, tuple: &Term) -> BifResult {
    let (Term::Integer(index), Term::Tuple(elements)) = (index, tuple) else {
        return BifResult::badarg();
    };
    usize::try_from(*index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| elements.get(index))
        .map_or_else(BifResult::badarg, |element| Value(element.clone()))
}

/// Run `erlang:is_function/2`: return `true` when `term` is a fun of
/// `arity` arguments, as ERTS `erl_is_function` (`erl_bif_op.c:237-267`)
/// does. An `arity` that is negative or not an integer raises `badarg`. A
/// non-negative arity that no fun has gives `false`. The `is_function2`
/// instruction runs the same check and passes only when it returns `true`
/// (`jit/arm/instr_common.cpp:1266-1283`).
///
/// # Examples
///
/// ```rust,ignore
/// // fun(X) -> {Tag, X} end
/// assert!(matches!(is_function(&pair, &Term::Integer(1)), BifResult::Value(value) if value == Term::static_atom("true")));
/// assert!(matches!(is_function(&pair, &Term::Integer(0)), BifResult::Value(value) if value == Term::static_atom("false")));
/// assert!(matches!(is_function(&Term::static_atom("notfun"), &Term::Integer(-1)), BifResult::Raise { .. }));
/// ```
pub(in crate::runtime::interpreter) fn is_function(term: &Term, arity: &Term) -> BifResult {
    let Term::Integer(arity) = *arity else {
        return BifResult::badarg();
    };
    if arity < 0 {
        return BifResult::badarg();
    }
    let matches = matches!(term, Term::Fun(fun) if i64::from(fun.target().arity()) == arity);
    Value(Term::static_atom(if matches { "true" } else { "false" }))
}

/// Send `message` to `to` and return the message, as ERTS `erl_send`
/// (`bif.c:2845-2908`) and `do_send` (`bif.c:2458-2695`) do for the `send`
/// instruction.
///
/// - A pid gets the message. ERTS drops a message to a process that does
///   not exist.
/// - A reference gives the message to the owner of its active alias, and
///   drops it when the reference is not an active alias
///   (`erl_proc_sig_queue.c:6026-6041`). A message through a
///   `reply_demonitor` alias ends the alias (see
///   [`Scheduler::alias_message_receiver`]).
/// - An atom gives the message to its registered process. A name that no
///   process has raises `badarg`. PBeam has no ports.
/// - `{Name, Node}` with two atoms and the local node gives the message to
///   the registered process, or drops it when no process has the name.
///   Another node drops the message, as ERTS does on a node that is not
///   alive (`dist.c:3234-3235`, `bif.c:2397-2400`, `2870-2872`). Each other
///   tuple raises `badarg`.
/// - Each other term raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(send(scheduler, &Term::Pid(pid), ok.clone()), BifResult::Value(value) if value == ok));
/// assert!(matches!(send(scheduler, &Term::static_atom("nobody"), ok.clone()), BifResult::Raise { .. }));
/// let remote = Term::Tuple(vec![Term::static_atom("name"), Term::static_atom("a@b")]);
/// assert!(matches!(send(scheduler, &remote, ok.clone()), BifResult::Value(_)));
/// ```
pub(in crate::runtime::interpreter) fn send(
    scheduler: &mut Scheduler,
    to: &Term,
    message: Term,
) -> BifResult {
    let target = match to {
        Term::Pid(pid) => Some(*pid),
        Term::Reference(reference) => scheduler.alias_message_receiver(*reference),
        Term::Atom(name) => match scheduler.whereis(name.name()) {
            Some(pid) => Some(pid),
            None => return BifResult::badarg(),
        },
        Term::Tuple(elements) => match elements.as_slice() {
            [Term::Atom(name), node @ Term::Atom(_)] if *node == local_node() => {
                scheduler.whereis(name.name())
            }
            [Term::Atom(_), Term::Atom(_)] => None,
            _ => return BifResult::badarg(),
        },
        _ => return BifResult::badarg(),
    };
    if let Some(target) = target {
        scheduler.deliver(target, message.clone());
    }
    Value(message)
}

/// Run `erlang:make_fun/3`: return the external fun of
/// `module:function/arity`, as ERTS `make_fun_3` (`bif.c:4467-4485`) does.
/// The module and the function must be atoms, and the arity must be an
/// integer in `0..=255` (`MAX_ARG` in `erl_vm.h:54`), else the call raises
/// `badarg`. The function does not need to exist.
///
/// # Examples
///
/// ```rust,ignore
/// let module = Term::static_atom("erlang");
/// let function = Term::static_atom("self");
/// assert!(matches!(make_fun(&module, &function, &Term::Integer(0)), BifResult::Value(Term::Fun(_))));
/// assert!(matches!(make_fun(&module, &function, &Term::Integer(256)), BifResult::Raise { .. }));
/// assert!(matches!(make_fun(&Term::Integer(1), &function, &Term::Integer(0)), BifResult::Raise { .. }));
/// ```
pub(super) fn make_fun(module: &Term, function: &Term, arity: &Term) -> BifResult {
    /// The largest arity of a BEAM function (`MAX_ARG` in `erl_vm.h:54`).
    const MAX_ARG: i64 = 255;
    match (module, function, arity.small_integer()) {
        (Term::Atom(module), Term::Atom(function), Some(arity @ 0..=MAX_ARG)) => Value(Term::Fun(
            Fun::external(module.clone(), function.clone(), arity as u32),
        )),
        _ => BifResult::badarg(),
    }
}

/// The largest binary, in bytes, that `erlang:iolist_to_iovec/1` copies into
/// the merged binary of its result. ERTS `IOL2V_COPY_THRESHOLD`
/// (`erl_iolist.c`) is `ERL_ONHEAP_BITS_LIMIT * 8` bits, and
/// `ERL_ONHEAP_BITS_LIMIT` (`erl_bits.h`) is `64 * 8` bits, so the limit is
/// 4096 bits, or 512 bytes. A larger binary is an element of the result.
const IOVEC_COPY_LIMIT: usize = 512;

/// Run `erlang:iolist_to_iovec/1` as ERTS `iolist_to_iovec_1` and
/// `iolist_to_iovec_engine` (`erl_iolist.c`) do. Return `[]` for `[]` and
/// `<<>>`, and `[Bin]` for a binary `Bin` that is not empty. For a list, walk
/// the nested lists in order. A binary tail of a list, as in `[97|<<"b">>]`,
/// comes after the elements of that list (`generic_iolist_size`,
/// `erl_iolist.c:153-236`). Copy each byte and each binary of at most
/// [`IOVEC_COPY_LIMIT`] bytes into one merged binary. Each larger binary
/// closes the current merged binary, if it is not empty, and is an element of
/// the result itself. An empty merged binary is not in the result. An integer
/// outside `0..=255`, a tail that is not a binary, or another term that is
/// not iodata raises `badarg`.
///
/// ERTS also copies a sub-binary at a bit offset that is not a multiple of 8,
/// whatever its size. PBeam binaries always start at a byte, so only the size
/// selects the rule.
///
/// # Examples
///
/// ```rust,ignore
/// let hello = Term::Binary(b"Hello".to_vec());
/// assert!(matches!(iolist_to_iovec(&hello), BifResult::Value(Term::List(iovec)) if iovec == [hello]));
/// assert!(matches!(iolist_to_iovec(&Term::Binary(Vec::new())), BifResult::Value(Term::List(iovec)) if iovec.is_empty()));
/// // [1, [<<2>>]] gives [<<1,2>>].
/// let list = Term::List(vec![Term::Integer(1), Term::List(vec![Term::Binary(vec![2])])]);
/// assert!(matches!(iolist_to_iovec(&list), BifResult::Value(Term::List(iovec)) if iovec == [Term::Binary(vec![1, 2])]));
/// // [1, <<0:513/unit:8>>] gives [<<1>>, <<0:513/unit:8>>].
/// let large = Term::Binary(vec![0; 513]);
/// let list = Term::List(vec![Term::Integer(1), large.clone()]);
/// assert!(matches!(iolist_to_iovec(&list), BifResult::Value(Term::List(iovec)) if iovec == [Term::Binary(vec![1]), large]));
/// // [97|<<"b">>] gives [<<"ab">>].
/// let improper = Term::ImproperList(vec![Term::Integer(97)], Box::new(Term::Binary(b"b".to_vec())));
/// assert!(matches!(iolist_to_iovec(&improper), BifResult::Value(Term::List(iovec)) if iovec == [Term::Binary(b"ab".to_vec())]));
/// assert!(matches!(iolist_to_iovec(&Term::List(vec![Term::Integer(256)])), BifResult::Raise { .. }));
/// assert!(matches!(iolist_to_iovec(&Term::static_atom("foo")), BifResult::Raise { .. }));
/// ```
pub(super) fn iolist_to_iovec(term: &Term) -> BifResult {
    match term {
        Term::Binary(bytes) if bytes.is_empty() => Value(Term::List(Vec::new())),
        Term::Binary(_) => Value(Term::List(vec![term.clone()])),
        Term::List(_) | Term::ImproperList(..) => {
            iolist_iovec(term).map_or_else(BifResult::badarg, |iovec| Value(Term::List(iovec)))
        }
        _ => BifResult::badarg(),
    }
}

/// Return the bytes of iodata: the bytes of a binary, or the bytes of each
/// element and each binary tail of an iolist in order, as
/// `erts_iolist_to_buf` writes them. Return `None` for any other term and
/// for an iolist with an element or a tail that is not iodata.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(iodata_bytes(&Term::Binary(b"ab".to_vec())), Some(b"ab".to_vec()));
/// // [$a, [<<"b">>]] gives <<"ab">>.
/// let list = Term::List(vec![Term::Integer(97), Term::List(vec![Term::Binary(b"b".to_vec())])]);
/// assert_eq!(iodata_bytes(&list), Some(b"ab".to_vec()));
/// // [$a|<<"b">>] gives <<"ab">>, and [$a|x] is not iodata.
/// let tail = |tail| Term::ImproperList(vec![Term::Integer(97)], Box::new(tail));
/// assert_eq!(iodata_bytes(&tail(Term::Binary(b"b".to_vec()))), Some(b"ab".to_vec()));
/// assert_eq!(iodata_bytes(&tail(Term::static_atom("x"))), None);
/// assert_eq!(iodata_bytes(&Term::List(vec![Term::Integer(256)])), None);
/// assert_eq!(iodata_bytes(&Term::static_atom("foo")), None);
/// ```
pub(super) fn iodata_bytes(term: &Term) -> Option<Vec<u8>> {
    match term {
        Term::Binary(bytes) => Some(bytes.clone()),
        Term::List(_) | Term::ImproperList(..) => {
            let mut bytes = Vec::new();
            // An iovec holds only binaries.
            for binary in iolist_iovec(term)? {
                if let Term::Binary(part) = &binary {
                    bytes.extend_from_slice(part);
                }
            }
            Some(bytes)
        }
        _ => None,
    }
}

/// Convert a proper or improper iolist to the iovec of
/// `erlang:iolist_to_iovec/1`, or return `None` when an element or a tail is
/// not iodata. A binary tail of a list comes after the elements of that
/// list. Each other tail gives `None`, whatever comes before it, because
/// ERTS walks the whole term before it builds the result
/// (`generic_iolist_size`, `erl_iolist.c:153-236`). Use a stack of list
/// iterators, as the ERTS engine does, so that a deep nested list does not
/// use the Rust call stack.
///
/// # Examples
///
/// ```rust,ignore
/// let iolist = Term::List(vec![Term::List(vec![]), Term::Binary(vec![]), Term::Integer(7)]);
/// assert_eq!(iolist_iovec(&iolist), Some(vec![Term::Binary(vec![7])]));
/// // [7|<<8>>] gives [<<7,8>>], and [7|8] is not iodata.
/// let improper = |tail| Term::ImproperList(vec![Term::Integer(7)], Box::new(tail));
/// assert_eq!(iolist_iovec(&improper(Term::Binary(vec![8]))), Some(vec![Term::Binary(vec![7, 8])]));
/// assert_eq!(iolist_iovec(&improper(Term::Integer(8))), None);
/// assert_eq!(iolist_iovec(&Term::List(vec![Term::Integer(-1)])), None);
/// ```
fn iolist_iovec(list: &Term) -> Option<Vec<Term>> {
    let mut iovec = Vec::new();
    let mut merged = Vec::new();
    // Each level holds the elements of a list that the walk did not read
    // yet, and its binary tail, which comes after them.
    let mut lists = vec![(std::slice::from_ref(list).iter(), None)];
    while let Some((elements, tail)) = lists.last_mut() {
        let Some(element) = elements.next().or_else(|| tail.take()) else {
            lists.pop();
            continue;
        };
        match element {
            Term::List(inner) => lists.push((inner.iter(), None)),
            Term::ImproperList(inner, tail) if matches!(**tail, Term::Binary(_)) => {
                lists.push((inner.iter(), Some(&**tail)));
            }
            Term::Integer(byte) => merged.push(u8::try_from(*byte).ok()?),
            Term::Binary(bytes) if bytes.len() <= IOVEC_COPY_LIMIT => {
                merged.extend_from_slice(bytes);
            }
            binary @ Term::Binary(_) => {
                if !merged.is_empty() {
                    iovec.push(Term::Binary(std::mem::take(&mut merged)));
                }
                iovec.push(binary.clone());
            }
            _ => return None,
        }
    }
    if !merged.is_empty() {
        iovec.push(Term::Binary(merged));
    }
    Some(iovec)
}

/// The item atoms that `erlang:process_info/2` accepts (`pi_arg2ix` in
/// `erl_bif_info.c`). The tuple `{dictionary, Key}` is also an item.
const PROCESS_INFO_ITEMS: [&str; 40] = [
    "registered_name",
    "current_function",
    "initial_call",
    "status",
    "messages",
    "message_queue_len",
    "links",
    "monitors",
    "monitored_by",
    "dictionary",
    "trap_exit",
    "error_handler",
    "heap_size",
    "stack_size",
    "memory",
    "garbage_collection",
    "group_leader",
    "reductions",
    "priority",
    "trace",
    "binary",
    "sequential_trace_token",
    "catchlevel",
    "backtrace",
    "last_calls",
    "total_heap_size",
    "suspending",
    "min_heap_size",
    "min_bin_vheap_size",
    "max_heap_size",
    "current_location",
    "current_stacktrace",
    "message_queue_data",
    "garbage_collection_info",
    "magic_ref",
    "fullsweep_after",
    "parent",
    "async_dist",
    "label",
    "priority_messages",
];

/// Return whether a term is one `erlang:process_info/2` item: an atom of
/// `PROCESS_INFO_ITEMS` or a tuple `{dictionary, Key}`, as `pi_arg2ix`
/// (`erl_bif_info.c`) accepts.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_process_info_item(&Term::static_atom("registered_name")));
/// let lookup = Term::Tuple(vec![Term::static_atom("dictionary"), Term::Integer(1)]);
/// assert!(is_process_info_item(&lookup));
/// assert!(!is_process_info_item(&Term::static_atom("ok")));
/// ```
fn is_process_info_item(item: &Term) -> bool {
    match item {
        Term::Atom(name) => PROCESS_INFO_ITEMS.contains(&name.name()),
        Term::Tuple(elements) => {
            matches!(elements.as_slice(), [Term::Atom(key), _] if key.name() == "dictionary")
        }
        _ => false,
    }
}

/// Run `erlang:process_info/2`. As `process_info_bif` (`erl_bif_info.c`)
/// does, first raise `badarg` when `item` is not an item or a proper list of
/// items, then raise `badarg` when `process` is not a pid. Return
/// `undefined` for each valid item when the process is not alive
/// (`erl_bif_info.c:1384-1386`). For a live process, return
/// `{registered_name, Name}`, or `[]` when the process has no name, as
/// `process_info_aux` does. For the calling process, return
/// `{current_stacktrace, []}`, because PBeam records no stack frames. The
/// `current_stacktrace` of another process, a list of items, and any other
/// item give a runtime error.
///
/// # Examples
///
/// ```rust,ignore
/// let process = Term::Pid(context.scheduler.current_process_mut().id);
/// let item = Term::static_atom("current_stacktrace");
/// let info = process_info(&mut context, &process, &item)?;
/// assert!(matches!(info, BifResult::Value(Term::Tuple(elements)) if elements[1] == Term::List(vec![])));
/// let name = process_info(&mut context, &process, &Term::static_atom("registered_name"))?;
/// assert!(matches!(name, BifResult::Value(Term::List(elements)) if elements.is_empty()));
/// let dead = process_info(&mut context, &Term::Pid(ProcessId(99)), &item)?;
/// assert!(matches!(dead, BifResult::Value(Term::Atom(atom)) if atom.name() == "undefined"));
/// let bogus = process_info(&mut context, &process, &Term::static_atom("bogus_item"))?;
/// assert!(matches!(bogus, BifResult::Raise { .. }));
/// assert!(process_info(&mut context, &process, &Term::static_atom("status")).is_err());
/// ```
pub(super) fn process_info(
    context: &mut BifContext<'_>,
    process: &Term,
    item: &Term,
) -> Result<BifResult, BeamError> {
    let valid = match item {
        Term::List(items) => items.iter().all(is_process_info_item),
        item => is_process_info_item(item),
    };
    let (true, Term::Pid(id)) = (valid, process) else {
        return Ok(BifResult::badarg());
    };
    let current = context.scheduler.current_process_mut().id;
    let Some(target) = context.scheduler.process(*id) else {
        return Ok(Value(Term::static_atom("undefined")));
    };
    let info = match item {
        Term::Atom(name) => match (name.name(), &target.registered_name) {
            // ponytail: PBeam records no stack frames; see "Exception stack trace" in docs/erts-differences.md.
            ("current_stacktrace", _) if *id == current => {
                Some(Term::Tuple(vec![item.clone(), Term::List(Vec::new())]))
            }
            ("registered_name", Some(registered)) => Some(Term::Tuple(vec![
                item.clone(),
                Term::Atom(registered.clone()),
            ])),
            ("registered_name", None) => Some(Term::List(Vec::new())),
            _ => None,
        },
        _ => None,
    };
    // ponytail: lists of items, other items, and the frames of another process need more state; see "BIF arguments" in docs/erts-differences.md.
    info.map(Value).ok_or_else(|| {
        error(format!(
            "unsupported erlang:process_info/2 arguments {process:?}, {item:?}"
        ))
    })
}

/// Run `erlang:function_exported/3`: return `true` when a loaded module
/// exports the function or the function is a BIF, as ERTS
/// `function_exported_3` (`bif.c`) does with `erts_find_function` and
/// `erts_is_builtin`. A `module` or `name` that is not an atom, and an
/// `arity` that is not a small integer, raise `badarg`. ERTS reads the
/// small-integer arity into a C `int`, so only its low 32 bits count, and a
/// negative arity finds no function.
///
/// # Examples
///
/// ```rust,ignore
/// let erlang = Term::static_atom("erlang");
/// let exported = function_exported(&program, &erlang, &Term::static_atom("self"), &Term::Integer(0));
/// assert!(matches!(exported, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let exported = function_exported(&program, &erlang, &Term::static_atom("self"), &Term::Integer(1 << 59));
/// assert!(matches!(exported, BifResult::Raise { .. }));
/// ```
pub(super) fn function_exported(
    program: &BeamProgram,
    module: &Term,
    name: &Term,
    arity: &Term,
) -> BifResult {
    let (Term::Atom(module), Term::Atom(name), Some(arity)) = (module, name, arity.small_integer())
    else {
        return BifResult::badarg();
    };
    let (module, name) = (module.name(), name.name());
    let exported = u32::try_from(arity as i32).is_ok_and(|arity| {
        program
            .modules
            .get(module)
            .is_some_and(|loaded| loaded.exports.contains_key(&(name.to_owned(), arity)))
            || is_bif(module, name, arity)
            // `erts_is_builtin` (`beam_common.c`) also counts these
            // instruction calls, which are not `bif.tab` entries.
            || matches!(
                (module, name, arity),
                ("erlang", "apply", 2 | 3) | ("erlang", "yield", 0)
            )
    });
    Value(Term::static_atom(if exported { "true" } else { "false" }))
}

/// Run `erlang:start_timer(Time, Dest, Msg)`, as ERTS `start_timer_3` and
/// `setup_bif_timer` (`erl_hl_timer.c:2605-2620`, `1804-1871`) do:
///
/// - `Time` goes through the relative branch of `parse_timeout_pos`
///   ([`Timers::timeout_position`]). A term that is not an integer, a
///   negative integer, an end after `ERTS_MONOTONIC_END`, and a 64-bit
///   overflow raise `badarg`.
/// - Then a `Dest` that is not a pid or an atom raises `badarg`.
/// - Then the BIF makes a new reference and returns it. A pid that is not
///   alive gets no timer (`erl_hl_timer.c:1832-1858`).
///
/// When the timer expires, `Dest` gets `{timeout, Reference, Msg}`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = start_timer(&mut context, &Term::Integer(100), &Term::Pid(pid), &msg)?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// let result = start_timer(&mut context, &Term::Integer(-1), &Term::Pid(pid), &msg)?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A reference counter overflow gives a runtime error.
pub(super) fn start_timer(
    context: &mut BifContext<'_>,
    time: &Term,
    receiver: &Term,
    message: &Term,
) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let end = match *time {
        Term::Integer(msec) => Timers::timeout_position(scheduler.timers.now(), msec),
        _ => None,
    };
    let Some(end) = end else {
        return Ok(BifResult::badarg());
    };
    let receiver = match receiver {
        Term::Pid(id) => Receiver::Process(*id),
        Term::Atom(name) => Receiver::Name(name.clone()),
        _ => return Ok(BifResult::badarg()),
    };
    let reference = scheduler.make_ref()?;
    if !matches!(receiver, Receiver::Process(id) if scheduler.process(id).is_none()) {
        let message = Term::Tuple(vec![
            Term::static_atom("timeout"),
            Term::Reference(reference),
            message.clone(),
        ]);
        scheduler.timers.start(
            end,
            Timer::Bif {
                reference,
                receiver,
                message,
            },
        );
    }
    Ok(Value(Term::Reference(reference)))
}

/// Run `erlang:cancel_timer(TimerRef, Options)`, as ERTS `cancel_timer_2`
/// (`erl_hl_timer.c:2649-2659`) does:
///
/// - `parse_bif_timer_options` (`erl_hl_timer.c:2352-2398`) reads a list of
///   `{async, Bool}` and `{info, Bool}`, and the last value of an option
///   wins. The defaults are `async` false and `info` true. Each other
///   element, also `{abs, Bool}`, raises `badarg`.
/// - Then a `TimerRef` that is not a reference raises `badarg`
///   (`access_bif_timer`, `erl_hl_timer.c:2237-2340`).
/// - Then `access_btm` (`erl_hl_timer.c:1922-1971`) cancels the active
///   timer and gives the milliseconds left (`get_time_left`), or `false`
///   when no timer is active.
/// - `access_sched_local_btm` (`erl_hl_timer.c:2078-2147`) gives the
///   result: `info` false gives `ok`. Else `async` false gives the
///   milliseconds left or `false` (`return_info`, `erl_hl_timer.c:1975-1989`),
///   and `async` true sends `{cancel_timer, TimerRef, Result}` to the caller
///   and gives `ok` (`send_async_info`, `erl_hl_timer.c:1992-2034`).
///
/// Each reference that PBeam can represent is a reference of the local
/// scheduler, so the `no_timer_result` path (`erl_hl_timer.c:2209-2234`)
/// never runs.
///
/// # Examples
///
/// ```rust,ignore
/// let result = cancel_timer(&mut context, &Term::Reference(reference), &Term::List(vec![]))?;
/// assert!(matches!(result, BifResult::Value(Term::Integer(100))));
/// let result = cancel_timer(&mut context, &Term::Reference(reference), &Term::List(vec![]))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// ```
pub(super) fn cancel_timer(
    context: &mut BifContext<'_>,
    reference: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let (Some((asynchronous, info)), Term::Reference(reference)) =
        (timer_options(options), reference)
    else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    let result = match scheduler.timers.cancel_bif(*reference) {
        Some(end) => Term::Integer(Timers::time_left(scheduler.timers.now(), end)),
        None => Term::static_atom("false"),
    };
    if !info {
        return Ok(Value(Term::static_atom("ok")));
    }
    if !asynchronous {
        return Ok(Value(result));
    }
    let caller = scheduler.current_process_mut().id;
    let message = Term::Tuple(vec![
        Term::static_atom("cancel_timer"),
        Term::Reference(*reference),
        result,
    ]);
    scheduler.deliver(caller, message);
    Ok(Value(Term::static_atom("ok")))
}

/// Read the options of `erlang:cancel_timer/2` as `parse_bif_timer_options`
/// (`erl_hl_timer.c:2352-2398`) does without the `abs` option. Return
/// `(async, info)`, or `None` for an option list that is not valid.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(timer_options(&Term::List(vec![])), Some((false, true)));
/// assert_eq!(timer_options(&Term::static_atom("foo")), None);
/// ```
fn timer_options(options: &Term) -> Option<(bool, bool)> {
    let Term::List(options) = options else {
        return None;
    };
    let (mut asynchronous, mut info) = (false, true);
    for option in options {
        let Term::Tuple(fields) = option else {
            return None;
        };
        let [Term::Atom(name), Term::Atom(value)] = fields.as_slice() else {
            return None;
        };
        let value = match value.name() {
            "true" => true,
            "false" => false,
            _ => return None,
        };
        match name.name() {
            "async" => asynchronous = value,
            "info" => info = value,
            _ => return None,
        }
    }
    Some((asynchronous, info))
}
