"""Check that the frequency gate rejects a changed test selection."""

import importlib.util
import sys
import unittest
from pathlib import Path


sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("ci_frequency", "scripts/ci_frequency.py")
frequency = importlib.util.module_from_spec(spec)
spec.loader.exec_module(frequency)


class FrequencyTest(unittest.TestCase):
    def test_reviewed_cases_have_erts_only_facets(self):
        self.assertEqual(len(frequency.identities()), len(Path("scripts/ci_erts_only.txt").read_text().splitlines()))

    def test_selection_requires_exact_difference(self):
        retained = ("pbeam-runtime::a", "retained")
        deferred = ("pbeam-runtime::a", "deferred")
        other = ("pbeam-runtime::b", "other")
        frequency.verify({retained, deferred}, {retained}, {deferred})
        for reduced in ({retained, deferred}, set(), {retained, other}):
            with self.subTest(reduced=reduced), self.assertRaises(ValueError):
                frequency.verify({retained, deferred}, reduced, {deferred})


if __name__ == "__main__":
    unittest.main()
