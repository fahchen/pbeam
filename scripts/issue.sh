#!/usr/bin/env bash
# Print the title, labels, milestone, and named body sections of an issue.
set -euo pipefail

number=$1
shift
if ! [[ $number =~ ^[1-9][0-9]*$ ]]; then
  echo "issue: $number must be an issue number, such as 105" >&2
  exit 1
fi

# Read the issue before awk, so that a gh error stops the script before awk prints `no section` lines.
issue=$(gh issue view "$number" --json number,state,title,labels,milestone,body --jq '
  "#\(.number) \(.state) \(.title)",
  "labels: \([.labels[].name] | join(", ") | if . == "" then "none" else . end); milestone: \(.milestone.title // "none")",
  .body')

# The GitHub web editor keeps CR characters. The section names come in ARGV, because macOS awk rejects newlines in -v values.
# Headings are `#` and `##` lines outside code fences; a `###` heading stays in its section.
# As in CommonMark, a fence has 0-3 spaces of indent. Only a fence of the same character, at least the same length, and no info string closes it.
tr -d '\r' <<<"$issue" | awk '
  BEGIN {
    for (i = 1; i < ARGC; i++) { name[i] = ARGV[i]; want[i] = tolower(ARGV[i]); delete ARGV[i] }
    names = ARGC - 1
  }
  /^(   |  | )?(```|~~~)/ {
    line = $0
    sub(/^ */, "", line)
    char = substr(line, 1, 1)
    for (len = 1; substr(line, len + 1, 1) == char; len++);
    rest = substr(line, len + 1)
    if (!fence && (char == "~" || rest !~ /`/)) { fence = char; fence_len = len }
    else if (fence && char == fence && len >= fence_len && rest ~ /^[ \t]*$/) fence = ""
  }
  !fence && /^[ \t]*$/ { next }
  !fence && NR > 2 && /^##? / {
    heading = $0
    sub(/^##? +/, "", heading)
    headings = headings (headings == "" ? "" : " | ") heading
    show = 0
    for (i = 1; i <= names; i++) if (index(tolower(heading), want[i]) == 1) { show = 1; found[i] = 1 }
    if (show || names == 0) print
    sections = 1
    next
  }
  !sections || show { print }
  END { for (i = 1; i <= names; i++) if (!found[i]) print "no section \"" name[i] "\"; sections: " headings }' "$@"
