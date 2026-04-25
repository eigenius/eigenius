#!/usr/bin/env bash

# Copyright 2026 The Eigenius Authors
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# scripts/make-scripts-executable.sh
#
# Ensures every .sh file in the repository has the executable bit set.
# Idempotent. Mirrors the directory-pruning conventions of
# scripts/apply-license-headers.sh so that vendored references, build
# artefacts, and tool state are excluded.
#
# Usage:
#   scripts/make-scripts-executable.sh             # apply chmod
#   scripts/make-scripts-executable.sh --dry-run   # report what would change
#   scripts/make-scripts-executable.sh --quiet     # suppress per-file output

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

DRY_RUN=0
QUIET=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --quiet)   QUIET=1 ;;
    -h|--help)
      sed -n '/^# scripts/,/^$/p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg" >&2
      echo "Run with --help for usage." >&2
      exit 2
      ;;
  esac
done

# Directory names pruned anywhere in the tree.
PRUNE_DIR_NAMES=(
  "target"          # all Cargo build dirs
  "node_modules"    # JS/TS dependencies
  ".git"
  ".claude"         # Claude Code project state
  ".cache"
  "references"      # external/vendored repos
  "generated"
  "gen"
  "dist"
  ".next"
  ".turbo"
)

build_prune_args() {
  local args=("(")
  local first=1
  for n in "${PRUNE_DIR_NAMES[@]}"; do
    if [[ $first -eq 1 ]]; then
      args+=("-type" "d" "-name" "$n")
      first=0
    else
      args+=("-o" "-type" "d" "-name" "$n")
    fi
  done
  args+=(")")
  printf '%s\n' "${args[@]}"
}

mapfile -t PRUNE_ARGS < <(build_prune_args)

cd "$REPO_ROOT"

already_exec=0
made_exec=0
total=0

while IFS= read -r -d '' file; do
  total=$((total + 1))
  if [[ -x "$file" ]]; then
    already_exec=$((already_exec + 1))
    if [[ "$QUIET" -eq 0 ]]; then echo "exec already: ${file#${REPO_ROOT}/}"; fi
  else
    if [[ "$DRY_RUN" -eq 1 ]]; then
      if [[ "$QUIET" -eq 0 ]]; then echo "WOULD chmod:  ${file#${REPO_ROOT}/}"; fi
    else
      chmod +x "$file"
      made_exec=$((made_exec + 1))
      if [[ "$QUIET" -eq 0 ]]; then echo "chmod +x:     ${file#${REPO_ROOT}/}"; fi
    fi
  fi
done < <(find . "${PRUNE_ARGS[@]}" -prune -o -type f -name "*.sh" -print0)

echo
if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "Dry run complete."
  echo "  .sh files seen:                $total"
  echo "  Already executable:            $already_exec"
  echo "  Would chmod +x:                $((total - already_exec))"
else
  echo "Done."
  echo "  .sh files seen:                $total"
  echo "  Already executable:            $already_exec"
  echo "  Newly executable (chmod +x):   $made_exec"
fi
