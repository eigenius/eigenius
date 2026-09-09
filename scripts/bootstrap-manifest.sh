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
#
# Print the bootstrap seed manifest for the CURRENT working tree — the same
# `name:sha256(source bytes)` lines `kernel/src/bootstrap/mod.rs::current_manifest()`
# computes, in the same order.
#
# A persisted store records this manifest at seed time and refuses to boot against a
# binary whose manifest differs ("seed manifest drift"). The hashes are over RAW SOURCE
# BYTES, so a comment, a trailing newline, or CRLF line endings all change them — being
# on the same git commit is NOT sufficient if the bytes on disk differ.
#
# Usage:
#   scripts/bootstrap-manifest.sh                 # this tree's manifest
#   scripts/bootstrap-manifest.sh --diff <file>   # compare against a kernel log's `stored:` list
#
# To get the stored side:  docker logs eigenius-kernel-1 2>&1 | grep -A40 'stored'

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# The manifest comes from the kernel, not from this script.
#
# This file used to keep its own copy of the layer list and hash each file with `sha256sum`.
# Both halves rotted. The list drifted — it lost `prov` and `encoding`, kept a
# `lean-expressions` the chain does not load, and still called `justification` by its old name
# `reasoning`. And the hashing was left behind by eigenius#213, which replaced raw-byte hashing
# with a presentation-insensitive form: JSON is parsed and canonicalised per resource, ESL is
# reduced to its token kinds, so that reindenting a file or rewording a comment no longer forces
# a ~40-minute reseed. `sha256sum` cannot produce that, so every line this script printed was
# wrong for every layer — silently, in the one diagnostic it exists to serve.
#
# Reproducing either half here would only re-rot. `eigenius db manifest` calls
# `bootstrap::current_manifest()` directly; this script now runs it and keeps the `--diff` view.
# BUILD, do not reuse. This tool answers "what is THIS tree's manifest", so a stale binary
# gives a stale answer — and once the subcommand exists everywhere, it gives it silently, which
# is the failure this rewrite exists to remove. `cargo run` is the correct default even though
# it is slower; set EIGENIUS_BIN to skip it when you know the binary is current.
manifest() {
  if [[ -n "${EIGENIUS_BIN:-}" ]]; then
    "$EIGENIUS_BIN" db manifest
  else
    cargo run -q --manifest-path "$ROOT/Cargo.toml" -p eigenius-cli -- db manifest
  fi
}

if [[ "${1:-}" == "--diff" ]]; then
  [[ -n "${2:-}" ]] || { echo "usage: $0 --diff <file-with-stored-manifest>" >&2; exit 2; }
  echo "layer                      stored (that store)   current (this tree)"
  drift=0
  while IFS= read -r line; do
    name="${line%%:*}"; cur="${line#*:}"
    stored="$(grep -oE "^${name}:[0-9a-f]{64}" "$2" 2>/dev/null | head -1 | cut -d: -f2 || true)"
    [[ -z "$stored" ]] && stored="(absent)"
    mark=" "; [[ "$stored" != "$cur" ]] && { mark="✗"; drift=1; }
    printf "%s %-24s %-21s %s\n" "$mark" "$name" "${stored:0:16}" "${cur:0:16}"
  done < <(manifest)
  [[ $drift == 0 ]] && echo && echo "identical — this tree can boot that store."
  exit $drift
fi

manifest
