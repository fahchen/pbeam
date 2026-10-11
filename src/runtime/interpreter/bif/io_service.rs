//! The IO service entry of the runtime to boot gate. `pbeam_cap:serve(Pid,
//! io)` registers a boot IO server, for example `user` or
//! `standard_error`, as an IO service. A message from a runtime process to
//! an IO service passes the gate only when it is an IO request that the
//! sender may make, so the IO server writes for a runtime process only
//! with the `terminal` authority of that process (row "Boot authority" of
//! `docs/erts-differences.md`).

use super::*;
use crate::runtime::capability::SecurityContext;

/// Return the `error:capability_denied` exception for a message from the
/// current process to the process `target` that the runtime to boot gate
/// denies (see [`boot_denial`]): the current process is not a boot
/// process, `target` is a boot process, and `target` is not an IO service
/// or `message` is not an IO request that the current process may make
/// (see [`io_request_permitted`]). Else return `None`. The send path calls
/// this after its capability check and the name lookup, and before the
/// send, so a denied message is not sent.
///
/// # Examples
///
/// ```rust,ignore
/// // The current process entered the runtime with `terminal` `write`;
/// // `user` is an IO service.
/// let request = Term::tuple(vec![io_request, Term::Pid(caller), reply_as, put_chars]);
/// assert!(send_denial(&mut scheduler, user, &request)?.is_none());
/// assert!(send_denial(&mut scheduler, user, &Term::static_atom("x"))?.is_some());
/// ```
///
/// # Errors
///
/// The errors of `AtomTable::put` for a full atom table.
pub(super) fn send_denial(
    scheduler: &mut Scheduler,
    target: ProcessId,
    message: &Term,
) -> Result<Option<BifResult>, BeamError> {
    let sender = scheduler.current_process();
    if scheduler.serves_io(target) && io_request_permitted(sender.id, &sender.security, message) {
        return Ok(None);
    }
    boot_denial(scheduler, target)
}

/// Return whether `message` from the process `sender` with the security
/// context `security` is an IO request that an IO service accepts from a
/// runtime process: `{io_request, From, ReplyAs, Request}`, where `From` is
/// `sender` and the IO server can do `Request` for the sender:
///
/// - `{put_chars, Encoding, Chars}` needs the `terminal` `write`
///   capability of the sender.
/// - `{put_chars, Encoding, io_lib, Function, Arguments}`, which `io`
///   makes for `io:format/2`, `io:fwrite/2`, and `io:write/1`
///   (`lib/stdlib/src/io.erl:1750-1755`), needs `terminal` `write`, a
///   `Function` of `format`, `fwrite`, or `write`, and `Arguments` with no
///   fun at any depth. The IO server applies the function in its own
///   process (`lib/kernel/src/group.erl:626-627`), and these functions call
///   a fun of the `~K` modifier (`lib/stdlib/src/io_lib_format.erl:247-248`)
///   or of the `{maps_order, Fun}` option (`lib/stdlib/src/io_lib.erl:599-604`).
/// - `{requests, Requests}` needs a proper list of such requests, also
///   nested.
///
/// Each other message and request is denied. The walk uses a work list,
/// not recursion, so deeply nested requests do not overflow the stack.
///
/// # Examples
///
/// ```rust,ignore
/// let put_chars = Term::tuple(vec![Term::static_atom("put_chars"), unicode, Term::binary(b"hi\n".to_vec())]);
/// let request = Term::tuple(vec![io_request, Term::Pid(sender), reply_as, put_chars]);
/// assert!(io_request_permitted(sender, &security, &request));
/// ```
fn io_request_permitted(sender: ProcessId, security: &SecurityContext, message: &Term) -> bool {
    let Term::Tuple(elements) = message else {
        return false;
    };
    let [Term::Atom(tag), Term::Pid(from), _reply_as, request] = elements.as_ref() else {
        return false;
    };
    if tag.name() != "io_request" || *from != sender {
        return false;
    }
    let writes = || {
        security
            .check(CapabilityKind::Terminal, Operation::Write, None)
            .is_ok()
    };
    let mut pending = vec![request];
    while let Some(request) = pending.pop() {
        let Term::Tuple(elements) = request else {
            return false;
        };
        let permitted = match elements.as_ref() {
            [Term::Atom(tag), _encoding, _chars] => tag.name() == "put_chars" && writes(),
            [
                Term::Atom(tag),
                _encoding,
                Term::Atom(module),
                Term::Atom(function),
                arguments,
            ] => {
                tag.name() == "put_chars"
                    && module.name() == "io_lib"
                    && matches!(function.name(), "format" | "fwrite" | "write")
                    && !arguments.contains_fun()
                    && writes()
            }
            [Term::Atom(tag), requests] if tag.name() == "requests" => {
                let Some(requests) = requests.proper_list() else {
                    return false;
                };
                pending.extend(requests);
                true
            }
            _ => false,
        };
        if !permitted {
            return false;
        }
    }
    true
}
