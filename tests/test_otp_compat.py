import contextlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from scripts import otp_compat as runner


FIRST = "example_SUITE:first"
SECOND = "example_SUITE:second"


class BaselineTests(unittest.TestCase):
    def check(self, base_files, cases, known):
        outputs = ["tree\n", "\n".join(str(path) for path in base_files), *base_files.values()]
        replies = [subprocess.CompletedProcess([], 0, output, "") for output in outputs]
        with patch.object(runner, "run", side_effect=replies):
            runner.check_baseline("base", cases, known)

    def test_first_baseline_must_be_empty(self):
        self.check({}, {FIRST}, set())
        with self.assertRaisesRegex(ValueError, "initial known-failure baseline must be empty"):
            self.check({}, {FIRST}, {FIRST})

    def test_partial_baseline_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "incomplete OTP compatibility baseline"):
            self.check({runner.CASES: FIRST}, {FIRST}, set())

    def test_baseline_can_expand_selection_and_shrink_failures(self):
        self.check({runner.CASES: FIRST, runner.KNOWN_FAILURES: FIRST}, {FIRST, SECOND}, set())

    def test_baseline_rejects_new_failures_and_removed_cases(self):
        base = {runner.CASES: FIRST, runner.KNOWN_FAILURES: ""}
        with self.assertRaisesRegex(ValueError, "added known failures"):
            self.check(base, {FIRST, SECOND}, {SECOND})
        with self.assertRaisesRegex(ValueError, "removed selected cases"):
            self.check(base, {SECOND}, set())

    def test_baseline_rejects_replacing_a_known_failure(self):
        base = {runner.CASES: f"{FIRST}\n{SECOND}\n", runner.KNOWN_FAILURES: FIRST}
        with self.assertRaisesRegex(ValueError, "added known failures"):
            self.check(base, {FIRST, SECOND}, {SECOND})

    def test_manifest_rejects_invalid_or_duplicate_ids(self):
        for value in ("invalid", f"{FIRST}\n{FIRST}"):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "invalid or duplicate"):
                runner.read_ids(runner.CASES, value)


class RunTests(unittest.TestCase):
    def test_runtime_requires_one_explicit_result(self):
        prefix = runner.RESULT
        for output in ("runtime stopped", prefix + "bad", f"{prefix}{{return,ok}}\n" * 2):
            with self.subTest(output=output), patch.object(
                runner, "run", return_value=subprocess.CompletedProcess([], 0, output, ""),
            ), self.assertRaisesRegex(ValueError, "missing or malformed case result"):
                runner.execute(["pbeam"], "fixtures", FIRST, "PBeam", {})

    def test_success_marker_does_not_hide_a_process_failure(self):
        for outcome in (
            subprocess.CompletedProcess([], -9, runner.RESULT + "{return,ok}", ""),
            subprocess.TimeoutExpired([], runner.TIMEOUT),
        ):
            with self.subTest(outcome=outcome), self.assertRaises(ValueError):
                if isinstance(outcome, subprocess.TimeoutExpired):
                    with patch.object(runner.subprocess, "run", side_effect=outcome):
                        runner.run(["pbeam"], "case")
                else:
                    with patch.object(runner.subprocess, "run", return_value=outcome):
                        runner.run(["pbeam"], "case")


class CommandTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        (self.root / runner.BEAM).mkdir(parents=True)
        (self.root / runner.BEAM / "example_SUITE.beam").touch()
        (self.root / runner.CASES).write_text(f"{FIRST}\n{SECOND}\n")
        self.baseline = self.root / runner.KNOWN_FAILURES
        self.baseline.write_text(f"{FIRST}\n")

    def command(self, mode, pbeam_results):
        def execute(_command, _directory, case, runtime, _env):
            if runtime == "ERTS":
                return "{return,127}" if case == SECOND else "{return,ok}"
            output = pbeam_results[case]
            if isinstance(output, ValueError):
                raise output
            return output

        with (
            patch.object(runner, "ROOT", self.root),
            patch.object(sys, "argv", ["otp_compat.py", mode]),
            patch.object(runner, "runtime_commands", return_value=(["erl"], ["pbeam"])),
            patch.object(runner, "execute", side_effect=execute),
            contextlib.redirect_stdout(io.StringIO()),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            return runner.main()

    def test_check_excludes_known_case_and_requires_exact_normal_return(self):
        self.assertEqual(self.command("check", {SECOND: "{return,127}"}), 0)
        self.assertEqual(self.command("check", {SECOND: "{return,ok}"}), 1)
        self.assertEqual(self.command("check", {SECOND: "{skip,unsupported}"}), 1)
        self.assertEqual(self.command("check", {SECOND: "{fail,unsupported}"}), 1)
        self.assertEqual(self.baseline.read_text(), f"{FIRST}\n")

    def test_update_runs_excluded_cases_and_removes_fixed_failures(self):
        self.assertEqual(self.command("update", {FIRST: "{exception,{error,badarg}}", SECOND: "{return,127}"}), 0)
        self.assertEqual(self.baseline.read_text(), f"{FIRST}\n")
        self.assertEqual(self.command("update", {FIRST: "{return,ok}", SECOND: "{return,127}"}), 0)
        self.assertEqual(self.baseline.read_text(), "")

    def test_update_preserves_baseline_on_new_failure_or_harness_failure(self):
        self.assertEqual(self.command("update", {FIRST: "{return,ok}", SECOND: "{skip,unsupported}"}), 1)
        self.assertEqual(self.baseline.read_text(), f"{FIRST}\n")
        self.assertEqual(self.command("update", {FIRST: ValueError("no result"), SECOND: "{return,127}"}), 1)
        self.assertEqual(self.baseline.read_text(), f"{FIRST}\n")


if __name__ == "__main__":
    unittest.main()
