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

# D57 schema.org generator — Level-2 lift demo (D60), end to end on a clean DB.
#
# Runs the schema.org generator AS A PROGRAM THROUGH THE KERNEL: the generic `oci`
# tool runtime spawns `eigenius-schemaorg-worker` in a pinned image, converts the
# content-pinned schema.org V30.0 vocabulary, and commits `generate_result` + a
# ProgramTrace -> IsDerivedAs(generate_result, GeneratorConforms). The chain's
# `concl_generator` then discharges its conformance leg via `derived(...)`, and the
# thesis `concl_main` composes m1..m4 — all kernel-checked `Holds`.
#
# Prerequisites:
#   EIGENIUS_MOCK_LLM=true docker compose up -d   (kernel + orchestrator healthy;
#   the orchestrator image must include eigenius-schemaorg-worker + EIGENIUS_OCI_*).
#   docker + buildah on the host (env build bakes the image; the substrate spawns
#   the worker as a sibling container via DooD).
#
# Usage:  ./demo/d57-schema-org/run.sh  [endpoint]

set -euo pipefail
EP="${1:-http://localhost:50051}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/../.." && pwd)"
OBJ="$REPO/experiments/objectives/d57-schema-org"
CHAIN="$OBJ/chain"
PROGRAMS="$OBJ/programs"
INPUT="$REPO/crates/eigenius-schemaorg/data/schemaorg-current-https-v30.0.jsonld"
HEX=0f0c97a4f666b2f8563573fe48453782fd51b87a504523cf0c9aff6a71c3eec4
CACHE=/var/lib/eigenius/substrate-depot/extfile-cache
ORCH="${ORCH:-eigenius-orchestrator-1}"

echo "Building CLI + worker (release)..."
(cd "$REPO" && cargo build -q -p eigenius-cli -p eigenius-schemaorg-worker --release)
EIG="$REPO/target/release/eigenius"
eig() { "$EIG" --endpoint "$EP" "$@"; }

if [ ! -f "$INPUT" ]; then
  echo "schema.org V30.0 input absent ($INPUT) — fetch it first (crates/eigenius-schemaorg/data/MANIFEST.md)." >&2
  exit 1
fi

# 1. Build the oci image FROM THE ORCHESTRATOR'S STAGED WORKER, so the image's
#    baked manifest-hash matches what the orchestrator's OciToolRuntime computes
#    at dispatch (boot cross-check, D26 §9.3). Then the run resolves this digest.
echo "=== Building oci tool image from the orchestrator's staged worker ==="
docker cp "$ORCH:/opt/eigenius/oci-runtime-worker/eigenius-schemaorg-worker" /tmp/d57-oci-worker
DIGEST="$(eig env build --language oci --worker-source-dir /tmp/d57-oci-worker \
  --base-image debian:bookworm-slim | grep -oE 'sha256:[0-9a-f]{64}' | head -1)"
echo "  oci image digest: $DIGEST"

# 2. Stage the content-pinned input into the depot's extfile-cache (the
#    DooD-shared mount the substrate + sibling worker both see).
echo "=== Staging V30.0 input into the extfile-cache ==="
docker exec "$ORCH" mkdir -p "$CACHE/$HEX"
docker cp "$INPUT" "$ORCH:$CACHE/$HEX/schemaorg-current-https-v30.0.jsonld"

# 3. Fresh branch + load the chain prefix (gen_input commits here).
echo "=== Loading the D57 chain prefix onto a fresh obj-d57 branch ==="
H="$(eig branch list | awk '/^main /{print $2}')"
eig branch delete obj-d57 --force >/dev/null 2>&1 || true
eig branch create obj-d57 --from "$H"
for f in 00-objective 01-discipline; do eig load --branch obj-d57 "$CHAIN/$f.esl"; done
eig load --branch obj-d57 "$REPO/ontologies/objective/objective-ontology.esl"
for f in 02-objective-typed 03-probe 04a-evidence; do eig load --branch obj-d57 "$CHAIN/$f.esl"; done

# 4. Run the generator THROUGH THE KERNEL (oci RunRuntimeScript).
echo "=== eigenius run: generator -> generate_result + ProgramTrace -> IsDerivedAs ==="
sed "s|sha256:0\{64\}|$DIGEST|" "$PROGRAMS/generate-program.json" > /tmp/d57-generate-program.json
eig run --branch obj-d57 /tmp/d57-generate-program.json "$PROGRAMS/gen-input.json" >/dev/null
echo "  committed urn:eigenius:obj:d57:generate_result"

# 5. Load the conclusions (AutoOnLoad gates them) + the thesis.
echo "=== Loading conclusions + thesis (kernel-checked) ==="
eig load --branch obj-d57 "$CHAIN/04b-conclusions.esl"
eig load --branch obj-d57 "$CHAIN/05-synthesis.esl"

# 6. Show the verdicts.
echo "=== Verdicts ==="
eig query --branch obj-d57 \
  'MATCH "urn:eigenius:institution:Verdict"(?v) { "urn:eigenius:institution:verdict_subject": ?s, "urn:eigenius:core:ctor_name": ?c } RETURN [] { subject: ?s, verdict: ?c }' \
  | grep -iE "concl_(discipline|probe|generator|cut|main).*|Holds" || true
echo
echo "concl_generator Holds via derived(generate_result, GeneratorConforms) — the lift."
