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

# Measure a SLASH-MODE experiment as a PATCH LAYER, without a reseed.
#
# A slash mode is denotation-transparent (D63 multimodal slashes): it restricts which combinatory
# rules may consume a slash, never what the entry denotes. So a mode experiment adds and removes no
# entries — it re-emits existing resources with a different `lexicon:cat`, which SHADOWS the base by
# parent-chain resolution. The base snapshot is never rebuilt.
#
# Why this exists: on 2026-08-01 three mode experiments were each run as a full
# reseed + align + measure (~40 min apiece, of which UMLS re-import is ~30) to test changes that
# touch NO UMLS entry — UMLS emits only `cat_n`, which has no slash, as does the WordNet↔UMLS
# aligner. `experiments/parsing/probes/frame2223-whole-lexicon.esl` already documented the
# technique; it just was not applied. This is that loop, made a command.
#
# NOTE the LATTICE itself (declaring `lexicon:Mode`, changing `fwd`/`bwd` arity in
# `ontologies/lexicon/lexicon-ontology.esl`) is a BOOTSTRAP edit and genuinely does need a reseed —
# the old store cannot be resumed (ManifestDrift, fail-closed). Only mode ASSIGNMENTS are patchable.
# A closed-class entry is patchable the same way: shadow it, do not edit the bootstrap file.
#
# Usage:
#   scripts/probe-mode-layer.sh --base <aligned-snapshot> --layer <patch.esl> [--layer <more.esl>]
#                               [--name <probe-name>] [--ranks <ranks.json>]
#
# Env:
#   SNAPSHOT_ROOT  where probe snapshots are written (default: ../db-snapshot)

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SNAPSHOT_ROOT="${SNAPSHOT_ROOT:-$ROOT/../db-snapshot}"
BASE=""
NAME=""
RANKS="experiments/parsing/ranks/2026-07-29-demonstratives.json"
LAYERS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)  BASE="$2";  shift 2 ;;
    --layer) LAYERS+=("$2"); shift 2 ;;
    --name)  NAME="$2";  shift 2 ;;
    --ranks) RANKS="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

[[ -n "$BASE" ]]        || { echo "--base is required" >&2; exit 2; }
[[ ${#LAYERS[@]} -gt 0 ]] || { echo "at least one --layer is required" >&2; exit 2; }
[[ -d "$BASE" ]]        || { echo "base snapshot not found: $BASE" >&2; exit 2; }
for l in "${LAYERS[@]}"; do
  [[ -f "$l" ]] || { echo "layer not found: $l" >&2; exit 2; }
done

[[ -n "$NAME" ]] || NAME="modeprobe-$(basename "${LAYERS[0]}" .esl)"
OUT="$SNAPSHOT_ROOT/$NAME"

echo "=== patch layer → probe snapshot ==="
echo "  base   : $BASE"
for l in "${LAYERS[@]}"; do echo "  layer  : $l ($(grep -c 'lexicon:LexicalEntry' "$l") entries)"; done
echo "  out    : $OUT"

scripts/add-layer-to-snapshot.sh --base "$BASE" --out "$OUT" "${LAYERS[@]}"

echo
echo "=== measure (deterministic replay — the only variable is the layer) ==="
scripts/measure-parse-rate.sh --snapshot "$OUT" --replay "$RANKS"
