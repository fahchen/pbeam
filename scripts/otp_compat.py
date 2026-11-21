#!/usr/bin/env python3
"""Run the selected upstream OTP test cases against PBeam."""
import argparse
import fcntl
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
CASES = Path("tests/otp/cases.txt")
KNOWN_FAILURES = CASES.with_name("known_failures.txt")
BEAM = CASES.with_name("beam")
CASE_ID = re.compile(r"[a-z][a-zA-Z0-9_]*_SUITE:[a-z][a-zA-Z0-9_]*")
RESULT = "PBEAM_OTP_RESULT "
TIMEOUT = 60


def read_ids(source, content):
    ids = set()
    for number, line in enumerate(content.splitlines(), start=1):
        case = line.strip()
        if not case:
            continue
        if not CASE_ID.fullmatch(case) or case in ids:
            raise ValueError(f"{source}:{number}: invalid or duplicate case ID {case!r}")
        ids.add(case)
    return ids


def run(command, subject, env=None):
    try:
        result = subprocess.run(
            command, cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
            capture_output=True, text=True, errors="backslashreplace", timeout=TIMEOUT,
        )
    except subprocess.TimeoutExpired as error:
        raise ValueError(
            f"{subject}: timed out after {TIMEOUT}s\n"
            f"stdout: {error.stdout!r}\nstderr: {error.stderr!r}"
        ) from error
    if result.returncode:
        raise ValueError(
            f"{subject}: exited with status {result.returncode}\n"
            f"stdout: {result.stdout}\nstderr: {result.stderr}"
        )
    return result


def check_baseline(revision, cases, known):
    tree = run(
        ["git", "rev-parse", "--verify", "--end-of-options", revision + "^{tree}"],
        f"base {revision}",
    ).stdout.strip()
    paths = set(run(
        ["git", "ls-tree", "-r", "--name-only", tree, "--", str(CASES), str(KNOWN_FAILURES)],
        f"base {revision}",
    ).stdout.splitlines())

    if not paths:
        if known:
            raise ValueError("the initial known-failure baseline must be empty")
        return
    if paths != {str(CASES), str(KNOWN_FAILURES)}:
        raise ValueError(f"base {revision}: incomplete OTP compatibility baseline")

    base_cases, base_known = (
        read_ids(f"{revision}:{path}", run(
            ["git", "show", f"{tree}:{path}"], f"base {revision}:{path}",
        ).stdout)
        for path in (CASES, KNOWN_FAILURES)
    )
    if not base_cases or not base_known <= base_cases:
        raise ValueError(f"base {revision}: invalid OTP compatibility baseline")
    if base_cases - cases:
        raise ValueError("removed selected cases: " + ", ".join(sorted(base_cases - cases)))
    if known - base_known:
        raise ValueError("added known failures: " + ", ".join(sorted(known - base_known)))


def suite_paths(cases):
    directories = {}
    for case in sorted(cases):
        suite, _ = case.split(":")
        if suite not in directories:
            matches = sorted((ROOT / BEAM).rglob(f"{suite}.beam"))
            if len(matches) != 1:
                raise ValueError(f"{suite}: expected one upstream BEAM file, found {len(matches)}")
            directories[suite] = str(matches[0].parent)
    return directories


def runtime_commands(pbeam_bin, env):
    native = ["erl", "+S", "1", "-noshell"]
    probe = (
        '{ok, [[Root]]} = init:get_argument(root), '
        '{ok, [[Bin]]} = init:get_argument(bindir), '
        'io:format("~s~n~s~n~s~n~s~n", '
        '[Root, Bin, code:lib_dir(erts, ebin), erlang:system_info(otp_release)]), halt().'
    )
    output = run(native + ["-eval", probe], "pinned ERTS paths", env).stdout.splitlines()
    if len(output) != 4:
        raise ValueError(f"invalid pinned ERTS path output: {output!r}")
    root, bindir, preloaded, release = output
    pin = re.search(r'^erlang = "([^"]+)"$', (ROOT / "mise.toml").read_text(), re.MULTILINE)
    installed = (Path(root) / "releases" / release / "OTP_VERSION").read_text().strip()
    if not pin or pin[1] != installed:
        raise ValueError(f"erl on PATH is OTP {installed}; use the OTP pin in mise.toml")
    if not pbeam_bin.is_file():
        raise ValueError(f"missing {pbeam_bin}; build pbeam first")
    pbeam = [
        str(pbeam_bin), "erl", "--root", root, "--bindir", bindir,
        "--preloaded", preloaded, "--", "+S", "1", "-noshell",
    ]
    return native, pbeam


def execute(command, directory, case, runtime, env):
    suite, name = case.split(":")
    expression = (
        f"{{module, '{suite}'}} = code:ensure_loaded('{suite}'), "
        f"true = erlang:function_exported('{suite}', '{name}', 1), "
        f"Result = try '{suite}':'{name}'([]) of "
        "{skip, Why} -> {skip, Why}; "
        "{fail, Why} -> {fail, Why}; "
        "Value -> {return, Value} "
        "catch Class:Reason -> {exception, {Class, Reason}} end, "
        f'io:format("~n{RESULT}~w~n", [Result]), halt().'
    )
    subject = f"{runtime} {case}"
    outcome = run(command + ["-pa", directory, "-eval", expression], subject, env)
    records = [line[len(RESULT):] for line in outcome.stdout.splitlines() if line.startswith(RESULT)]
    if len(records) != 1 or not re.fullmatch(r"\{(return|skip|fail|exception),.+\}", records[0]):
        raise ValueError(
            f"{subject}: missing or malformed case result\n"
            f"stdout: {outcome.stdout}\nstderr: {outcome.stderr}"
        )
    return records[0]


def replace_failures(previous, failures):
    destination = ROOT / KNOWN_FAILURES
    # Lock the directory because replacing the baseline changes its inode.
    directory = os.open(destination.parent, os.O_RDONLY)
    try:
        fcntl.flock(directory, fcntl.LOCK_EX)
        if destination.read_text() != previous:
            raise ValueError(f"{KNOWN_FAILURES} changed during execution")
        descriptor, filename = tempfile.mkstemp(dir=destination.parent)
        temporary = Path(filename)
        try:
            with os.fdopen(descriptor, "w") as file:
                file.writelines(f"{case}\n" for case in sorted(failures))
            os.replace(temporary, destination)
        finally:
            temporary.unlink(missing_ok=True)
    finally:
        os.close(directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("check", "update"))
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    parser.add_argument("--pbeam", type=Path, default=target / "debug/pbeam")
    parser.add_argument("--base", help="Git revision with the required cases and known failures")
    args = parser.parse_args()
    if args.base and args.mode != "check":
        parser.error("--base requires check")

    try:
        cases = read_ids(CASES, (ROOT / CASES).read_text())
        previous = (ROOT / KNOWN_FAILURES).read_text()
        known = read_ids(KNOWN_FAILURES, previous)
        if not cases:
            raise ValueError(f"{CASES} must select at least one case")
        if known - cases:
            raise ValueError("unknown case IDs in the failure list: " + ", ".join(sorted(known - cases)))
        if args.base:
            check_baseline(args.base, cases, known)

        directories = suite_paths(cases)
        env = {
            key: value
            for key, value in os.environ.items()
            if key not in ("ERL_AFLAGS", "ERL_FLAGS", "ERL_ZFLAGS", "ERL_LIBS", "ERL_CRASH_DUMP")
            and not (key.startswith("ERL_OTP") and key.endswith("_FLAGS"))
        }
        env["ERL_CRASH_DUMP_SECONDS"] = "0"
        native, pbeam = runtime_commands(args.pbeam.resolve(), env)
        selected = cases - known if args.mode == "check" else cases
        failures = set()
        for case in sorted(selected):
            directory = directories[case.split(":")[0]]
            expected = execute(native, directory, case, "ERTS", env)
            if not expected.startswith("{return,"):
                raise ValueError(f"ERTS {case}: expected a normal return, got {expected}")
            actual = execute(pbeam, directory, case, "PBeam", env)
            if actual != expected:
                failures.add(case)
                print(f"FAIL {case}: expected {expected}, got {actual}")
            else:
                print(f"PASS {case}")

        if failures - known:
            return 1
        if args.mode == "update" and failures != known:
            replace_failures(previous, failures)
        print(f"OTP compatibility: {len(selected - failures)} passed, {len(failures)} failed, "
              f"{len(cases - selected)} excluded")
        return 0
    except (OSError, ValueError) as error:
        print(f"OTP compatibility: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
