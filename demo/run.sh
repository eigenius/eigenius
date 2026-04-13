#!/usr/bin/env bash
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
eigenius --endpoint "$ENDPOINT" load "$SCRIPT_DIR/document.json"
echo

# Step 2: Inspect a core class
echo "--- Step 2: Inspect core:Class ---"
eigenius --endpoint "$ENDPOINT" inspect "urn:eigenius:core:Class"
echo

# Step 3: Query all loaded classes
echo "--- Step 3: Query all classes ---"
eigenius --endpoint "$ENDPOINT" query 'MATCH "urn:eigenius:core:Class"(?c) { short_name: ?name } RETURN [] { class: ?c, name: ?name }'
echo

# Step 4: Run the summarization program
echo "--- Step 4: Run summarize program ---"
eigenius --endpoint "$ENDPOINT" run "$SCRIPT_DIR/summarize-program.json" "$SCRIPT_DIR/document.json"
echo

echo "=== Demo complete ==="
