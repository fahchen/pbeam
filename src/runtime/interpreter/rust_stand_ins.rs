//! `erlang` functions that ERTS 29.1.1 runs as BEAM code from `erlang.beam`
//! and PBeam still runs in Rust. They are not BIFs: `erlang:is_builtin/3`
//! returns `false` for them, so they are not in the BIF table. #87 removes
//! this module and runs the functions from the pinned `erlang.beam`.

use super::*;

/// Run `erlang:group_leader/2` or `erlang:spawn_monitor/3` for an external
/// call, or return `None` for any other MFA.
///
/// # Examples
///
/// ```rust,ignore
/// let value = run(&program, &mut scheduler, "erlang", "group_leader", &[Term::Pid(leader), Term::Pid(target)]);
/// assert_eq!(value.unwrap()?, Term::static_atom("true"));
/// assert!(run(&program, &mut scheduler, "erlang", "self", &[]).is_none());
/// ```
pub(super) fn run(
    program: &BeamProgram,
    scheduler: &mut Scheduler,
    module: &str,
    name: &str,
    arguments: &[Term],
) -> Option<Result<Term, BeamError>> {
    match (module, name, arguments) {
        ("erlang", "group_leader", [Term::Pid(leader), Term::Pid(target)]) => Some(
            scheduler
                .set_group_leader(*target, *leader)
                .map(|()| Term::static_atom("true")),
        ),
        ("erlang", "spawn_monitor", [Term::Atom(module), Term::Atom(function), arguments]) => {
            let options = SpawnOptions {
                monitor: true,
                ..SpawnOptions::default()
            };
            Some(create_process(
                program, scheduler, module, function, arguments, options,
            ))
        }
        _ => None,
    }
}
