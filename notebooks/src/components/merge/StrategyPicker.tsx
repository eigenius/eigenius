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
 * D36 §6 — Per-conflict strategy picker.
 *
 * Renders the strategy radio list for a single `TypedConflictWire`,
 * dispatches to the relevant editor component when a strategy is
 * selected, and surfaces the per-conflict applicability table from
 * `conflict.applicableStrategies` (greyed-out radios with inline
 * "not applicable here" copy for strategies the kernel rejects).
 *
 * KeepBoth is always rendered, even when no v1 conflict kind admits
 * it (D36 §6.5 / §15.5) — teaches the user that the strategy exists
 * and clarifies the structural reason it doesn't apply.
 */
import { useCallback, useMemo, useState } from "react";
import {
  Caption1,
  Field,
  makeStyles,
  MessageBar,
  MessageBarBody,
  Radio,
  RadioGroup,
  tokens,
} from "@fluentui/react-components";
import {
  MergeQuotientKind,
  MergeStrategyKind,
  type MergeResolutionWire,
  type TypedConflictWire,
} from "@eigenius/client";
import { WitnessEditor } from "./WitnessEditor";
import { RenameEditor } from "./RenameEditor";
import { QuotientEditor } from "./QuotientEditor";
import { RestructureEditor } from "./RestructureEditor";

const useStyles = makeStyles({
  card: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
    padding: tokens.spacingHorizontalM,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    borderRadius: tokens.borderRadiusMedium,
    background: tokens.colorNeutralBackground1,
  },
  header: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXS,
  },
  iri: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase300,
  },
  kindLabel: {
    color: tokens.colorNeutralForeground2,
  },
  body: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
    paddingLeft: tokens.spacingHorizontalL,
    borderLeft: `2px solid ${tokens.colorNeutralStroke2}`,
  },
  inapplicableNote: {
    color: tokens.colorNeutralForeground3,
    fontStyle: "italic",
    marginLeft: tokens.spacingHorizontalM,
  },
});

export interface StrategyPickerProps {
  conflict: TypedConflictWire;
  resolution: MergeResolutionWire | undefined;
  /** Pass the store action directly so we can derive a stable
   * per-conflict `onChange` here via `useCallback`. An inline
   * `(next) => setMergeResolution(conflict.id, next)` from the
   * parent re-creates on every render, which combined with the
   * editors' `useEffect` deps would loop the picker into a render
   * storm. */
  setResolution: (conflictId: string, next: MergeResolutionWire | undefined) => void;
}

/**
 * Header line summarising the conflict. Mirrors the kind-specific
 * info the user needs to pick a strategy without having to scroll
 * back into the body of the conflict.
 */
function ConflictHeader(
  { conflict, styles }: {
    conflict: TypedConflictWire;
    styles: ReturnType<typeof useStyles>;
  },
) {
  const kind = conflict.kind;
  if (kind === undefined) {
    return (
      <MessageBar intent="warning">
        <MessageBarBody>
          Internal: kernel surfaced a conflict kind without a wire shape.
          Reload to retry.
        </MessageBarBody>
      </MessageBar>
    );
  }
  switch (kind.case) {
    case "propertyDataType":
      return (
        <div className={styles.header}>
          <Caption1 className={styles.kindLabel}>
            Property data-type disagreement
          </Caption1>
          <div className={styles.iri}>{kind.value.property}</div>
          <Caption1>
            Branch A: <code>{kind.value.branchAType}</code> · Branch B:{" "}
            <code>{kind.value.branchBType}</code>
            {kind.value.ancestorType
              ? <> · Ancestor: <code>{kind.value.ancestorType}</code></>
              : null}
          </Caption1>
        </div>
      );
    case "kindMismatch":
      return (
        <div className={styles.header}>
          <Caption1 className={styles.kindLabel}>Kind mismatch</Caption1>
          <div className={styles.iri}>{kind.value.iri}</div>
          <Caption1>
            Branch A says <code>{kind.value.branchAKind}</code>; Branch B
            says <code>{kind.value.branchBKind}</code>.
          </Caption1>
        </div>
      );
    case "iriCollision":
      return (
        <div className={styles.header}>
          <Caption1 className={styles.kindLabel}>
            IRI collision — bodies differ
          </Caption1>
          <div className={styles.iri}>{kind.value.iri}</div>
          <Caption1>
            Both branches modified this resource with different bodies.
            {kind.value.ancestorBodyJson
              ? " The ancestor's body is available as a fallback."
              : " The ancestor has no body at this IRI."}
          </Caption1>
        </div>
      );
    case "inheritanceCycle":
      return (
        <div className={styles.header}>
          <Caption1 className={styles.kindLabel}>
            Inheritance cycle (subclass_of)
          </Caption1>
          <Caption1>
            Merged graph cycles through:{" "}
            <code>{kind.value.cycle.join(" → ")}</code>
          </Caption1>
        </div>
      );
    default:
      return null;
  }
}

/**
 * Conflict's primary IRI — the value the resolution targets. Witness
 * + Rename + Quotient all key on this; Restructure has a richer
 * shape but the affected_class is the primary handle. Returns empty
 * string only for `InheritanceCycle` (which has a cycle, not a
 * single IRI) and for reserved kinds the v1 wire doesn't carry.
 */
function primaryIri(conflict: TypedConflictWire): string {
  const k = conflict.kind;
  if (k === undefined) return "";
  switch (k.case) {
    case "propertyDataType":
      return k.value.property;
    case "kindMismatch":
      return k.value.iri;
    case "iriCollision":
      return k.value.iri;
    case "inheritanceCycle":
      // Cycles don't have a primary IRI — the editors that key off
      // it (Witness, Rename) need a sub-picker we don't ship in
      // PR 2. The picker disables those strategies for cycles.
      return "";
    default:
      return "";
  }
}

export function StrategyPicker(
  { conflict, resolution, setResolution }: StrategyPickerProps,
) {
  const styles = useStyles();
  // Local UI state: which strategy the user has selected (may be
  // different from `resolution` while the editor's form is still
  // incomplete — `onChange(undefined)` keeps `resolution` cleared
  // until the editor is submittable).
  const [strategy, setStrategy] = useState<MergeStrategyKind>(
    resolution ? resolutionStrategyKind(resolution) : MergeStrategyKind.UNSPECIFIED,
  );
  // Per-conflict stable handler; the editors put `onChange` in
  // their `useEffect` deps and need a reference that doesn't churn
  // every render.
  const onChange = useCallback(
    (next: MergeResolutionWire | undefined) => setResolution(conflict.id, next),
    [conflict.id, setResolution],
  );

  const applicable = useMemo(
    () => new Set(conflict.applicableStrategies),
    [conflict.applicableStrategies],
  );
  const targetIri = primaryIri(conflict);
  const targetMissing = targetIri === "";

  const handleStrategyChange = (next: MergeStrategyKind) => {
    setStrategy(next);
    // Clear any previously-selected resolution — the editor below
    // will fire `onChange` when the user completes the new form.
    onChange(undefined);
  };

  return (
    <div className={styles.card}>
      <ConflictHeader conflict={conflict} styles={styles} />

      <Field label="Strategy">
        <RadioGroup
          value={String(strategy)}
          onChange={(_, data) => handleStrategyChange(Number(data.value) as MergeStrategyKind)}
        >
          {renderStrategyRadio(
            MergeStrategyKind.WITNESS,
            "Witness — apply a typed merge term",
            applicable,
            targetMissing,
            styles,
          )}
          {renderStrategyRadio(
            MergeStrategyKind.RENAME,
            "Rename — disambiguate one side's IRI",
            applicable,
            targetMissing,
            styles,
          )}
          {renderStrategyRadio(
            MergeStrategyKind.KEEP_BOTH,
            "Keep both — accept the freely-combined pushout",
            applicable,
            // KeepBoth's inapplicability is structural, not
            // target-missing — render with kind-specific copy.
            false,
            styles,
          )}
          {renderStrategyRadio(
            MergeStrategyKind.KEEP_ONE,
            "Keep one — pick a winner",
            applicable,
            false,
            styles,
          )}
          {renderStrategyRadio(
            MergeStrategyKind.KEEP_NEITHER,
            "Keep neither — restore the ancestor's body (or tombstone)",
            applicable,
            false,
            styles,
          )}
          {renderStrategyRadio(
            MergeStrategyKind.RESTRUCTURE,
            "Restructure — introduce a new common parent",
            applicable,
            // Restructure needs the affected_class to derive the
            // resolution shape; cycle conflicts (which have no
            // single primary IRI) can't drive the form, so disable
            // for those.
            targetMissing,
            styles,
          )}
        </RadioGroup>
      </Field>

      <div className={styles.body}>
        {strategy === MergeStrategyKind.WITNESS && !targetMissing && (
          <WitnessEditor
            conflict={conflict}
            onChange={onChange}
          />
        )}
        {strategy === MergeStrategyKind.RENAME && !targetMissing && (
          <RenameEditor
            conflictId={conflict.id}
            oldIri={targetIri}
            onChange={onChange}
          />
        )}
        {(strategy === MergeStrategyKind.KEEP_BOTH ||
          strategy === MergeStrategyKind.KEEP_ONE ||
          strategy === MergeStrategyKind.KEEP_NEITHER) && (
          <QuotientEditor
            conflictId={conflict.id}
            strategy={strategy}
            conflict={conflict}
            onChange={onChange}
          />
        )}
        {strategy === MergeStrategyKind.RESTRUCTURE && !targetMissing && (
          <RestructureEditor
            conflictId={conflict.id}
            affectedClass={targetIri}
            onChange={onChange}
          />
        )}
      </div>
    </div>
  );
}

/**
 * Map a stored `MergeResolutionWire` back to its `MergeStrategyKind`.
 * Used when restoring a partial picker state from localStorage so
 * the radio reflects the user's earlier choice.
 */
function resolutionStrategyKind(r: MergeResolutionWire): MergeStrategyKind {
  const s = r.strategy;
  if (s === undefined) return MergeStrategyKind.UNSPECIFIED;
  switch (s.case) {
    case "witness":
      return MergeStrategyKind.WITNESS;
    case "rename":
      return MergeStrategyKind.RENAME;
    case "quotient": {
      const k = s.value.kind;
      if (k === MergeQuotientKind.KEEP_BOTH) return MergeStrategyKind.KEEP_BOTH;
      if (k === MergeQuotientKind.KEEP_ONE) return MergeStrategyKind.KEEP_ONE;
      return MergeStrategyKind.KEEP_NEITHER;
    }
    default:
      return MergeStrategyKind.UNSPECIFIED;
  }
}

/**
 * Render one radio row, greyed-out with an inline rationale when
 * the strategy isn't applicable. Encapsulates the "always-show,
 * sometimes-disable" pattern D36 §6.5 specifies for KeepBoth and
 * D36 §14's PR 3 deferment for Restructure.
 */
function renderStrategyRadio(
  kind: MergeStrategyKind,
  label: string,
  applicable: ReadonlySet<MergeStrategyKind>,
  alsoDisabled: boolean,
  styles: ReturnType<typeof useStyles>,
) {
  const disabled = !applicable.has(kind) || alsoDisabled;
  const inapplicable = !applicable.has(kind);
  return (
    <>
      <Radio
        value={String(kind)}
        label={label}
        disabled={disabled}
      />
      {inapplicable && (
        <Caption1 className={styles.inapplicableNote}>
          Not applicable to this conflict kind.
        </Caption1>
      )}
      {!inapplicable && alsoDisabled && (
        <Caption1 className={styles.inapplicableNote}>
          Not yet wired in the notebook UI — use the{" "}
          <code>eigenius db merge resolve</code> CLI for this case.
        </Caption1>
      )}
    </>
  );
}
