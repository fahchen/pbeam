#!/usr/bin/env python3
"""Keep timing data without test output or failure messages."""

import json
import sys
import xml.etree.ElementTree as ET
from pathlib import Path


def main() -> None:
    source, destination = map(Path, sys.argv[1:])
    if not source.exists():
        print("No nextest timing report was produced.")
        return

    root = ET.parse(source).getroot()
    cases = []
    for case in root.iter("testcase"):
        outcome = next((name for name in ("failure", "error", "skipped") if case.find(name) is not None), "passed")
        cases.append({
            "binary": case.attrib["classname"],
            "test": case.attrib["name"],
            "seconds": float(case.attrib["time"]),
            "outcome": outcome,
        })
    destination.write_text(json.dumps(cases, indent=2) + "\n")
    print(f"Saved durations for {len(cases)} tests.")


if __name__ == "__main__":
    main()
