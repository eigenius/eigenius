#!/usr/bin/env bash
#
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
# WRN-Helicase case study — reproducible external-data fetch + verify + derive.
#
# Reads sources.tsv (the machine-readable manifest) and makes every study input
# present and content-verified under data/:
#   - raw     artifacts are downloaded from their public origin (figshare DepMap,
#             NCBI GEO, MSigDB, Springer source-data), md5- and sha256-checked.
#   - manual  artifacts (the paper's NIHMS author-manuscript supplement, not
#             cleanly auto-fetchable) are copied from the in-repo fallback if
#             present, else reported with instructions.
#   - derived artifacts are produced from raw inputs by the extract/ scripts and
#             sha256-checked against their pinned content address.
#
# Idempotent + fail-closed: an artifact already present with the right sha256 is
# skipped; any checksum mismatch aborts that artifact and the run exits non-zero.
#
# Usage:
#   bash data/fetch.sh            # fetch + derive everything missing, then verify
#   bash data/fetch.sh --check    # verify what's present; never download or derive
#
# Provenance narrative: data/MANIFEST.md. Checksums live in data/sources.tsv.

set -uo pipefail

DATA_DIR="$(cd "$(dirname "$0")" && pwd)"
WRN_DIR="$(cd "$DATA_DIR/.." && pwd)"
REPO_DIR="$(cd "$WRN_DIR/../../.." && pwd)"
MANIFEST="$DATA_DIR/sources.tsv"

CHECK_ONLY=0
[ "${1:-}" = "--check" ] && CHECK_ONLY=1

sha256_of() { sha256sum "$1" | awk '{print $1}'; }
md5_of()    { md5sum "$1"    | awk '{print $1}'; }

ok=0; fetched=0; derived=0; missing=0; failed=0

# verify <file> <want-sha> -> 0 if present and matching
verify() {
    [ -f "$1" ] || return 1
    [ "$(sha256_of "$1")" = "$2" ]
}

echo "=== WRN external-data fetch ($([ "$CHECK_ONLY" = 1 ] && echo "verify-only" || echo "fetch+derive")) ==="
mkdir -p "$DATA_DIR/slices" "$DATA_DIR/large"

while IFS=$'\t' read -r id kind source md5 sha dest; do
    case "$id" in ''|\#*) continue ;; esac
    out="$DATA_DIR/$dest"

    if verify "$out" "$sha"; then
        echo "  OK       $id"
        ok=$((ok + 1))
        continue
    fi

    if [ "$CHECK_ONLY" = 1 ]; then
        if [ -f "$out" ]; then
            echo "  MISMATCH $id  (sha256 differs from sources.tsv)"
            failed=$((failed + 1))
        else
            echo "  MISSING  $id  ($dest)"
            missing=$((missing + 1))
        fi
        continue
    fi

    case "$kind" in
    raw)
        echo "  FETCH    $id  <- $source"
        tmp="$(mktemp)"
        if ! curl -fL --retry 3 -o "$tmp" "$source"; then
            echo "    download failed"; rm -f "$tmp"; failed=$((failed + 1)); continue
        fi
        if [ "$md5" != "-" ] && [ "$(md5_of "$tmp")" != "$md5" ]; then
            echo "    md5 mismatch (publisher md5 $md5)"; rm -f "$tmp"; failed=$((failed + 1)); continue
        fi
        if [ "$(sha256_of "$tmp")" != "$sha" ]; then
            echo "    sha256 mismatch (want $sha)"; rm -f "$tmp"; failed=$((failed + 1)); continue
        fi
        mv "$tmp" "$out"
        echo "    ok"; fetched=$((fetched + 1))
        ;;
    manual)
        fallback="$REPO_DIR/$source"
        if [ -f "$fallback" ]; then
            cp "$fallback" "$out"
            if verify "$out" "$sha"; then
                echo "  COPY     $id  <- $source (in-repo fallback)"; fetched=$((fetched + 1))
            else
                echo "  MISMATCH $id  (fallback sha256 differs)"; failed=$((failed + 1))
            fi
        else
            echo "  MANUAL   $id  — not auto-fetchable."
            echo "    This is the paper's NIHMS author-manuscript supplement."
            echo "    Obtain NIHMS1522798-supplement-Supp_Table_1.xlsx and place it at:"
            echo "      $out"
            echo "    (expected sha256 $sha)"
            missing=$((missing + 1))
        fi
        ;;
    derived)
        IFS='|' read -r interp script input <<<"$source"
        if ! command -v "$interp" >/dev/null 2>&1; then
            echo "  SKIP     $id  — $interp not installed; cannot derive"
            missing=$((missing + 1)); continue
        fi
        if [ ! -f "$DATA_DIR/slices/$input" ]; then
            echo "  SKIP     $id  — input $input not present; fetch raw inputs first"
            missing=$((missing + 1)); continue
        fi
        echo "  DERIVE   $id  ($interp $script)"
        ( cd "$DATA_DIR/slices" && "$interp" "$WRN_DIR/$script" ) >/dev/null
        if verify "$out" "$sha"; then
            echo "    ok"; derived=$((derived + 1))
        else
            echo "    sha256 mismatch after derive (want $sha)"; failed=$((failed + 1))
        fi
        ;;
    *)
        echo "  ??       $id  — unknown kind '$kind'"; failed=$((failed + 1))
        ;;
    esac
done <"$MANIFEST"

echo
echo "=== summary: $ok present, $fetched fetched, $derived derived, $missing missing, $failed failed ==="
if [ "$failed" -gt 0 ]; then exit 1; fi
if [ "$CHECK_ONLY" = 0 ] && [ "$missing" -gt 0 ]; then exit 1; fi
exit 0
