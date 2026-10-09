use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const EXIT_OWNED_PROBE: &[u8] = include_bytes!("../../fixtures/preempt/exit_owned_probe.beam");

/// The stand-in `prim_tty` with the NIF stubs of the pinned module. It is in
/// a subdirectory, so `-pa` does not load it at boot.
const STAND_IN: &[u8] = include_bytes!("../../fixtures/prim_tty/nif/prim_tty.beam");

/// The spawn options of the measured processes in the ERTS reference run: a
/// heap of 4,000,000 words and a binary heap of 40,000,000 words, so that no
/// measured process collects garbage. PBeam has no process heap and runs
/// with `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,4000000},{min_bin_vheap_size,40000000}]";

/// The cases of `exit_owned_probe:tables/2`: `{Sizes, K}`. The exiting
/// process `a` has `3997 - K` reductions left when its exit starts, and
/// `3897 - K` after the bump of 100 (`erl_process.c:14229`).
const TABLE_CASES: &str = "[{[10],3854},{[10],3855},{[10],3837},{[300],3600},{[3000,3000],3335},{\
                           [3000,3000],3336},{[12000,10],2795},{[12000,10],2796},{\
                           [{5000,4000}],2800},{[{5000,4000}],2801},{[{atoms,3000},{\
                           atoms,3000}],3328},{[{atoms,3000},{atoms,3000}],3329},{[{atoms,3000},{\
                           atoms,3000}],3368}]";

/// `exit_owned_probe:tables/2` with [`TABLE_CASES`] on the pinned ERTS with
/// `erl +S 1`. The deletion counts from the reductions left in the turn.
/// Each segment uses 32 and 1 for each object in it (`erl_db_hash.c:3141`),
/// from the top segment down. The deletion switches out when the reductions
/// are less than 0 after a segment, or 0 after a table
/// (`erl_db.c:5098-5104`, `5131`).
///
/// - `[10]`: one segment of 42. With 43 left, the deletion ends with 1, and
///   leaving the process table switches out the exit, so `b` first sees
///   that `a` has no name. With 42 left, the deletion ends at 0 and switches
///   out, so `b` sees the name of `a` but not its table. With 60 left, the
///   exit ends in its first turn: leaving the process table uses 50 of the
///   count of the exit, which the deletion does not change
///   (`erl_process.c:14276`, `14360-14366`, `14475`).
/// - `[300]` with 297 left: 300 buckets give a top segment of 65 and a
///   first segment of 299, so the deletion switches out.
/// - `[3000, 3000]`: the top segment of each table uses 562. With 562 left,
///   it ends at 0, the next segment switches out, and the second turn
///   deletes both tables. With 561 left, the first segment switches out,
///   and the second table needs a third turn.
/// - `[12000, 10]`: the top segment uses 1102. With 1102 left, the exit
///   takes three turns, and with 1101 left four.
/// - `[{5000, 4000}]`: the deletes join buckets down to 3592, so three
///   segments use 1096. With 1097 left, the exit ends in its first turn,
///   and with 1096 left it switches out.
/// - `[{atoms, 3000}, {atoms, 3000}]`: the atom table hash puts 537 keys in
///   the top segment (569). With 569 left, the exit takes two turns, and
///   with 568 left three. With 529 left, it takes three turns. A hash that
///   reads the two bytes of `é` as the second byte gives 529 for the top
///   segment, and so two turns.
const TABLES: &str = "[{{[10],3854},[{{b,false,0},363},{{down,normal},1},{\
                      {b,false,0},5637}]},{{[10],3855},[{{b,true,0},363},{{down,normal},1},{\
                      {b,true,0},1},{{b,false,0},5636}]},{{[10],3837},[{{down,normal},1},{\
                      {b,false,0},6000}]},{{[300],3600},[{{b,true,0},363},{{down,normal},1},{\
                      {b,true,0},1},{{b,false,0},5636}]},{\
                      {[3000,3000],3335},[{{b,true,1},285},{{down,normal},1},{{b,true,1},1},{\
                      {b,false,0},5714}]},{{[3000,3000],3336},[{{b,true,1},286},{\
                      {b,true,0},307},{{down,normal},1},{{b,true,0},1},{{b,false,0},5406}]},{\
                      {[12000,10],2795},[{{b,true,1},571},{{down,normal},1},{{b,true,0},1},{\
                      {b,false,0},5428}]},{{[12000,10],2796},[{{b,true,1},857},{\
                      {down,normal},1},{{b,false,0},5143}]},{\
                      {[{5000,4000}],2800},[{{down,normal},1},{{b,false,0},6000}]},{\
                      {[{5000,4000}],2801},[{{b,true,0},363},{{down,normal},1},{\
                      {b,true,0},1},{{b,false,0},5636}]},{{[{atoms,3000},{\
                      atoms,3000}],3328},[{{b,true,1},285},{{down,normal},1},{{b,true,1},1},{\
                      {b,false,0},5714}]},{{[{atoms,3000},{\
                      atoms,3000}],3329},[{{b,true,1},286},{{b,true,0},307},{\
                      {down,normal},1},{{b,true,0},1},{{b,false,0},5406}]},{{[{atoms,3000},{\
                      atoms,3000}],3368},[{{b,true,1},286},{{b,true,0},307},{\
                      {down,normal},1},{{b,true,0},1},{{b,false,0},5406}]}]";

/// The cases of `exit_owned_probe:ttys/2`: `{Before, After, Late, K}`. The
/// monitor list of `a` has the monitor of `spawn_opt/4`, the `Before`
/// monitors, the TTY monitor of `tty_select/2`, and the `After` monitors.
const TTY_CASES: &str = "[{10,10,false,3842},{10,10,false,3830},{10,10,true,3842},{\
                         10,10,true,3600}]";

/// `exit_owned_probe:ttys/2` with [`TTY_CASES`] on the pinned ERTS with
/// `erl +S 1`. `enif_monitor_process` sends a monitor signal, so the TTY
/// monitor is in the list at the point where `a` handled the signal
/// (`erl_nif.c:3985-3989`; `erl_proc_sig_queue.c:6544-6562`). Each entry
/// uses 1 (`erl_process.c:13797-13833`).
///
/// - `{10, 10, false, 3842}`: 3 reductions are left for the list, so the
///   first turn sends 3 `DOWN` messages. A TTY monitor at the start of the
///   list would give 2.
/// - `{10, 10, false, 3830}`: 15 are left: 14 `DOWN` messages and the TTY
///   monitor.
/// - `{10, 10, true, 3842}`: the late `tty_select/2` uses 1 more, so 2
///   `DOWN` messages. Its monitor signal is still in the queue, and the
///   signal phase runs the TTY monitor (`erl_proc_sig_queue.c:7347-7355`).
///   Two TTY monitors at the start of the list would give no `DOWN`
///   message.
/// - `{10, 10, true, 3600}`: the exit ends in its first turn.
const TTYS: &str = "[{{10,10,false,3842},[{{down,normal},3},{{b,false,0},444},{\
                    {down,normal},18},{{b,false,0},1556}]},{\
                    {10,10,false,3830},[{{down,normal},14},{{b,false,0},444},{\
                    {down,normal},7},{{b,false,0},1556}]},{\
                    {10,10,true,3842},[{{down,normal},2},{{b,false,0},444},{\
                    {down,normal},19},{{b,false,0},1556}]},{\
                    {10,10,true,3600},[{{down,normal},21},{{b,false,0},2000}]}]";

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// lists, and tuples of the probe results.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::List(vec![Term::Integer(1)])), "[1]");
/// ```
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        Term::List(terms) => format!("[{}]", join(terms)),
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

/// Return the term of the case list `cases`, which has only atoms,
/// integers, lists, and tuples.
///
/// # Examples
///
/// ```rust,ignore
/// let cases = parse(&program, "[{[10],3854}]");
/// ```
fn parse(program: &BeamProgram, cases: &str) -> Term {
    fn term(program: &BeamProgram, text: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Term {
        let close = match text.next() {
            Some('[') => ']',
            Some('{') => '}',
            Some(first) => {
                let mut word = first.to_string();
                while let Some(&next) = text.peek()
                    && next.is_ascii_alphanumeric()
                {
                    word.push(next);
                    text.next();
                }
                return match word.parse() {
                    Ok(value) => Term::Integer(value),
                    Err(_) => Term::Atom(program.atom(&word).unwrap()),
                };
            }
            None => panic!("unexpected end"),
        };
        let mut elements = Vec::new();
        while text.peek() != Some(&close) {
            elements.push(term(program, text));
            if text.peek() == Some(&',') {
                text.next();
            }
        }
        text.next();
        if close == ']' {
            Term::List(elements)
        } else {
            Term::Tuple(elements)
        }
    }
    let parsed = term(program, &mut cases.chars().peekable());
    assert_eq!(text(&parsed), cases);
    parsed
}

#[test]
fn deletes_the_owned_tables_as_pinned_erts_with_one_scheduler() {
    let program = BeamProgram::load(&[EXIT_OWNED_PROBE]).unwrap();
    let tables = program
        .execute(
            "exit_owned_probe",
            "tables",
            &[parse(&program, TABLE_CASES), Term::List(vec![])],
        )
        .unwrap();
    assert_eq!(text(&tables), TABLES);

    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            &format!(
                "{{module, M}} = code:ensure_loaded(exit_owned_probe), \
                 jit = erlang:system_info(emu_flavor), O = {ERTS_OPTIONS}, C = {TABLE_CASES}, \
                 E = M:tables(C, O), [0] = lists:usort([G || {{_, G}} <- M:table_gcs(C, O)]), \
                 io:format(\"~w~n\", [E]), halt()."
            ),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{TABLES}\n")
    );
}

#[test]
fn runs_the_tty_monitors_in_the_order_of_pinned_erts() {
    let program = BeamProgram::load(&[EXIT_OWNED_PROBE, STAND_IN]).unwrap();
    let ttys = program
        .execute(
            "exit_owned_probe",
            "ttys",
            &[parse(&program, TTY_CASES), Term::List(vec![])],
        )
        .unwrap();
    assert_eq!(text(&ttys), TTYS);

    // The stand-in replaces the `prim_tty` of the boot, so the reference
    // prints with `erlang:display/1`, which needs no IO server and ends the
    // line with `\r\n`.
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noinput",
            "-nostick",
            "-pa",
            "fixtures/preempt",
            "-eval",
            &format!(
                "{{module, prim_tty}} = code:load_abs(\"fixtures/prim_tty/nif/prim_tty\"), \
                 ok = prim_tty:load(\"prim_tty\"), \
                 {{module, M}} = code:ensure_loaded(exit_owned_probe), \
                 jit = erlang:system_info(emu_flavor), O = {ERTS_OPTIONS}, C = {TTY_CASES}, \
                 E = M:ttys(C, O), [0] = lists:usort([G || {{_, G}} <- M:tty_gcs(C, O)]), \
                 erlang:display(E), halt()."
            ),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{TTYS}\r\n")
    );
}
