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

# Provision earlier HPO releases for the registry converter's label check (D99 §7).
#
# A registry row stores an HP ID and the label it was coded with. HPO renames terms between releases
# and does not always keep the old label as a synonym (HP:0003236 lost "Elevated circulating creatine
# kinase concentration" in 2026-06-06), so telling a label from an earlier release from one that names
# a different term needs the releases themselves. This fetches `hp.obo` for a span of releases into
# references/hpo/history/<tag>.obo (gitignored: HPO is third-party data, provisioned on demand).
#
# Releases from 2023 on publish hp.obo as a release asset; earlier ones carry it in the repository at
# the tag. Both are tried, in that order.
#
# Usage:
#   scripts/provision-hpo-history.sh               # the default span, 2018-03-08 … 2026-06-23
#   scripts/provision-hpo-history.sh v2025-11-24   # specific tags

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/references/hpo/history"
REPO="obophenotype/human-phenotype-ontology"

TAGS=("$@")
if [[ ${#TAGS[@]} -eq 0 ]]; then
  TAGS=(v2018-03-08 v2019-02-12 v2020-02-27 v2021-02-08 v2021-04-13 v2021-06-08 v2021-08-02
        v2021-10-10 v2022-02-14 v2023-01-27 v2024-01-11 v2025-01-16 v2025-11-24 v2026-01-08
        v2026-02-16 v2026-06-06 v2026-06-23)
fi

mkdir -p "$OUT"
for tag in "${TAGS[@]}"; do
  dest="$OUT/$tag.obo"
  if [[ -s "$dest" ]]; then
    echo "have  $tag"
    continue
  fi
  if curl -sfL -o "$dest.part" "https://github.com/$REPO/releases/download/$tag/hp.obo" \
     || curl -sfL -o "$dest.part" "https://raw.githubusercontent.com/$REPO/$tag/hp.obo"; then
    # A release's hp.obo names its version in the header; refuse a file that is not one.
    if grep -q "^data-version:" "$dest.part"; then
      mv "$dest.part" "$dest"
      echo "fetch $tag ($(grep -m1 '^data-version:' "$dest" | cut -d' ' -f2))"
    else
      rm -f "$dest.part"
      echo "error: $tag: the fetched file is not an OBO release" >&2
      exit 1
    fi
  else
    rm -f "$dest.part"
    echo "error: $tag: hp.obo is neither a release asset nor in the repository at the tag" >&2
    exit 1
  fi
done
echo "→ $OUT ($(ls "$OUT"/*.obo | wc -l) releases)"
