#!/usr/bin/env bash
# Checks that commit subjects follow Conventional Commits
# (docs/08-coding-standard.md#commit-messages).
#
#   check-commit-msg.sh FILE          check the message in FILE (commit-msg hook)
#   check-commit-msg.sh --range A..B  check every non-merge commit in A..B (CI)
set -euo pipefail

TYPES="feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert"
PATTERN="^($TYPES)(\([a-z0-9][a-z0-9._/-]*\))?!?: [^ ].*"

# Subjects git or tools generate that are allowed as they are.
allowed() {
  [[ "$1" =~ ^(Merge |Revert \"|fixup! |squash! |amend! ) ]]
}

check_subject() {
  local subject="$1" where="$2"
  if allowed "$subject" || [[ "$subject" =~ $PATTERN ]]; then
    return 0
  fi
  echo "error: $where: not a Conventional Commit subject:" >&2
  echo "  $subject" >&2
  return 1
}

usage() {
  cat >&2 <<EOF
Expected: <type>(<scope>)?!?: <summary>
  type:  ${TYPES//|/, }
  scope: lowercase, e.g. engine, hook, ffi, app, ime, ci (optional)
  !:     breaking change
Example: feat(hook): add usage statistics per app
EOF
}

if [[ "${1:-}" == "--range" ]]; then
  range="${2:?usage: check-commit-msg.sh --range A..B}"
  failed=0
  while IFS=$'\t' read -r sha subject; do
    check_subject "$subject" "commit ${sha:0:7}" || failed=1
  done < <(git log --no-merges --format=$'%H\t%s' "$range")
  if [[ $failed -ne 0 ]]; then
    usage
    exit 1
  fi
  exit 0
fi

file="${1:?usage: check-commit-msg.sh FILE | --range A..B}"
# First line that is not a comment (git strips `#` lines after the hook).
subject="$(grep -v '^#' "$file" | sed -n '1p')"
if ! check_subject "$subject" "commit message"; then
  usage
  exit 1
fi
