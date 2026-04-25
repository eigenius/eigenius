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

# Patent Analysis Demo
#
# Two-step LLM pipeline:
#   1. CompleteJson: extract structured patent analysis
#   2. CompleteText: generate plain-language summary from the structured analysis
#
# Prerequisites:
#   docker compose up (or: just orchestrator-mock + just serve)
#
# Usage:
#   ./demo/patent/run.sh

set -euo pipefail

ENDPOINT="${1:-http://localhost:50051}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

if command -v eigenius &>/dev/null; then
  EIGENIUS="eigenius"
else
  EIGENIUS="cargo run -q -p eigenius-cli --"
fi

echo "=== Patent Analysis Demo ==="
echo "Kernel: $ENDPOINT"
echo

echo "--- Step 1: Load patent ontology ---"
$EIGENIUS --endpoint "$ENDPOINT" load "$SCRIPT_DIR/patent-ontology.esl"
echo

echo "--- Step 2: Load patent document ---"
$EIGENIUS --endpoint "$ENDPOINT" load "$SCRIPT_DIR/transformer-patent.json"
echo

echo "--- Step 3: Run patent analysis pipeline ---"
echo "(CompleteJson → structured extraction, then CompleteText → narrative summary)"
echo
$EIGENIUS --endpoint "$ENDPOINT" run "$SCRIPT_DIR/analyze-patent.esl" "$SCRIPT_DIR/transformer-patent.json"
echo

echo "=== Demo complete ==="
