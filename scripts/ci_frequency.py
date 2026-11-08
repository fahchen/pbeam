#!/usr/bin/env python3
"""Select reviewed ERTS-only checks for the full CI run."""

import json
import re
import sys
from pathlib import Path


def identities():
    rows = Path("scripts/ci_erts_only.txt").read_text().splitlines()
    cases = set()
    for row in rows:
        parts = row.split("|")
        if len(parts) != 2 or not all(re.fullmatch(r"[\w:-]+", x) for x in parts):
            raise ValueError(f"invalid case: {row}")
        cases.add(tuple(parts))
    if len(cases) != len(rows) or not cases:
        raise ValueError("the ERTS-only list is empty or has duplicates")
    inventory = {}
    for path in ("scripts/test_classes.exs", "scripts/test_integration_classes.txt"):
        for line in Path(path).read_text().splitlines():
            parts = line.strip().split("|")
            if len(parts) == 4:
                inventory[tuple(parts[:2])] = (parts[2].split(","), parts[3].split(","))
    for case in cases:
        purposes, dependencies = inventory.get(case, ([], []))
        if "erts_only" not in purposes or "pbeam_process" in dependencies or "pbeam_tool" in dependencies:
            raise ValueError(f"case is not reviewed as ERTS-only: {case}")
    return cases


def selected(path):
    suites = json.loads(Path(path).read_text())["rust-suites"]
    return {
        (binary, name)
        for binary, suite in suites.items()
        for name, case in suite["testcases"].items()
        if case["filter-match"]["status"] == "matches"
    }


def verify(full, reduced, cases):
    if full - reduced != cases or reduced - full:
        raise ValueError(f"unexpected selection: missing={sorted(full - reduced - cases)}, stale={sorted(cases - (full - reduced))}, added={sorted(reduced - full)}")


def main():
    cases = identities()
    if sys.argv[1:] == ["filter"]:
        terms = [f"(binary_id(={binary}) & test(={name}))" for binary, name in sorted(cases)]
        print("not (" + " | ".join(terms) + ")")
    elif len(sys.argv) == 4 and sys.argv[1] == "verify":
        full, reduced = selected(sys.argv[2]), selected(sys.argv[3])
        verify(full, reduced, cases)
        print(f"Frequency selection: {len(full)} full, {len(reduced)} PR, {len(cases)} ERTS-only")
    else:
        raise ValueError("usage: ci_frequency.py filter | verify FULL_LIST REDUCED_LIST")


if __name__ == "__main__":
    main()
