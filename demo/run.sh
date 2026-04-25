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

# Eigenius End-to-End Demo
#
# Prerequisites:
#   docker compose up   (or: EIGENIUS_MOCK_LLM=true docker compose up)
#
# Usage:
#   ./demo/run.sh                          # against Docker Compose stack
#   ./demo/run.sh http://localhost:50051   # custom kernel endpoint
#
# What it does:
#   1. Health-checks the orchestrator
#   2. Loads a document into the kernel
#   3. Runs a summarization program (dispatches to CompleteText)
#   4. Inspects a core resource
#   5. Queries all loaded classes

set -euo pipefail

ENDPOINT="${1:-http://localhost:50051}"
ORCHESTRATOR="${2:-http://localhost:8080}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Use installed eigenius if available, otherwise cargo run
if command -v eigenius &>/dev/null; then
  EIGENIUS="eigenius"
else
  EIGENIUS="cargo run -q -p eigenius-cli --"
fi

echo "=== Eigenius End-to-End Demo ==="
echo "Kernel:       $ENDPOINT"
echo "Orchestrator: $ORCHESTRATOR"
echo

# Step 0: Health check
echo "--- Step 0: Health check ---"
if curl -sf "$ORCHESTRATOR/health" | head -c 200; then
    echo
    echo "Orchestrator is healthy."
else
    echo "ERROR: Orchestrator not reachable at $ORCHESTRATOR/health"
    echo "Start the stack first: docker compose up"
    exit 1
fi
echo

# Step 1: Load document
echo "--- Step 1: Load document ---"
$EIGENIUS --endpoint "$ENDPOINT" load "$SCRIPT_DIR/document.json"
echo

# Step 2: Inspect a core class
echo "--- Step 2: Inspect core:Class ---"
$EIGENIUS --endpoint "$ENDPOINT" inspect "urn:eigenius:core:Class"
echo

# Step 3: Query all loaded classes
echo "--- Step 3: Query all classes ---"
$EIGENIUS --endpoint "$ENDPOINT" query 'MATCH "urn:eigenius:core:Class"(?c) { short_name: ?name } RETURN [] { class: ?c, name: ?name }'
echo

# Step 4: Run the summarization program
echo "--- Step 4: Run summarize program ---"
$EIGENIUS --endpoint "$ENDPOINT" run "$SCRIPT_DIR/summarize-program.json" "$SCRIPT_DIR/input.json"
echo

# Step 5: Load ESL directly into the remote kernel
echo "--- Step 5: Load ESL into kernel ---"
$EIGENIUS --endpoint "$ENDPOINT" load "$SCRIPT_DIR/document.esl"
echo

# Step 6: Run ESL program against the remote kernel
echo "--- Step 6: Run ESL program ---"
$EIGENIUS --endpoint "$ENDPOINT" run "$SCRIPT_DIR/summarize.esl" "$SCRIPT_DIR/input.json"
echo

echo "=== Demo complete ==="
