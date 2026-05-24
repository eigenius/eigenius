#!/usr/bin/env bash
set -u

ENDPOINT="${1:-http://localhost:50051}"
EIGENIUS="cargo run -q -p eigenius-cli -- --endpoint $ENDPOINT"

run_query() {
    local label="$1"
    local query="$2"
    echo "=== $label ==="
    timeout 10 $EIGENIUS query "$query" || echo "FAILED (or timed out)"
    echo
}

run_query "Cell 3 — Verdict" \
    'MATCH "urn:eigenius:institution:Verdict"(?v) {
        "urn:eigenius:institution:verdict_subject": ?subject,
        "urn:eigenius:core:ctor_name":              ?ctor
    }
    WHERE ?subject = "urn:eigenius:demo:lean:proof_term"
    RETURN [] { verdict_iri: ?v, ctor: ?ctor }'

run_query "Cell 5 — LeanProofTerm" \
    'MATCH "urn:eigenius:lean:LeanProofTerm"(?t) {
        "urn:eigenius:lean:target_name": ?theorem,
        "urn:eigenius:lean:claim_iri":   ?claim,
        "urn:eigenius:lean:mirror_iri":  ?mirror
    }
    WHERE ?t = "urn:eigenius:demo:lean:proof_term"
    RETURN [] { proof_term: ?t, theorem: ?theorem, claim: ?claim, mirror: ?mirror }'

run_query "Cell 7 — LeanPackageMirror" \
    'MATCH "urn:eigenius:runtime:RuntimePackageMirror"(?m) {
        "urn:eigenius:runtime:source_layer":          ?source_layer,
        "urn:eigenius:runtime:library_content_hash":  ?lib_hash,
        "urn:eigenius:runtime:mirrored_classes":      ?mirrored
    }
    WHERE ?m = "urn:eigenius:demo:lean:mirror"
    RETURN [] { mirror: ?m, source_layer: ?source_layer, library_hash: ?lib_hash, mirrors: ?mirrored }'

run_query "Cell 9 — Patient class" \
    'MATCH "urn:eigenius:core:Class"(?c) {
        "urn:eigenius:core:short_name": ?name
    }
    WHERE ?c = "urn:eigenius:demo:lean:Patient"
    RETURN [] { class: ?c, short_name: ?name }'
