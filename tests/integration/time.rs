#[path = "../common/boot.rs"]
mod boot;
#[path = "../common/mod.rs"]
mod common;

use std::process::{Command, Output, Stdio};

use boot::{BOOT_MODULES, boot_beam, otp_root};
use common::{pinned_beam, pinned_beam_path};
use pbeam_runtime::{BeamProgram, Term};

const TIME_PROBE: &[u8] = include_bytes!("../../fixtures/time/time_probe.beam");

/// Run `time_probe:Function()` on the pinned ERTS and return its `~w` output.
fn reference(function: &str) -> Vec<u8> {
    reference_with(&[], function)
}

/// Run `time_probe:Function()` on the pinned ERTS with the emulator flags
/// `flags` and return its printed result.
fn reference_with(flags: &[&str], function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args(flags)
        .args([
            "-noshell",
            "-pa",
            "fixtures/time",
            "-eval",
            &format!("io:format(\"~w~n\", [time_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

/// Load the pinned `erlang.beam`, which runs the `erlang:convert_time_unit/3`
/// trap, and the probe.
fn program() -> BeamProgram {
    let erlang = pinned_beam("erlang");
    let erts_internal = pinned_beam("erts_internal");
    BeamProgram::load(&[&erlang, &erts_internal, TIME_PROBE]).unwrap()
}

#[test]
fn reads_the_monotonic_and_the_system_clock_as_pinned_erts() {
    // `monotonic/0` checks that `erlang:monotonic_time(native)` starts at
    // -576460752000000000 (`erl_time.h:318-321`, `331-340`), and that each
    // unit gives the floor of a `native` value read before and after it
    // (`erl_time_sup.c:2268-2350`). The unit 3 traps to
    // `erlang:convert_time_unit/3`. `system/0` checks the same relations
    // for `os:system_time/1` on the host `CLOCK_REALTIME`
    // (`erl_time_sup.c:2595-2599`, `sys_time.c:469-497`).
    let program = program();
    let truths = Term::list_from(vec![Term::Atom(program.atom("true").unwrap()); 16]);
    for function in ["monotonic", "system"] {
        assert_eq!(
            program.execute("time_probe", function, &[]).unwrap().term,
            truths
        );
        assert_eq!(
            reference(function),
            b"[true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true]\n"
        );
    }
}

#[test]
fn raises_badarg_for_a_unit_that_is_not_valid_as_pinned_erts() {
    // Zero, a negative integer, and each term that is not a time unit or a
    // positive integer raise `badarg` (`erl_time_sup.c:2328-2341`).
    let program = program();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badargs = Term::list_from(vec![Term::tuple(vec![atom("error"), atom("badarg")]); 10]);
    assert_eq!(
        program.execute("time_probe", "units", &[]).unwrap().term,
        Term::list_from(vec![badargs.clone(), badargs])
    );
    let badargs = ["{error,badarg}"; 10].join(",");
    assert_eq!(
        reference("units"),
        format!("[[{badargs}],[{badargs}]]\n").into_bytes()
    );
}

#[test]
fn converts_the_time_unit_in_the_pinned_erlang_beam() {
    // `erts_internal:perf_counter_unit/0` gives `ERTS_HRTIME_UNIT`
    // (`erl_time_sup.c:2607-2610`) on each current host, also on x86_64
    // (row "Clocks" in `docs/erts-differences.md`), so the ERTS reference
    // is the same on each host. `erlang:convert_time_unit/3` runs
    // `gc_bif` `erlang:'*'/2`, `erlang:'-'/2`, and `erlang:'div'/2`
    // (`erlang.erl:5299-5340`). It rounds a negative time down.
    let program = program();
    assert_eq!(
        program.execute("time_probe", "convert", &[]).unwrap().term,
        Term::list_from(
            [
                1_000_000_000,
                -1_729_382_256,
                -1_729_382_256,
                5_373_665_196,
                1,
                -1,
                7,
                -8,
                -9,
            ]
            .map(Term::Integer)
        )
    );
    assert_eq!(
        reference("convert"),
        b"[1000000000,-1729382256,-1729382256,5373665196,1,-1,7,-8,-9]\n"
    );
}

#[test]
fn converts_the_perf_counter_time_through_a_bignum_product() {
    // `erlang:monotonic_time(perf_counter)` traps to
    // `erlang:convert_time_unit(Time, 1000000000, perf_counter)`, which
    // multiplies 1000000000 and the time (`erlang.erl:5333-5335`). The
    // product is a bignum, and the quotient is the time again.
    let program = program();
    let Term::Integer(time) = program
        .execute("time_probe", "perf_counter", &[])
        .unwrap()
        .term
    else {
        panic!("perf_counter gives an integer");
    };
    // The time is the `native` monotonic time, at most 10 s after the start.
    let first_seconds = -576_460_752_000_000_000..-576_460_742_000_000_000;
    assert!(first_seconds.contains(&time), "{time}");
    // ERTS returns the time in the `perf_counter` unit, 1000000000.
    let erts = reference("perf_counter");
    let erts: i64 = std::str::from_utf8(&erts)
        .unwrap()
        .trim_end()
        .parse()
        .unwrap();
    assert!(first_seconds.contains(&erts), "{erts}");
}

#[test]
fn gives_the_date_of_a_posix_time_as_pinned_erts() {
    // `erlang:posixtime_to_universaltime/1` gives the date of each 64-bit
    // integer with the C integer arithmetic of `seconds_to_univ`
    // (`erl_time_sup.c:1669-1702`), and raises `badarg` for each other term
    // (`bif.c:4025-4050`).
    let program = program();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let date = |[year, month, day, hour, minute, second]: [i64; 6]| {
        Term::tuple(vec![
            Term::tuple([year, month, day].map(Term::Integer).to_vec()),
            Term::tuple([hour, minute, second].map(Term::Integer).to_vec()),
        ])
    };
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    let mut expected: Vec<Term> = [
        [1970, 1, 1, 0, 0, 0],
        [1969, 12, 31, 23, 59, 59],
        [1970, 1, 1, 23, 59, 59],
        [1970, 1, 2, 0, 0, 0],
        [2000, 2, 29, 0, 0, 0],
        [2000, 3, 1, 0, 0, 0],
        [2100, 1, 1, 0, 0, 0],
        [1900, 1, 1, 0, 0, 0],
        [-1, 12, 31, 0, 0, 0],
        [-1, 12, 30, 23, 59, 59],
        [18_267_316_009, 3, 8, 6, 58, 8],
        [-18_267_312_070, 10, 25, 17, 1, 51],
        [292_277_026_596, 12, 4, 15, 30, 7],
        [-292_277_022_657, 1, 26, 8, 29, 52],
    ]
    .into_iter()
    .map(date)
    .collect();
    expected.extend([badarg.clone(), badarg.clone(), badarg]);
    assert_eq!(
        program.execute("time_probe", "posix", &[]).unwrap().term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference("posix"),
        b"[{{1970,1,1},{0,0,0}},{{1969,12,31},{23,59,59}},{{1970,1,1},{23,59,59}},\
{{1970,1,2},{0,0,0}},{{2000,2,29},{0,0,0}},{{2000,3,1},{0,0,0}},{{2100,1,1},{0,0,0}},\
{{1900,1,1},{0,0,0}},{{-1,12,31},{0,0,0}},{{-1,12,30},{23,59,59}},\
{{18267316009,3,8},{6,58,8}},{{-18267312070,10,25},{17,1,51}},\
{{292277026596,12,4},{15,30,7}},{{-292277022657,1,26},{8,29,52}},\
{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn raises_badarg_for_a_date_that_is_not_valid_as_pinned_erts() {
    // `erlang:universaltime_to_localtime/1` raises `badarg` for a term that
    // is not `{{Y,M,D},{H,Mi,S}}` with small integers (`time_to_parts`,
    // `bif.c:3902-3931`) and for a value out of range (`univ_to_local`,
    // `erl_time_sup.c:1808-1818`).
    let program = program();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("time_probe", "local_badarg", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::tuple(vec![atom("error"), atom("badarg")]); 23])
    );
    assert_eq!(
        reference("local_badarg"),
        format!("[{}]\n", ["{error,badarg}"; 23].join(",")).into_bytes()
    );
}

/// Run `time_probe:Function()` with the time zone `tz` in `erl -noshell`
/// and in the `pbeam` binary, as child processes. Only each child gets
/// `TZ`, so the environment of the test process does not change.
fn run_with_time_zone(function: &str, tz: &str) -> [(&'static str, Output); 2] {
    let beams = BOOT_MODULES.map(pinned_beam_path);
    let mut erts = Command::new("erl");
    erts.args([
        "-noshell",
        "-pa",
        "fixtures/time",
        "-eval",
        &format!("time_probe:{function}(), halt()."),
    ]);
    let mut pbeam = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    pbeam
        .args([
            "pbeam_boot",
            "start",
            "time_probe",
            function,
            "--root",
            otp_root().to_str().unwrap(),
            "--",
        ])
        .arg(boot_beam())
        .arg("fixtures/time/time_probe.beam")
        .args(&beams);
    [("ERTS", erts), ("PBeam", pbeam)].map(|(name, mut command)| {
        let output = command
            .env("TZ", tz)
            .env("TERM", "dumb")
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (name, output)
    })
}

/// Assert that a child of [`run_with_time_zone`] printed `stdout`, wrote no
/// error output, and exited with status 0.
fn assert_printed(name: &str, output: &Output, stdout: &[u8]) {
    assert_eq!(
        (
            output.stdout.as_slice(),
            output.stderr.as_slice(),
            output.status.code()
        ),
        (stdout, &b""[..], Some(0)),
        "{name}"
    );
}

#[test]
fn gives_the_local_time_of_the_host_time_zone_as_pinned_erts() {
    // `erlang:universaltime_to_localtime/1` calls `tzset` and `localtime_r`
    // (`sys/unix/erl_unix_sys.h:331-343`), so it reads `TZ`. The POSIX rule
    // does not depend on the tzdata of the host. `local_check/0` prints
    // `ok` when the 11 dates, which include both changes of daylight saving
    // time in 2026, give the pinned ERTS results of this rule. UTC gives
    // other results.
    for (tz, stdout) in [
        ("EST5EDT,M3.2.0,M11.1.0", &b"ok\n"[..]),
        ("UTC", &b"mismatch\n"[..]),
    ] {
        for (name, output) in run_with_time_zone("local_check", tz) {
            assert_printed(&format!("{name} {tz}"), &output, stdout);
        }
    }
}

#[test]
fn adds_the_leap_seconds_of_the_time_zone_as_the_host_erts() {
    // `fixtures/time/leap_utc` is UTC with the leap seconds of the tz
    // database (`fixtures/time/leap_utc.zone`). ERTS on macOS calls
    // `posix2time` before `localtime_r` (`HAVE_POSIX2TIME`,
    // `erl_time_sup.c:1822-1837`), so the leap seconds cancel and the local
    // time is the UTC time. A glibc host has no `posix2time`, so the local
    // time is 27 leap seconds earlier.
    let tz = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/time/leap_utc");
    let stdout: &[u8] = if cfg!(target_os = "macos") {
        b"posix2time\n"
    } else {
        b"localtime_r\n"
    };
    for (name, output) in run_with_time_zone("leap_check", tz) {
        assert_printed(name, &output, stdout);
    }
    // `erlang:localtime/0` reads the local time with `sys_localtime_r`
    // (`get_localtime`, `erl_time_sup.c:1543-1557`), which adds no leap
    // second (`sys/unix/erl_unix_sys.h:331-343`): the C libraries of the
    // hosts read the leap seconds of that zone differently, so the local
    // time is 27 seconds before the UTC time on macOS and the UTC time on
    // glibc. The check is the value of the ERTS of the same host, and the
    // ERTS run itself must give `leap` or `same`.
    let [(erts_name, erts), (name, pbeam)] = run_with_time_zone("leap_local", tz);
    assert_eq!(erts_name, "ERTS");
    assert_eq!(
        (erts.status.code(), &erts.stderr[..]),
        (Some(0), &b""[..]),
        "ERTS {tz}"
    );
    assert_ne!(erts.stdout, b"other\n", "ERTS {tz}");
    assert_eq!(
        (pbeam.status.code(), &pbeam.stderr[..]),
        (Some(0), &b""[..]),
        "{name} {tz}"
    );
    if cfg!(target_os = "macos") {
        // `posix2time` exists only on macOS, so only there does the path of
        // `local_time` add the leap seconds and differ from the path of
        // `local_zone_time`: PBeam gives the value of the ERTS of the host.
        assert_printed(name, &pbeam, &erts.stdout);
    } else {
        // The glibc `gmtime_r` subtracts the leap correction and PBeam
        // does not (row "Clocks" of `docs/erts-differences.md`), so the
        // two values can differ there where the two paths of the local
        // clock cannot. Each run must give a category of the leap handling.
        assert!(
            matches!(&pbeam.stdout[..], b"leap\n" | b"same\n"),
            "{name} {tz}: {pbeam:?}"
        );
    }
}

#[test]
fn disables_the_scheduler_wall_time_as_pinned_erts() {
    // `kernel_refc:init/1` calls `erts_internal:scheduler_wall_time(false)`
    // (`kernel_refc.erl:52-54`, `141-142`). Each term other than `true`
    // disables the scheduler wall time and gives the old state `false`
    // (`bif.c:5306-5315`). PBeam has no scheduler wall time, so `true`
    // gives a runtime error.
    let program = program();
    let falses = Term::list_from(vec![Term::Atom(program.atom("false").unwrap()); 4]);
    assert_eq!(
        program
            .execute("time_probe", "wall_time", &[])
            .unwrap()
            .term,
        falses
    );
    assert_eq!(reference("wall_time"), b"[false,false,false,false]\n");
    // The call traps to a receive of the reply of each scheduler
    // (`bif.c:5313-5314`, `erts_internal.erl:759-774`), so the spawned
    // process sends `ping` before the call returns.
    let order = Term::list_from(vec![
        Term::Atom(program.atom("false").unwrap()),
        Term::Atom(program.atom("ping").unwrap()),
    ]);
    assert_eq!(
        program
            .execute("time_probe", "wall_time_order", &[])
            .unwrap()
            .term,
        order
    );
    assert_eq!(reference("wall_time_order"), b"[false,ping]\n");
    // The BIF sends one reference to the caller for the one scheduler and
    // traps to `erts_internal:await_sched_wall_time_modifications/2`, which
    // receives it (`erl_process.c:1382-1421`, `erts_internal.erl:756-774`).
    // The reductions between the two `process_info/2` reads are those of
    // the pinned ERTS with one scheduler.
    let reds = reference_with(&["+S", "1"], "wall_time_reds");
    assert_eq!(reds, b"12\n");
    assert_eq!(
        program
            .execute("time_probe", "wall_time_reds", &[])
            .unwrap()
            .term,
        Term::Integer(12)
    );
    let error = program
        .execute("time_probe", "wall_time_on", &[])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unsupported erts_internal:scheduler_wall_time(true)"),
        "{error}"
    );
}

/// Load the pinned `os.beam`, which runs `os:perf_counter/1`, and the
/// probe.
fn os_program() -> BeamProgram {
    let os = pinned_beam("os");
    let erlang = pinned_beam("erlang");
    let erts_internal = pinned_beam("erts_internal");
    BeamProgram::load(&[&os, &erlang, &erts_internal, TIME_PROBE]).unwrap()
}

#[test]
fn gives_the_clock_of_the_default_arity_bifs_as_pinned_erts() {
    // `clock/0` checks the relations of `erlang:system_time/0`,
    // `erlang:monotonic_time/0`, `erlang:time_offset/0`, and
    // `os:system_time/0` of the pinned ERTS (`erl_time_sup.c:2500-2552`,
    // `2589-2593`). A value of the live clock is compared as its range, its
    // unit, and its relation to the other calls, and the ERTS run of the
    // same function gives the same checks.
    let program = os_program();
    let truths = Term::list_from(vec![Term::Atom(program.atom("true").unwrap()); 20]);
    assert_eq!(
        program.execute("time_probe", "clock", &[]).unwrap().term,
        truths
    );
    assert_eq!(
        reference("clock"),
        format!("[{}]\n", ["true"; 20].join(",")).into_bytes()
    );
}

#[test]
fn runs_the_timestamp_bifs_as_pinned_erts() {
    // `timestamp/0` checks the parts of `erlang:timestamp/0`
    // (`erl_time_sup.c:2555-2587`) and `os:timestamp/0`
    // (`erl_bif_os.c:48-59`) and their relation to the system time in
    // microseconds within 10 ms.
    let program = os_program();
    let truths = Term::list_from(vec![Term::Atom(program.atom("true").unwrap()); 10]);
    assert_eq!(
        program
            .execute("time_probe", "timestamp", &[])
            .unwrap()
            .term,
        truths
    );
    assert_eq!(
        reference("timestamp"),
        format!("[{}]\n", ["true"; 10].join(",")).into_bytes()
    );
}

#[test]
fn raises_badarg_for_a_time_offset_unit_that_is_not_valid_as_pinned_erts() {
    // Zero, a negative integer, an atom, and a tuple raise `badarg`
    // (`erl_time_sup.c:2549-2552`, `2268-2350`).
    let program = os_program();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("time_probe", "time_offset_units", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::tuple(vec![atom("error"), atom("badarg")]); 4])
    );
    assert_eq!(
        reference("time_offset_units"),
        format!("[{}]\n", ["{error,badarg}"; 4].join(",")).into_bytes()
    );
}

#[test]
fn gives_the_perf_counter_of_the_host_as_pinned_erts() {
    // `perf()` checks that `os:perf_counter/0` is non-negative, that it
    // grows across a busy loop, and that `os:perf_counter/1` gives its
    // floor in seconds (`erl_time_sup.c:2601-2605`,
    // `lib/kernel/src/os.erl:188-191`). The counter counts from the boot of
    // the host on ERTS and from the first call of the run on PBeam, a base
    // that ERTS documents as the same only within a runtime restart, so
    // only the relations are compared.
    let program = os_program();
    let truths = Term::list_from(vec![Term::Atom(program.atom("true").unwrap()); 3]);
    assert_eq!(
        program.execute("time_probe", "perf", &[]).unwrap().term,
        truths
    );
    assert_eq!(reference("perf"), b"[true,true,true]\n");
}

#[test]
fn gives_the_host_pid_as_pinned_erts() {
    // `os:getpid/0` gives the decimal digits of the host process identifier
    // (`erl_bif_os.c:60-69`, `sys/unix/sys.c:934-939`). The probe runs in
    // the test process, so the value is the identifier of that process.
    let program = os_program();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("time_probe", "pid", &[]).unwrap().term,
        Term::list_from(vec![atom("true"); 5])
    );
    assert_eq!(reference("pid"), b"[true,true,true,true,true]\n");
    let digits = std::process::id().to_string();
    assert_eq!(
        program
            .execute("time_probe", "pid_value", &[])
            .unwrap()
            .term,
        Term::list_from(digits.bytes().map(|byte| Term::Integer(byte.into())))
    );
}

#[test]
fn changes_the_environment_copy_as_pinned_erts() {
    // `environment/0` checks `os:putenv/2`, `os:getenv/1`,
    // `os:unsetenv/1`, and `os:env/0` (`erl_bif_os.c:82-142`): the set
    // value, the empty value as `[]`, the removed variable as `false`, the
    // list of the environment, and `badarg` for a bad name or value. The
    // key is only this test's, so the change of the environment copy of the
    // run does not reach another test.
    let program = os_program();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let characters =
        |text: &str| Term::list_from(text.bytes().map(|byte| Term::Integer(byte.into())));
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    let mut expected = vec![
        Term::tuple(vec![characters("TIME_PROBE_VAR"), characters("value")]),
        Term::nil(),
        atom("false"),
        atom("false"),
        atom("true"),
    ];
    expected.extend(std::iter::repeat_n(badarg, 6));
    assert_eq!(
        program
            .execute("time_probe", "environment", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );
    // `~w` prints the characters of a string that is an element of another
    // term as integers, so the reference text holds the code points.
    let digits = |text: &str| {
        format!(
            "[{}]",
            text.bytes()
                .map(|byte| byte.to_string())
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let badargs = ["{error,badarg}"; 6].join(",");
    assert_eq!(
        reference("environment"),
        format!(
            "[{{{},{}}},[],false,false,true,{badargs}]\n",
            digits("TIME_PROBE_VAR"),
            digits("value")
        )
        .into_bytes()
    );
}

#[test]
fn gives_the_local_time_of_the_host_time_zone_as_the_default_arity_bifs() {
    // `clock_utc/0` and `clock_jst/0` compare the local clock BIFs with the
    // UTC BIFs: `erlang:localtime/0`, `erlang:date/0`, and `erlang:time/0`
    // read the host time zone (`get_localtime`, `get_date`, and `get_time`,
    // `erl_time_sup.c:1516-1557`), and `erlang:universaltime/0` reads UTC
    // (`get_universaltime`, `erl_time_sup.c:1559-1583`). The difference of
    // the two is the offset of the zone. `TZ=UTC0` gives 0 and `TZ=JST-9`
    // gives 32400, and each of the two calls fails in the other zone.
    for (tz, utc, jst) in [
        ("UTC0", &b"ok\n"[..], &b"mismatch\n"[..]),
        ("JST-9", &b"mismatch\n"[..], &b"ok\n"[..]),
    ] {
        for (name, output) in run_with_time_zone("clock_utc", tz) {
            assert_printed(&format!("{name} clock_utc {tz}"), &output, utc);
        }
        for (name, output) in run_with_time_zone("clock_jst", tz) {
            assert_printed(&format!("{name} clock_jst {tz}"), &output, jst);
        }
    }
}

#[test]
fn gives_the_posix_seconds_of_a_utc_time_as_pinned_erts() {
    // `erlang:universaltime_to_posixtime/1` accepts the years from 1600 to
    // 2147483646 (`univ_to_seconds`, `erl_time_sup.c:1704-1727`), and
    // raises `badarg` for a term that `time_to_parts` does not read
    // (`bif.c:4010-4016`).
    let program = program();
    let badarg = Term::tuple(vec![
        Term::Atom(program.atom("error").unwrap()),
        Term::Atom(program.atom("badarg").unwrap()),
    ]);
    let mut expected = [0, -1, -11_676_096_000].map(Term::Integer).to_vec();
    expected.push(badarg.clone());
    expected.extend([951_786_123, 67_767_976_201_996_799].map(Term::Integer));
    expected.extend(std::iter::repeat_n(badarg, 8));
    assert_eq!(
        program
            .execute("time_probe", "posix_seconds", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference("posix_seconds"),
        b"[0,-1,-11676096000,{error,badarg},951786123,67767976201996799,\
{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn gives_increasing_now_values_as_pinned_erts() {
    // `erlang:now/0` gives `{MegaSecs, Secs, MicroSecs}` of the system time,
    // larger than each earlier value (`bif.c:4057-4067`,
    // `erl_time_sup.c:1852-1887`).
    let program = program();
    assert_eq!(
        program.execute("time_probe", "now", &[]).unwrap().term,
        Term::list_from(vec![Term::Atom(program.atom("true").unwrap()); 5])
    );
    assert_eq!(reference("now"), b"[true,true,true,true,true]\n");
}

#[test]
fn gives_the_utc_time_of_a_local_time_as_pinned_erts() {
    // `erlang:localtime_to_universaltime/2` calls `tzset`, `mktime`,
    // `time2posix` on macOS, and `gmtime_r` (`erl_time_sup.c:1735-1795`).
    // The `mktime` rules for `tm_isdst` and for a local time that does not
    // exist or occurs twice belong to the C library, so each zone compares
    // PBeam with the ERTS of the same host. `fixtures/time/leap_utc` adds
    // leap seconds; a zone without daylight saving time uses `tm_isdst` 0
    // (`erl_time_sup.c:1764-1769`).
    let leap = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/time/leap_utc");
    for tz in [
        "EST5EDT,M3.2.0,M11.1.0",
        "America/New_York",
        "Europe/London",
        "Australia/Sydney",
        "UTC",
        leap,
    ] {
        let [(_, erts), (_, pbeam)] = run_with_time_zone("local_utc", tz);
        assert_printed(&format!("ERTS {tz}"), &erts, &erts.stdout);
        assert_printed(&format!("PBeam {tz}"), &pbeam, &erts.stdout);
    }
    // UTC has no daylight saving time, so each flag gives the same time on
    // each host.
    let [_, (_, pbeam)] = run_with_time_zone("local_utc", "UTC");
    let utc = String::from_utf8(pbeam.stdout).unwrap();
    assert!(utc.starts_with(
        "[{{2026,1,15},{7,0,0}},{{2026,1,15},{7,0,0}},{{2026,1,15},{7,0,0}},{{2026,7,15},{7,0,0}},"
    ));
    assert!(utc.ends_with("{error,badarg},{error,badarg}]\n"));
    assert_eq!(utc.matches("{error,badarg}").count(), 9 * 3 + 6);
}
