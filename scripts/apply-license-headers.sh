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

# scripts/apply-license-headers.sh
#
# Applies the Apache 2.0 license header to every Rust, TypeScript, Protobuf,
# and shell-script source file in the repository.
#
# Idempotent: files that already carry a recognisable header are skipped.
# Shebangs are preserved.
#
# Usage:
#   scripts/apply-license-headers.sh                # apply
#   scripts/apply-license-headers.sh --dry-run      # report what would change
#   scripts/apply-license-headers.sh --quiet        # suppress per-file output
#
# Environment variables:
#   COPYRIGHT_YEAR    — defaults to 2026
#   COPYRIGHT_HOLDER  — defaults to "The Eigenius Authors"

set -euo pipefail

# --- Configuration ---

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
COPYRIGHT_YEAR="${COPYRIGHT_YEAR:-2026}"
COPYRIGHT_HOLDER="${COPYRIGHT_HOLDER:-The Eigenius Authors}"

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

# Marker that indicates a header is already present in a file.
# Checked against the first 30 lines (handles SPDX-style and full ASF headers).
HEADER_MARKER='Apache License'

# --- Header text (without comment markers; prefixed per file type below) ---

read -r -d '' HEADER_TEXT <<EOF || true
Copyright ${COPYRIGHT_YEAR} ${COPYRIGHT_HOLDER}

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
EOF

# --- Comment-prefix helpers ---

# Prefix every line with "// " (and bare "//" for empty lines).
slashify() {
  awk '{ if ($0 == "") print "//"; else print "// " $0 }' <<< "$1"
}

# Prefix every line with "# " (and bare "#" for empty lines).
hashify() {
  awk '{ if ($0 == "") print "#"; else print "# " $0 }' <<< "$1"
}

# --- Per-file application ---

applied=0
skipped_existing=0
skipped_excluded=0
total_seen=0

apply_header_to() {
  local file="$1"
  total_seen=$((total_seen + 1))

  # Idempotency: skip if a header is already present.
  if head -n 30 "$file" 2>/dev/null | grep -q "$HEADER_MARKER"; then
    skipped_existing=$((skipped_existing + 1))
    if [[ "$QUIET" -eq 0 ]]; then echo "skip (already has header): ${file#${REPO_ROOT}/}"; fi
    return 0
  fi

  local ext="${file##*.}"
  local header
  case "$ext" in
    rs|ts|tsx|proto)  header=$(slashify "$HEADER_TEXT") ;;
    sh|bash)          header=$(hashify  "$HEADER_TEXT") ;;
    *)
      skipped_excluded=$((skipped_excluded + 1))
      return 0
      ;;
  esac

  # Preserve shebang (any "#!..." first line) by inserting the header below it.
  local first_line
  first_line=$(head -n 1 "$file")

  local tmp
  tmp=$(mktemp)
  trap 'rm -f "$tmp"' RETURN

  if [[ "$first_line" == \#!* ]]; then
    {
      echo "$first_line"
      echo
      echo "$header"
      echo
      tail -n +2 "$file"
    } > "$tmp"
  else
    {
      echo "$header"
      echo
      cat "$file"
    } > "$tmp"
  fi

  if [[ "$DRY_RUN" -eq 1 ]]; then
    if [[ "$QUIET" -eq 0 ]]; then echo "WOULD apply: ${file#${REPO_ROOT}/}"; fi
    rm -f "$tmp"
  else
    mv "$tmp" "$file"
    applied=$((applied + 1))
    if [[ "$QUIET" -eq 0 ]]; then echo "applied:    ${file#${REPO_ROOT}/}"; fi
  fi
  return 0
}

# --- File discovery ---

cd "$REPO_ROOT"

# Directory names pruned anywhere in the tree (name-based, not path-based,
# so that any `target/` or `node_modules/` is excluded regardless of depth).
PRUNE_DIR_NAMES=(
  "target"          # all Cargo build dirs (workspace, orchestration/runtime-substrate-native, etc.)
  "node_modules"    # JS/TS dependencies
  ".git"
  ".claude"         # Claude Code project state
  ".cache"
  "references"      # external/vendored repos
  "generated"       # generated Connect/proto stubs
  "gen"             # ditto
  "dist"            # bundler output
  ".next"           # Next.js build output (precautionary)
  ".turbo"          # Turbo cache (precautionary)
)

# Files always excluded by name (regardless of directory).
EXCLUDED_FILES=(
)

is_excluded_file() {
  local name
  name="$(basename "$1")"
  for x in "${EXCLUDED_FILES[@]:-}"; do
    [[ -n "$x" && "$name" == "$x" ]] && return 0
  done
  return 1
}

# Build name-based prune expression: ( -type d -name target -o -type d -name node_modules ... )
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

# Read prune args into an array (newline-separated -> safe word splitting).
mapfile -t PRUNE_ARGS < <(build_prune_args)

while IFS= read -r -d '' file; do
  if is_excluded_file "$file"; then
    skipped_excluded=$((skipped_excluded + 1))
    if [[ "$QUIET" -eq 0 ]]; then echo "skip (generated): ${file#${REPO_ROOT}/}"; fi
    continue
  fi
  apply_header_to "$file"
done < <(find . "${PRUNE_ARGS[@]}" -prune -o -type f \
  \( -name "*.rs" -o -name "*.ts" -o -name "*.tsx" -o -name "*.proto" \
     -o -name "*.sh" -o -name "*.bash" \) -print0)

# --- Summary ---

echo
total_files=$((total_seen + skipped_excluded))
if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "Dry run complete."
  echo "  Files seen:                      $total_files"
  echo "  Already had header (skipped):    $skipped_existing"
  echo "  Generated / excluded (skipped):  $skipped_excluded"
  echo "  Would apply header to:           $((total_seen - skipped_existing))"
else
  echo "Done."
  echo "  Files seen:                      $total_files"
  echo "  Applied:                         $applied"
  echo "  Already had header (skipped):    $skipped_existing"
  echo "  Generated / excluded (skipped):  $skipped_excluded"
fi
