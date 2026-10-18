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
 * D36 §6.2 — Rename strategy editor.
 *
 * Three controls: side radio (A/B), old-IRI display (read-only,
 * derived from the conflict), new-IRI input. Inline collision
 * validation against the merge span is deferred to a follow-up
 * (chain-browser helper, D36 §9.3) — the kernel's
 * `RenameCollision` typed error catches mistakes at commit time.
 */
import { useEffect, useState } from "react";
import {
  Caption1,
  Field,
  Input,
  Radio,
  RadioGroup,
} from "@fluentui/react-components";
import {
  MergeSide,
  type MergeResolutionWire,
} from "@eigenius/client";

export interface RenameEditorProps {
  conflictId: string;
  oldIri: string;
  onChange: (next: MergeResolutionWire | undefined) => void;
}

export function RenameEditor({ conflictId, oldIri, onChange }: RenameEditorProps) {
  const [side, setSide] = useState<MergeSide>(MergeSide.A);
  const [newIri, setNewIri] = useState("");

  useEffect(() => {
    if (newIri.trim() === "" || newIri.trim() === oldIri) {
      onChange(undefined);
      return;
    }
    const resolution: MergeResolutionWire = {
      $typeName: "eigenius.v1.MergeResolutionWire",
      conflictId,
      strategy: {
        case: "rename",
        value: {
          $typeName: "eigenius.v1.RenameStrategy",
          side,
          oldIri,
          newIri: newIri.trim(),
        },
      },
    };
    onChange(resolution);
  }, [side, oldIri, newIri, conflictId, onChange]);

  return (
    <>
      <Field label="Which side to rename?">
        <RadioGroup
          value={String(side)}
          onChange={(_, data) => setSide(Number(data.value) as MergeSide)}
          layout="horizontal"
        >
          <Radio value={String(MergeSide.A)} label="Branch A" />
          <Radio value={String(MergeSide.B)} label="Branch B" />
        </RadioGroup>
      </Field>
      <Field label="Old IRI">
        <Caption1>
          <code>{oldIri}</code>
        </Caption1>
      </Field>
      <Field
        label="New IRI"
        hint="Must not collide with any IRI in the other branch or the ancestor chain."
        validationMessage={
          newIri.trim() === oldIri
            ? "Same as old IRI — the rename is a no-op."
            : undefined
        }
        validationState={newIri.trim() === oldIri ? "warning" : undefined}
      >
        <Input
          value={newIri}
          onChange={(_, data) => setNewIri(data.value)}
          placeholder="urn:project:billing:Patient"
        />
      </Field>
    </>
  );
}
