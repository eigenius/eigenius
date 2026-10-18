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
 * D36 §6.3 — SchemaQuotient strategy editor.
 *
 * Three sub-strategies share this editor (driven by the parent
 * `StrategyPicker`'s `strategy` prop):
 *
 * - `KeepBoth`: structurally inapplicable to every v1 conflict
 *   kind. The picker greys out the radio; if the user somehow
 *   reaches this editor for KeepBoth (future taxonomies), the
 *   editor would just emit the KeepBoth resolution with no winner
 *   field — the kernel's applicability validator catches it.
 * - `KeepOne`: winner radio (A/B) gates the resolution shape.
 * - `KeepNeither`: no nested fields — the resolution is fully
 *   determined by the strategy + the conflict's IRI.
 */
import { useEffect, useState } from "react";
import {
  Caption1,
  Field,
  Radio,
  RadioGroup,
} from "@fluentui/react-components";
import {
  MergeQuotientKind,
  MergeSide,
  MergeStrategyKind,
  type MergeResolutionWire,
  type TypedConflictWire,
} from "@eigenius/client";

export interface QuotientEditorProps {
  conflictId: string;
  strategy:
    | typeof MergeStrategyKind.KEEP_BOTH
    | typeof MergeStrategyKind.KEEP_ONE
    | typeof MergeStrategyKind.KEEP_NEITHER;
  conflict: TypedConflictWire;
  onChange: (next: MergeResolutionWire | undefined) => void;
}

export function QuotientEditor(
  { conflictId, strategy, conflict, onChange }: QuotientEditorProps,
) {
  const [winner, setWinner] = useState<MergeSide>(MergeSide.A);

  useEffect(() => {
    const kind = strategyToQuotientKind(strategy);
    if (kind === undefined) {
      onChange(undefined);
      return;
    }
    if (strategy === MergeStrategyKind.KEEP_ONE && winner === MergeSide.UNSPECIFIED) {
      onChange(undefined);
      return;
    }
    const resolution: MergeResolutionWire = {
      $typeName: "eigenius.v1.MergeResolutionWire",
      conflictId,
      strategy: {
        case: "quotient",
        value: {
          $typeName: "eigenius.v1.QuotientStrategy",
          kind,
          winner: strategy === MergeStrategyKind.KEEP_ONE
            ? winner
            : MergeSide.UNSPECIFIED,
        },
      },
    };
    onChange(resolution);
  }, [strategy, winner, conflictId, onChange]);

  if (strategy === MergeStrategyKind.KEEP_BOTH) {
    return (
      <Caption1>
        Keep both accepts the freely-combined pushout. No v1 conflict
        kind admits this — the kernel will reject at commit time.
      </Caption1>
    );
  }

  if (strategy === MergeStrategyKind.KEEP_NEITHER) {
    const k = conflict.kind;
    const ancestorAvailable = k !== undefined &&
      k.case === "iriCollision" &&
      k.value.ancestorBodyJson !== "";
    return (
      <Caption1>
        {ancestorAvailable
          ? "Both branches' bodies are dropped; the ancestor's body is committed in the merge layer."
          : "Both branches' bodies are dropped. The ancestor has no body, so the merge layer tombstones the IRI — post-merge resolve returns None."}
      </Caption1>
    );
  }

  // KeepOne
  return (
    <Field label="Winner">
      <RadioGroup
        value={String(winner)}
        onChange={(_, data) => setWinner(Number(data.value) as MergeSide)}
        layout="horizontal"
      >
        <Radio value={String(MergeSide.A)} label="Branch A" />
        <Radio value={String(MergeSide.B)} label="Branch B" />
      </RadioGroup>
    </Field>
  );
}

function strategyToQuotientKind(
  strategy: QuotientEditorProps["strategy"],
): MergeQuotientKind | undefined {
  switch (strategy) {
    case MergeStrategyKind.KEEP_BOTH:
      return MergeQuotientKind.KEEP_BOTH;
    case MergeStrategyKind.KEEP_ONE:
      return MergeQuotientKind.KEEP_ONE;
    case MergeStrategyKind.KEEP_NEITHER:
      return MergeQuotientKind.KEEP_NEITHER;
  }
}
