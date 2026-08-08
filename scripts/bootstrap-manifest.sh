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

# Order and paths mirror BOOTSTRAP_SPECS in kernel/src/bootstrap/mod.rs.
SPECS=(
  "core:ontologies/core/core-ontology.json"
  "eigentt-type-fragment:ontologies/eigentt/eigentt-type-fragment.json"
  "program:ontologies/program/program-ontology.json"
  "reflection:ontologies/reflection/reflection-ontology.json"
  "obo:ontologies/obo/obo-meta-ontology.json"
  "institution:ontologies/institution/institution-ontology.json"
  "runtime:ontologies/runtime/runtime-substrate-ontology.json"
  "formulas:ontologies/formulas/formulas-ontology.json"
  "lean-expressions:ontologies/lean/lean-expressions.eigon.json"
  "lean-runtime-classes:ontologies/lean/lean-runtime-classes.eigon.json"
  "lean-institution:ontologies/lean/lean-institution.eigon.json"
  "reasoning:ontologies/reasoning/reasoning.esl"
  "statistics:ontologies/statistics/statistics.esl"
  "notebook:ontologies/notebook/notebook-ontology.json"
  "ingest:ontologies/ingest/ingest-ontology.json"
  "reference:ontologies/reference/reference.esl"
  "logic:ontologies/logic/logic.esl"
  "lexicon:ontologies/lexicon/lexicon-ontology.esl"
  "ontology:ontologies/ontology/ontology.esl"
  "closed-class:ontologies/lexicon/closed-class.esl"
)

manifest() {
  for spec in "${SPECS[@]}"; do
    name="${spec%%:*}"; path="${spec#*:}"
    if [[ ! -f "$ROOT/$path" ]]; then
      echo "$name:<MISSING $path>"
    else
      echo "$name:$(sha256sum "$ROOT/$path" | cut -d' ' -f1)"
    fi
  done
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
