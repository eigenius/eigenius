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
 * D36 §7 — Cascade preview + acknowledgment checklist.
 *
 * Renders one section per kind of `CascadeItemWire`. Each item has
 * an "I understand" checkbox keyed by `itemId`; the parent
 * (`MergeResolutionFlow`) gates the commit button on every box
 * being ticked. D20 §8 mandates the ack discipline; the friction
 * is intentional — the user is being asked to see N consequences
 * before committing.
 */
import { Fragment, useState } from "react";
import {
  Button,
  Caption1,
  Checkbox,
  makeStyles,
  Subtitle2,
  tokens,
} from "@fluentui/react-components";
import type { CascadeItemWire } from "@eigenius/client";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalL,
  },
  section: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  sectionHeader: {
    display: "flex",
    gap: tokens.spacingHorizontalS,
    alignItems: "baseline",
  },
  count: {
    color: tokens.colorNeutralForeground3,
  },
  item: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXXS,
    paddingLeft: tokens.spacingHorizontalM,
  },
  itemBody: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground2,
  },
  emptyBanner: {
    color: tokens.colorNeutralForeground2,
    fontStyle: "italic",
  },
});

export interface CascadePreviewPaneProps {
  items: readonly CascadeItemWire[];
  acknowledged: Readonly<Record<string, boolean>>;
  onToggle: (itemId: string) => void;
}

export function CascadePreviewPane(
  { items, acknowledged, onToggle }: CascadePreviewPaneProps,
) {
  const styles = useStyles();

  if (items.length === 0) {
    return (
      <Caption1 className={styles.emptyBanner}>
        Resolutions are self-contained — no downstream consequences.
      </Caption1>
    );
  }

  // Group by kind so the user sees one section per consequence type.
  const buckets = bucketByKind(items);

  return (
    <div className={styles.root}>
      {buckets.orphanedReferences.length > 0 && (
        <Section
          title="Orphaned references"
          items={buckets.orphanedReferences}
          renderItem={(item) => (
            <OrphanedReferenceRow
              key={item.itemId}
              item={item}
              checked={!!acknowledged[item.itemId]}
              onToggle={onToggle}
              styles={styles}
            />
          )}
          styles={styles}
        />
      )}
      {buckets.orphanedTypings.length > 0 && (
        <Section
          title="Orphaned typing"
          items={buckets.orphanedTypings}
          renderItem={(item) => (
            <OrphanedTypingRow
              key={item.itemId}
              item={item}
              checked={!!acknowledged[item.itemId]}
              onToggle={onToggle}
              styles={styles}
            />
          )}
          styles={styles}
        />
      )}
      {buckets.invalidatedSignatures.length > 0 && (
        <Section
          title="Invalidated signatures (informational)"
          items={buckets.invalidatedSignatures}
          renderItem={(item) => (
            <InfoRow
              key={item.itemId}
              itemId={item.itemId}
              label={`Program ${labelForInvalidatedSignature(item)}`}
              checked={!!acknowledged[item.itemId]}
              onToggle={onToggle}
              styles={styles}
            />
          )}
          styles={styles}
        />
      )}
      {buckets.invalidatedTraces.length > 0 && (
        <Section
          title="Invalidated traces (informational)"
          items={buckets.invalidatedTraces}
          renderItem={(item) => (
            <InfoRow
              key={item.itemId}
              itemId={item.itemId}
              label={labelForInvalidatedTrace(item)}
              checked={!!acknowledged[item.itemId]}
              onToggle={onToggle}
              styles={styles}
            />
          )}
          styles={styles}
        />
      )}
    </div>
  );
}

/**
 * Items per section past which the section folds and surfaces a
 * "Show all N" toggle. Picked at 20 because real-world cascade
 * sections rarely exceed it; when they do, dumping 100s of
 * checkboxes inline hurts both render time and the user's ability
 * to track which items they've ticked. The folded view shows the
 * first `SECTION_FOLD_THRESHOLD` items + a toggle.
 */
const SECTION_FOLD_THRESHOLD = 20;

function Section<T>({
  title,
  items,
  renderItem,
  styles,
}: {
  title: string;
  items: readonly T[];
  renderItem: (item: T) => React.ReactNode;
  styles: ReturnType<typeof useStyles>;
}) {
  const [expanded, setExpanded] = useState(false);
  const folded = items.length > SECTION_FOLD_THRESHOLD && !expanded;
  const visible = folded ? items.slice(0, SECTION_FOLD_THRESHOLD) : items;
  return (
    <section className={styles.section}>
      <div className={styles.sectionHeader}>
        <Subtitle2>{title}</Subtitle2>
        <Caption1 className={styles.count}>({items.length})</Caption1>
      </div>
      {visible.map(renderItem)}
      {items.length > SECTION_FOLD_THRESHOLD && (
        <Button
          size="small"
          appearance="subtle"
          onClick={() => setExpanded(!expanded)}
        >
          {folded
            ? `Show all ${items.length}`
            : `Show first ${SECTION_FOLD_THRESHOLD}`}
        </Button>
      )}
    </section>
  );
}

function OrphanedReferenceRow(
  { item, checked, onToggle, styles }: {
    item: CascadeItemWire;
    checked: boolean;
    onToggle: (id: string) => void;
    styles: ReturnType<typeof useStyles>;
  },
) {
  const kind = item.kind;
  if (kind?.case !== "orphanedReference") return null;
  const path = kind.value.propertyPath.length === 0
    ? "<root>"
    : kind.value.propertyPath.join("/");
  return (
    <div className={styles.item}>
      <Checkbox
        label={
          <span>
            <code>{kind.value.resource}</code> →{" "}
            <code>{kind.value.droppedTarget}</code>
          </span>
        }
        checked={checked}
        onChange={() => onToggle(item.itemId)}
      />
      <Caption1 className={styles.itemBody}>
        at <code>{path}</code> — reference will dangle post-merge.
      </Caption1>
    </div>
  );
}

function OrphanedTypingRow(
  { item, checked, onToggle, styles }: {
    item: CascadeItemWire;
    checked: boolean;
    onToggle: (id: string) => void;
    styles: ReturnType<typeof useStyles>;
  },
) {
  const kind = item.kind;
  if (kind?.case !== "orphanedTyping") return null;
  const count = kind.value.affectedResources.length;
  return (
    <div className={styles.item}>
      <Checkbox
        label={
          <span>
            <code>{kind.value.class}</code> — {count} resource(s) will lose
            their typing.
          </span>
        }
        checked={checked}
        onChange={() => onToggle(item.itemId)}
      />
      {count > 0 && count <= 5 && (
        <Caption1 className={styles.itemBody}>
          Affected:{" "}
          {kind.value.affectedResources.map((r, i) => (
            <Fragment key={r}>
              {i > 0 && ", "}
              <code>{r}</code>
            </Fragment>
          ))}
        </Caption1>
      )}
      {count > 5 && (
        <Caption1 className={styles.itemBody}>
          Affected (first 5):{" "}
          {kind.value.affectedResources.slice(0, 5).map((r, i) => (
            <Fragment key={r}>
              {i > 0 && ", "}
              <code>{r}</code>
            </Fragment>
          ))}
          {" "}and {count - 5} more.
        </Caption1>
      )}
    </div>
  );
}

function InfoRow(
  { itemId, label, checked, onToggle, styles }: {
    itemId: string;
    label: string;
    checked: boolean;
    onToggle: (id: string) => void;
    styles: ReturnType<typeof useStyles>;
  },
) {
  return (
    <div className={styles.item}>
      <Checkbox
        label={label}
        checked={checked}
        onChange={() => onToggle(itemId)}
      />
    </div>
  );
}

function labelForInvalidatedSignature(item: CascadeItemWire): string {
  const kind = item.kind;
  if (kind?.case !== "invalidatedSignature") return item.itemId;
  return `${kind.value.program}: ${kind.value.signatureProblem}`;
}

function labelForInvalidatedTrace(item: CascadeItemWire): string {
  const kind = item.kind;
  if (kind?.case !== "invalidatedTrace") return item.itemId;
  return `Trace ${kind.value.trace}: ${kind.value.reason}`;
}

interface CascadeBuckets {
  orphanedReferences: CascadeItemWire[];
  orphanedTypings: CascadeItemWire[];
  invalidatedSignatures: CascadeItemWire[];
  invalidatedTraces: CascadeItemWire[];
}

function bucketByKind(items: readonly CascadeItemWire[]): CascadeBuckets {
  const buckets: CascadeBuckets = {
    orphanedReferences: [],
    orphanedTypings: [],
    invalidatedSignatures: [],
    invalidatedTraces: [],
  };
  for (const item of items) {
    switch (item.kind?.case) {
      case "orphanedReference":
        buckets.orphanedReferences.push(item);
        break;
      case "orphanedTyping":
        buckets.orphanedTypings.push(item);
        break;
      case "invalidatedSignature":
        buckets.invalidatedSignatures.push(item);
        break;
      case "invalidatedTrace":
        buckets.invalidatedTraces.push(item);
        break;
    }
  }
  return buckets;
}
