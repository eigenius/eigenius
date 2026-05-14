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
 * D36 §6.4 — Restructure strategy editor.
 *
 * Four sub-fields: affected class (read-only, derived from the
 * conflict), new parent IRI (with a conditional "Define the new
 * class" section when the IRI is fresh), classes-under-new
 * multi-select with free-IRI input, and the
 * `affected_class_under_new` toggle.
 *
 * The new-class definition is a mini resource builder. The user
 * supplies short_name + description; the editor synthesises the
 * Eigon-JSON shape (`{ "@id": …, "urn:eigenius:core:is_a":
 * ["urn:eigenius:core:Class"], "urn:eigenius:core:short_name": …,
 * "urn:eigenius:core:description": … }`) and ships it as the
 * `newParentDefJson` field on the wire.
 *
 * "New parent exists in chain" detection is deferred to a follow-up
 * (chain-browser helper, D36 §9.3). For now the editor exposes a
 * toggle: "new parent already exists in the chain" vs "I'm
 * defining it here." When toggled to "exists," the resolution ships
 * without a `new_parent_def_json` and the kernel rejects with a
 * typed error if the IRI isn't in the chain.
 */
import { useEffect, useMemo, useState } from "react";
import {
  Caption1,
  Checkbox,
  Divider,
  Field,
  Input,
  Subtitle2,
  Switch,
  Textarea,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { Dismiss20Regular } from "@fluentui/react-icons";
import { Button } from "@fluentui/react-components";
import type { MergeResolutionWire } from "@eigenius/client";

const useStyles = makeStyles({
  section: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  divider: {
    marginTop: tokens.spacingVerticalS,
    marginBottom: tokens.spacingVerticalS,
  },
  affectedClass: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase300,
  },
  list: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXXS,
    paddingLeft: tokens.spacingHorizontalXS,
  },
  listRow: {
    display: "flex",
    gap: tokens.spacingHorizontalS,
    alignItems: "center",
  },
  iri: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    flex: 1,
  },
  addRow: {
    display: "flex",
    gap: tokens.spacingHorizontalS,
    alignItems: "flex-end",
  },
});

export interface RestructureEditorProps {
  conflictId: string;
  affectedClass: string;
  onChange: (next: MergeResolutionWire | undefined) => void;
}

export function RestructureEditor(
  { conflictId, affectedClass, onChange }: RestructureEditorProps,
) {
  const styles = useStyles();

  const [newParent, setNewParent] = useState("");
  // Toggle: true = the user is introducing `new_parent` as a fresh
  // Class. False = the user is attaching to a parent that already
  // exists in the chain (no `new_parent_def_json` shipped). Defaults
  // to true (the common case for the D20 §6.4 motivating example).
  const [isNew, setIsNew] = useState(true);
  const [shortName, setShortName] = useState("");
  const [description, setDescription] = useState("");
  const [classesUnderNew, setClassesUnderNew] = useState<string[]>([]);
  const [newClassUnderNew, setNewClassUnderNew] = useState("");
  const [affectedUnderNew, setAffectedUnderNew] = useState(true);

  const newParentDefJson = useMemo(() => {
    if (!isNew) return "";
    if (newParent.trim() === "") return "";
    // Synthesise the Eigon-JSON Class resource definition. Keys are
    // the canonical core-namespace IRIs that the kernel's
    // `eigon_json::parse_embedded` expects; values are strings for
    // string-typed properties and arrays of IRI strings for
    // resource_array properties (the format the core ontology's
    // is_a / parent_classes round-trip through).
    const def: Record<string, unknown> = {
      "@id": newParent.trim(),
      "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
    };
    if (shortName.trim()) {
      def["urn:eigenius:core:short_name"] = shortName.trim();
    }
    if (description.trim()) {
      def["urn:eigenius:core:description"] = description.trim();
    }
    return JSON.stringify(def);
  }, [isNew, newParent, shortName, description]);

  useEffect(() => {
    if (newParent.trim() === "") {
      onChange(undefined);
      return;
    }
    if (isNew && shortName.trim() === "") {
      // Fresh classes need at least a short_name (core ontology's
      // `Class` `requires` includes it). Keep the resolution
      // unsubmittable until the user provides one.
      onChange(undefined);
      return;
    }
    const resolution: MergeResolutionWire = {
      $typeName: "eigenius.v1.MergeResolutionWire",
      conflictId,
      strategy: {
        case: "restructure",
        value: {
          $typeName: "eigenius.v1.RestructureStrategy",
          affectedClass,
          newParent: newParent.trim(),
          newParentDefJson,
          classesUnderNew,
          affectedClassUnderNew: affectedUnderNew,
        },
      },
    };
    onChange(resolution);
  }, [
    conflictId,
    affectedClass,
    newParent,
    isNew,
    shortName,
    newParentDefJson,
    classesUnderNew,
    affectedUnderNew,
    onChange,
  ]);

  const addClassUnderNew = () => {
    const trimmed = newClassUnderNew.trim();
    if (trimmed === "") return;
    if (classesUnderNew.includes(trimmed)) {
      setNewClassUnderNew("");
      return;
    }
    setClassesUnderNew([...classesUnderNew, trimmed]);
    setNewClassUnderNew("");
  };

  const removeClassUnderNew = (iri: string) => {
    setClassesUnderNew(classesUnderNew.filter((i) => i !== iri));
  };

  return (
    <>
      <Subtitle2>Affected class</Subtitle2>
      <Caption1 className={styles.affectedClass}>{affectedClass}</Caption1>

      <Divider className={styles.divider} />
      <Subtitle2>New parent</Subtitle2>
      <Field
        label="New parent IRI"
        hint={
          isNew
            ? "Must be a fresh IRI not already in the chain."
            : "Must already resolve in the chain."
        }
      >
        <Input
          value={newParent}
          onChange={(_, data) => setNewParent(data.value)}
          placeholder="urn:project:Animal"
        />
      </Field>
      <Switch
        checked={isNew}
        onChange={(_, data) => setIsNew(data.checked)}
        label="Defining this parent as a new class here"
      />
      {isNew && (
        <div className={styles.section}>
          <Field
            label="Short name"
            required
            hint="The Class's `short_name` (required by the core ontology)."
          >
            <Input
              value={shortName}
              onChange={(_, data) => setShortName(data.value)}
              placeholder="Animal"
            />
          </Field>
          <Field label="Description">
            <Textarea
              value={description}
              onChange={(_, data) => setDescription(data.value)}
              placeholder="Common parent for Mammal and Reptile."
              rows={2}
            />
          </Field>
        </div>
      )}

      <Divider className={styles.divider} />
      <Subtitle2>Existing classes to subclass it</Subtitle2>
      <div className={styles.list}>
        {classesUnderNew.length === 0 && (
          <Caption1>
            No classes selected. Add IRIs of classes you want to point
            at the new parent.
          </Caption1>
        )}
        {classesUnderNew.map((iri) => (
          <div key={iri} className={styles.listRow}>
            <code className={styles.iri}>{iri}</code>
            <Button
              size="small"
              appearance="subtle"
              icon={<Dismiss20Regular />}
              aria-label={`Remove ${iri}`}
              onClick={() => removeClassUnderNew(iri)}
            />
          </div>
        ))}
      </div>
      <div className={styles.addRow}>
        <Field label="Class IRI to add">
          <Input
            value={newClassUnderNew}
            onChange={(_, data) => setNewClassUnderNew(data.value)}
            placeholder="urn:project:Mammal"
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                addClassUnderNew();
              }
            }}
          />
        </Field>
        <Button onClick={addClassUnderNew}>Add</Button>
      </div>

      <Divider className={styles.divider} />
      <Subtitle2>Affected class placement</Subtitle2>
      <Checkbox
        checked={affectedUnderNew}
        onChange={(_, data) => setAffectedUnderNew(!!data.checked)}
        label={
          <span>
            <code>{affectedClass}</code> subclasses{" "}
            <code>{newParent.trim() || "<new parent>"}</code>{" "}
            directly (replaces its current parents)
          </span>
        }
      />
    </>
  );
}
