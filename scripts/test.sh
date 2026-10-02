#!/usr/bin/env bash
set -euo pipefail

(cd fixtures/hello && mix test --cover --warnings-as-errors)

# Only the exact empty bootstrap is exempt; any Rust source addition enables coverage.
rust_sources=$(git ls-files --cached --others --exclude-standard -- '*.rs')
if [[ "$rust_sources" == 'src/lib.rs' ]] &&
  [[ "$(cat src/lib.rs)" == '//! Portable BEAM runtime. Implementation begins in subsequent task PRs.' ]]; then
  cargo test --locked --workspace --all-features
  echo 'Line coverage: N/A (empty bootstrap; no executable Rust code).'
else
  cargo llvm-cov --locked --workspace --all-features \
    --cobertura --output-path target/coverage.xml
  python3 -c '
import sys
import xml.etree.ElementTree as ET

coverage = ET.parse(sys.argv[1]).getroot()
covered = int(coverage.attrib["lines-covered"])
total = int(coverage.attrib["lines-valid"])
print("Application line coverage: {}/{} ({:.2f}%)".format(
    covered, total, float(coverage.attrib["line-rate"]) * 100
))
sys.exit(0 if total > 0 and covered == total else 1)
' target/coverage.xml
fi
