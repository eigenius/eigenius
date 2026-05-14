// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

/**
 * D36 §6.1 — Witness strategy editor.
 *
 * One IRI input: the `MergeComorphism` resource the kernel will
 * apply as the merge witness. The kernel type-checks the witness
 * against the conflict's class at submission time, so this editor
 * doesn't need to do pre-validation beyond the IRI parse — the
 * structural failures surface as `MALFORMED_RESOLUTION` /
 * `WitnessTypeMismatch` from `submitResolution` and the error
 * banner routes back to picking with the field highlighted.
 *
 * The "Browse committed MergeComorphisms" affordance D36 §6.1
 * sketches is a follow-up (chain-browser helper, D36 §9.3).
 */
import { useEffect, useState } from "react";
import { Field, Input } from "@fluentui/react-components";
import {
  MergeStrategyKind,
  type MergeResolutionWire,
} from "@eigenius/client";

export interface WitnessEditorProps {
  conflictId: string;
  onChange: (next: MergeResolutionWire | undefined) => void;
}

export function WitnessEditor({ conflictId, onChange }: WitnessEditorProps) {
  const [iri, setIri] = useState("");

  useEffect(() => {
    // Empty IRI → incomplete form; surface `undefined` so the
    // panel-level Preview button stays disabled.
    if (iri.trim() === "") {
      onChange(undefined);
      return;
    }
    const resolution: MergeResolutionWire = {
      $typeName: "eigenius.v1.MergeResolutionWire",
      conflictId,
      strategy: {
        case: "witness",
        value: {
          $typeName: "eigenius.v1.WitnessStrategy",
          comorphismIri: iri.trim(),
        },
      },
    };
    onChange(resolution);
  }, [iri, conflictId, onChange]);

  return (
    <Field
      label="Comorphism IRI"
      hint="IRI of a MergeComorphism resource committed earlier in the chain."
    >
      <Input
        value={iri}
        onChange={(_, data) => setIri(data.value)}
        placeholder="urn:project:patient_merge_witness"
      />
    </Field>
  );
}

// Silence unused-import warning in case the file is included before
// callers wire MergeStrategyKind through.
void MergeStrategyKind;
