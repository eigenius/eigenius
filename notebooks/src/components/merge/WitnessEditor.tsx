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
 * D36 §6.1 / D37 §7.1 — Witness strategy editor.
 *
 * Combobox of applicable `MergeComorphism` IRIs, populated by an
 * EigenQL query against the chain for comorphisms whose
 * `merge_target_class` matches the conflict's class. The Combobox is
 * preferred over a free-form IRI input because:
 *
 * 1. Discoverability — the user sees which witnesses exist for this
 *    conflict's class without leaving the resolution flow.
 * 2. Validation — typoed IRIs no longer slip through to
 *    `MALFORMED_RESOLUTION` at submit time; the picker only shows
 *    IRIs that resolve in the chain.
 * 3. Class-correctness — the kernel rejects comorphisms applied to
 *    the wrong class (D37 §6.2), but the picker pre-filters them so
 *    the user never sees an incompatible witness in the first place.
 *
 * Three fallback shapes:
 * - **Conflict class undetectable** (kind isn't IriCollision /
 *   KindMismatch, or the body JSON didn't parse): render the
 *   free-form Input the editor used pre-D37.
 * - **No applicable comorphisms found**: free-form Input plus a
 *   caption explaining no witnesses are committed for this class.
 * - **Query errored**: free-form Input plus a soft error caption;
 *   the user can still author by hand.
 *
 * The query fires on mount + whenever the conflict's id changes.
 * The resolution session itself is atomic against a captured
 * `branchTip` (D36 §11), so we don't need to re-fire on chain
 * advance during a session — the tip is frozen for the session's
 * lifetime, and CAS-race recovery re-mounts the picker.
 */
import { useEffect, useState } from "react";
import {
  Badge,
  Button,
  Caption1,
  Combobox,
  Field,
  Input,
  Link,
  makeStyles,
  Option,
  Spinner,
  tokens,
} from "@fluentui/react-components";
import { Dismiss12Regular } from "@fluentui/react-icons";
import { decode as cborDecode } from "cbor-x";
import { type Eigen, type MergeResolutionWire, type TypedConflictWire } from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
import { decodeResultDocument } from "../../runtime/resultDocument";
import { useNotebookStore } from "../../runtime/notebookStore";

export interface WitnessEditorProps {
  conflict: TypedConflictWire;
  onChange: (next: MergeResolutionWire | undefined) => void;
}

const useStyles = makeStyles({
  scope: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXS,
    marginTop: tokens.spacingVerticalS,
  },
  scopeChips: {
    display: "flex",
    flexWrap: "wrap",
    gap: tokens.spacingHorizontalXXS,
  },
  scopeChip: {
    display: "inline-flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalXXS,
  },
  scopeRow: {
    display: "flex",
    gap: tokens.spacingHorizontalXS,
    alignItems: "flex-end",
  },
});

interface ApplicableComorphism {
  iri: string;
  shortName: string | null;
}

type PickerState =
  | { kind: "loading" }
  | { kind: "ready"; comorphisms: ApplicableComorphism[]; conflictClass: string }
  | { kind: "no-class" } // Class wasn't extractable from the conflict.
  | { kind: "error"; message: string };

export function WitnessEditor({ conflict, onChange }: WitnessEditorProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const [iri, setIri] = useState("");
  const [picker, setPicker] = useState<PickerState>({ kind: "loading" });
  const [scopeExpanded, setScopeExpanded] = useState(false);
  const [scopeDraft, setScopeDraft] = useState("");

  // D38 §4 — caller-controlled extra search branches. State lives
  // in the merge-resolution store so it survives re-renders + race
  // recovery; the WitnessEditor only reads/writes via the store
  // actions.
  const searchBranches = useNotebookStore((s) =>
    s.mergeResolution.kind === "picking"
      ? s.mergeResolution.witnessSearchBranches
      : []
  );
  const setSearchBranches = useNotebookStore((s) => s.setWitnessSearchBranches);

  // Fire the comorphism query when the conflict's id or search-scope
  // changes. The picker unions matches from the default branch with
  // matches from each named search branch.
  useEffect(() => {
    let cancelled = false;
    const conflictClass = extractConflictClass(conflict);
    if (conflictClass === null) {
      setPicker({ kind: "no-class" });
      return;
    }
    setPicker({ kind: "loading" });
    void queryApplicableComorphisms(eigen, conflictClass, searchBranches).then(
      (comorphisms) => {
        if (cancelled) return;
        setPicker({ kind: "ready", comorphisms, conflictClass });
      },
      (err: unknown) => {
        if (cancelled) return;
        setPicker({
          kind: "error",
          message: err instanceof Error ? err.message : String(err),
        });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [conflict, eigen, searchBranches]);

  const addScopeFromDraft = () => {
    const additions = scopeDraft
      .split(/[,\s]+/)
      .map((s) => s.trim())
      .filter((s) => s !== "");
    if (additions.length === 0) return;
    setSearchBranches([...searchBranches, ...additions]);
    setScopeDraft("");
  };
  const removeScope = (name: string) => {
    setSearchBranches(searchBranches.filter((b) => b !== name));
  };

  // Common disclosure block — rendered alongside whichever picker
  // surface fires. Putting it outside the switch keeps the affordance
  // visible across the no-class / error / empty paths so a user who
  // can't find a witness in the default scope has a way to expand.
  const scopeDisclosure = (
    <div className={styles.scope}>
      <Link
        as="button"
        appearance="subtle"
        onClick={() => setScopeExpanded((v) => !v)}
      >
        {scopeExpanded ? "▾" : "▸"} Search additional branches
        {searchBranches.length > 0 && ` (${searchBranches.length})`}
      </Link>
      {scopeExpanded && (
        <>
          <Caption1>
            Names entered here are passed to{" "}
            <code>witness_search_branches</code> (D38 §4). The resolver
            falls through to them after the merge span's sources and
            the ancestor chain.
          </Caption1>
          <div className={styles.scopeRow}>
            <Input
              size="small"
              value={scopeDraft}
              onChange={(_, data) => setScopeDraft(data.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  addScopeFromDraft();
                }
              }}
              placeholder="witness-library, prod-witnesses"
              style={{ flex: 1 }}
            />
            <Button
              size="small"
              onClick={addScopeFromDraft}
              disabled={scopeDraft.trim() === ""}
            >
              Add
            </Button>
          </div>
          {searchBranches.length > 0 && (
            <div className={styles.scopeChips}>
              {searchBranches.map((b) => (
                <Badge
                  key={b}
                  appearance="outline"
                  className={styles.scopeChip}
                >
                  <code>{b}</code>
                  <Button
                    appearance="subtle"
                    size="small"
                    icon={<Dismiss12Regular />}
                    aria-label={`Remove ${b}`}
                    onClick={() => removeScope(b)}
                  />
                </Badge>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );

  // Push the current IRI (free-form or picker-selected) up to the
  // resolution state as a witness resolution. Empty → undefined so
  // the panel-level Preview button stays disabled.
  useEffect(() => {
    if (iri.trim() === "") {
      onChange(undefined);
      return;
    }
    const resolution: MergeResolutionWire = {
      $typeName: "eigenius.v1.MergeResolutionWire",
      conflictId: conflict.id,
      strategy: {
        case: "witness",
        value: {
          $typeName: "eigenius.v1.WitnessStrategy",
          comorphismIri: iri.trim(),
        },
      },
    };
    onChange(resolution);
  }, [iri, conflict.id, onChange]);

  const onSelectComorphism = (selectedIri: string) => {
    setIri(selectedIri);
  };

  // Render path picks based on the picker state.
  switch (picker.kind) {
    case "loading":
      return (
        <>
          <Field label="Comorphism IRI">
            <Spinner size="tiny" label="searching for applicable comorphisms…" />
          </Field>
          {scopeDisclosure}
        </>
      );

    case "ready": {
      if (picker.comorphisms.length === 0) {
        return (
          <>
            <Field
              label="Comorphism IRI"
              hint={
                <span>
                  No <code>MergeComorphism</code>{" "}
                  resources are committed for class{" "}
                  <code>{picker.conflictClass}</code>. Author one
                  (see <code>merge_comorphism</code> in ESL, D37 §3.3)
                  then paste its IRI here, or expand the scope below.
                </span>
              }
            >
              <Input
                value={iri}
                onChange={(_, data) => setIri(data.value)}
                placeholder="urn:project:patient_merge_witness"
              />
            </Field>
            {scopeDisclosure}
          </>
        );
      }
      return (
        <>
          <Field
            label="Comorphism"
            hint={
              <span>
                Pick a witness committed for class{" "}
                <code>{picker.conflictClass}</code>.
              </span>
            }
          >
            <Combobox
              value={iri}
              selectedOptions={iri ? [iri] : []}
              onOptionSelect={(_e, data) => {
                if (data.optionValue) onSelectComorphism(data.optionValue);
              }}
              placeholder="Select a comorphism"
            >
              {picker.comorphisms.map((c) => (
                <Option key={c.iri} value={c.iri} text={c.shortName ?? c.iri}>
                  {c.shortName
                    ? (
                      <span>
                        <strong>{c.shortName}</strong>{" "}
                        <code style={{ opacity: 0.6 }}>{c.iri}</code>
                      </span>
                    )
                    : <code>{c.iri}</code>}
                </Option>
              ))}
            </Combobox>
          </Field>
          {scopeDisclosure}
        </>
      );
    }

    case "no-class":
      return (
        <>
          <Field
            label="Comorphism IRI"
            hint="Conflict shape doesn't expose a target class for the picker — paste an IRI directly."
          >
            <Input
              value={iri}
              onChange={(_, data) => setIri(data.value)}
              placeholder="urn:project:patient_merge_witness"
            />
          </Field>
          {scopeDisclosure}
        </>
      );

    case "error":
      return (
        <>
          <Field
            label="Comorphism IRI"
            hint={
              <span>
                Couldn't query the chain for applicable comorphisms:{" "}
                <em>{picker.message}</em>. Paste an IRI directly.
              </span>
            }
            validationState="warning"
          >
            <Input
              value={iri}
              onChange={(_, data) => setIri(data.value)}
              placeholder="urn:project:patient_merge_witness"
            />
          </Field>
          {scopeDisclosure}
        </>
      );
  }
}

/**
 * Pull the conflict's class IRI from the wire shape (D37 §7.1).
 *
 * `TypedConflictWire`'s variants don't expose `class` directly. For
 * `IriCollisionConflict` the bodies are class instances — we parse
 * `branchABodyJson` and read `urn:eigenius:core:is_a[0]`. For
 * `KindMismatchConflict` the `branchAKind` field is already the
 * class IRI. Other kinds (`PropertyDataTypeConflict`,
 * `InheritanceCycleConflict`) don't have a single target class the
 * Witness strategy operates on — return null so the editor falls
 * back to the free-form IRI input.
 */
function extractConflictClass(conflict: TypedConflictWire): string | null {
  const kind = conflict.kind;
  if (!kind) return null;
  switch (kind.case) {
    case "iriCollision": {
      const bodyJson = kind.value.branchABodyJson;
      if (!bodyJson) return null;
      try {
        const parsed = JSON.parse(bodyJson) as Record<string, unknown>;
        const isA = parsed["urn:eigenius:core:is_a"];
        if (!Array.isArray(isA) || isA.length === 0) return null;
        const first = isA[0];
        return typeof first === "string" ? first : null;
      } catch {
        return null;
      }
    }
    case "kindMismatch": {
      // `branchAKind` is itself the class IRI; use it as the target
      // class for the Witness picker query.
      return kind.value.branchAKind || null;
    }
    case "propertyDataType":
    case "inheritanceCycle":
    default:
      return null;
  }
}

/**
 * Query the chain for `MergeComorphism` resources whose
 * `merge_target_class` matches the supplied class IRI. Returns the
 * applicable comorphisms with their (optional) `core:short_name`
 * surfaced for the Combobox label.
 */
async function queryApplicableComorphisms(
  eigen: Eigen,
  classIri: string,
  extraBranches: string[] = [],
): Promise<ApplicableComorphism[]> {
  // EigenQL — return the matching comorphism IRIs. `short_name` is
  // optional on a `MergeComorphism` (the ESL compiler doesn't emit
  // it for `merge_comorphism` declarations), and the current
  // EigenQL surface doesn't have an OPTIONAL pattern, so binding
  // `short_name` in the MATCH would filter out every comorphism
  // that lacks it. We resolve the short name out-of-band via a
  // follow-up inspect call per row — cheap (one round-trip per
  // applicable witness, usually 1-3) and keeps the Combobox label
  // human-readable when one exists.
  //
  // D38 §4 — the user-supplied `extraBranches` extend the picker's
  // visibility to witnesses on sibling branches outside the merge
  // span. We query the default branch (no `branch` option) plus
  // each named branch and union the results — see D38 §10.1 for
  // why we don't ask EigenQL to take a branch set in one go.
  const eigenql = `
USING "urn:eigenius:core:MergeComorphism"

MATCH MergeComorphism(?c) {
    "urn:eigenius:core:merge_target_class": ?cls
}
WHERE ?cls = ${JSON.stringify(classIri)}
RETURN [] {
    iri: ?c
}
ORDER BY ?c
`;
  const runOne = async (branch?: string): Promise<string[]> => {
    const resp = branch === undefined
      ? await eigen.query(eigenql)
      : await eigen.query(eigenql, { branch });
    if (!resp.success) {
      throw new Error(resp.error || "comorphism query failed");
    }
    const decoded = decodeResultDocument(resp.document);
    const out: string[] = [];
    for (const row of decoded.rows) {
      for (const [key, value] of row.values) {
        if (typeof value !== "string") continue;
        if (key.endsWith(":iri")) {
          out.push(value);
          break;
        }
      }
    }
    return out;
  };
  const allIris = new Set<string>();
  for (const iri of await runOne()) allIris.add(iri);
  for (const branch of extraBranches) {
    try {
      for (const iri of await runOne(branch)) allIris.add(iri);
    } catch {
      // Unknown / unreachable branch — skip silently. The kernel's
      // resolver applies the same "best-effort" semantics to
      // `witness_search_branches` (D38 §5.4), so a stale picker
      // entry doesn't surface as a query error.
    }
  }
  const iris = Array.from(allIris).sort();
  return await Promise.all(
    iris.map(async (iri) => ({
      iri,
      shortName: await fetchShortName(eigen, iri),
    })),
  );
}

/**
 * Best-effort `core:short_name` lookup for the Combobox label.
 * Returns `null` on any failure (not-found, parse error, network) —
 * the picker falls back to the IRI in that case.
 */
async function fetchShortName(eigen: Eigen, iri: string): Promise<string | null> {
  try {
    const resp = await eigen.inspect(iri);
    if (!resp.found) return null;
    const decoded = cborDecode(resp.resource) as Record<string, unknown> | null;
    if (decoded === null || typeof decoded !== "object") return null;
    const v = decoded["urn:eigenius:core:short_name"];
    return typeof v === "string" ? v : null;
  } catch {
    return null;
  }
}
