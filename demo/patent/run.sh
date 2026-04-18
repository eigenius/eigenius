#!/usr/bin/env bash
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
