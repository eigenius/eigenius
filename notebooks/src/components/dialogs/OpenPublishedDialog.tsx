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
 * Modal dialog for opening a previously-published Notebook from the
 * active layer chain (D22 Phase 6 polish).
 *
 * Two text inputs (title, description) compose into LIKE filters
 * against `notebook:title` and `notebook:description`. The matching
 * notebooks render as a vertical card list — title at top, IRI in
 * monospaced caption, description with three-line clamp + a Show
 * more / Show less toggle, and the saved-at timestamp on the right.
 *
 * The card layout was chosen over a DataGrid because descriptions
 * are commonly long, multi-sentence prose; cards give the dialog
 * room to breathe and let each row decide its own height when the
 * user expands a description.
 *
 * The search fires automatically with a short debounce so the user
 * sees results as they type.
 */

import { useEffect, useState } from "react";
import {
  Body1,
  Body1Strong,
  Button,
  Caption1,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Input,
  makeStyles,
  mergeClasses,
  shorthands,
  Spinner,
  tokens,
} from "@fluentui/react-components";
import { useEigen } from "../../runtime/EigenProvider";
import {
  loadPublishedNotebook,
  type PublishedNotebookSummary,
  searchPublishedNotebooks,
} from "../../runtime/publishedNotebooks";
import type { NotebookJson } from "../../persistence/notebook-format";

const useStyles = makeStyles({
  surface: {
    width: "min(900px, 95vw)",
    maxWidth: "none",
  },
  body: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalM,
  },
  filterRow: {
    display: "grid",
    gridTemplateColumns: "1fr 1fr",
    gap: tokens.spacingHorizontalM,
  },
  resultsArea: {
    minHeight: "260px",
    maxHeight: "min(60vh, 520px)",
    overflowY: "auto",
    padding: tokens.spacingVerticalXS,
  },
  cardList: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  card: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXS,
    padding: tokens.spacingVerticalM,
    ...shorthands.border("1px", "solid", tokens.colorNeutralStroke2),
    borderRadius: tokens.borderRadiusMedium,
    background: tokens.colorNeutralBackground1,
    cursor: "pointer",
    textAlign: "left",
    transition: "border-color 100ms ease, background 100ms ease",
    ":hover": {
      ...shorthands.borderColor(tokens.colorNeutralStroke1),
      background: tokens.colorNeutralBackground1Hover,
    },
    ":focus-visible": {
      outline: `2px solid ${tokens.colorBrandStroke1}`,
      outlineOffset: "1px",
    },
  },
  cardSelected: {
    ...shorthands.borderColor(tokens.colorBrandStroke1),
    background: tokens.colorBrandBackground2,
    ":hover": {
      ...shorthands.borderColor(tokens.colorBrandStroke1),
      background: tokens.colorBrandBackground2Hover,
    },
  },
  cardTitleRow: {
    display: "flex",
    justifyContent: "space-between",
    alignItems: "baseline",
    gap: tokens.spacingHorizontalM,
  },
  cardTitle: {
    minWidth: 0,
    flex: 1,
  },
  cardSaved: {
    color: tokens.colorNeutralForeground3,
    fontVariantNumeric: "tabular-nums",
    whiteSpace: "nowrap",
  },
  cardIri: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase100,
    color: tokens.colorNeutralForeground3,
    wordBreak: "break-all",
  },
  cardDescription: {
    color: tokens.colorNeutralForeground2,
    overflow: "hidden",
    display: "-webkit-box",
    WebkitLineClamp: 3,
    WebkitBoxOrient: "vertical",
    whiteSpace: "pre-wrap",
  },
  cardDescriptionExpanded: {
    display: "block",
    WebkitLineClamp: "unset",
  },
  cardDescriptionEmpty: {
    color: tokens.colorNeutralForeground3,
    fontStyle: "italic",
  },
  showMoreButton: {
    alignSelf: "flex-start",
    padding: 0,
    minWidth: 0,
    color: tokens.colorBrandForeground1,
    fontSize: tokens.fontSizeBase200,
  },
  status: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    color: tokens.colorNeutralForeground3,
    padding: tokens.spacingVerticalM,
  },
  errorStatus: {
    color: tokens.colorPaletteRedForeground1,
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    padding: tokens.spacingVerticalM,
  },
});

const SEARCH_DEBOUNCE_MS = 300;
const DESCRIPTION_CLAMP_LENGTH = 240;

export interface OpenPublishedDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Called once the user picks a card and confirms; the dialog closes itself afterwards. */
  onPicked: (notebook: NotebookJson) => void;
}

type SearchState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ready"; results: readonly PublishedNotebookSummary[] }
  | { kind: "error"; message: string };

export function OpenPublishedDialog(
  { open, onOpenChange, onPicked }: OpenPublishedDialogProps,
) {
  const styles = useStyles();
  const eigen = useEigen();
  const [titleQuery, setTitleQuery] = useState("");
  const [descQuery, setDescQuery] = useState("");
  const [search, setSearch] = useState<SearchState>({ kind: "idle" });
  const [selectedIri, setSelectedIri] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);

  // Reset on each open so the dialog starts fresh.
  useEffect(() => {
    if (open) {
      setSelectedIri(null);
      setOpenError(null);
      setSearch({ kind: "loading" });
    }
  }, [open]);

  // Debounced search on filter change.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setSearch((prev) => prev.kind === "ready" ? prev : { kind: "loading" });
    const handle = setTimeout(() => {
      searchPublishedNotebooks(eigen, {
        titleQuery,
        descriptionQuery: descQuery,
      })
        .then((results) => {
          if (!cancelled) setSearch({ kind: "ready", results });
        })
        .catch((err: unknown) => {
          if (!cancelled) {
            setSearch({
              kind: "error",
              message: err instanceof Error ? err.message : String(err),
            });
          }
        });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(handle);
    };
  }, [eigen, open, titleQuery, descQuery]);

  const onConfirmOpen = async () => {
    if (!selectedIri) return;
    setOpening(true);
    setOpenError(null);
    try {
      const json = await loadPublishedNotebook(eigen, selectedIri);
      onPicked(json);
      onOpenChange(false);
    } catch (err) {
      setOpenError(err instanceof Error ? err.message : String(err));
    } finally {
      setOpening(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(_e, data) => onOpenChange(data.open)}
      modalType="modal"
    >
      <DialogSurface className={styles.surface}>
        <DialogBody>
          <DialogTitle>Open published notebook</DialogTitle>
          <DialogContent>
            <div className={styles.body}>
              <div className={styles.filterRow}>
                <Field label="Title contains">
                  <Input
                    value={titleQuery}
                    placeholder="e.g. kinase"
                    onChange={(_e, data) => setTitleQuery(data.value)}
                  />
                </Field>
                <Field label="Description contains">
                  <Input
                    value={descQuery}
                    placeholder="e.g. assay"
                    onChange={(_e, data) => setDescQuery(data.value)}
                  />
                </Field>
              </div>
              <div className={styles.resultsArea}>
                {search.kind === "loading" && (
                  <div className={styles.status}>
                    <Spinner size="tiny" />
                    <Caption1>searching…</Caption1>
                  </div>
                )}
                {search.kind === "error" && (
                  <Caption1 className={styles.errorStatus}>
                    {search.message}
                  </Caption1>
                )}
                {search.kind === "ready" && search.results.length === 0 && (
                  <Caption1 className={styles.status}>
                    no published notebooks match these filters
                  </Caption1>
                )}
                {search.kind === "ready" && search.results.length > 0 && (
                  <div className={styles.cardList}>
                    {search.results.map((item) => (
                      <ResultCard
                        key={item.iri}
                        item={item}
                        selected={selectedIri === item.iri}
                        onSelect={() => setSelectedIri(item.iri)}
                        onActivate={() => {
                          setSelectedIri(item.iri);
                          void onConfirmOpen();
                        }}
                      />
                    ))}
                  </div>
                )}
              </div>
              {openError && (
                <Caption1 className={styles.errorStatus}>{openError}</Caption1>
              )}
            </div>
          </DialogContent>
          <DialogActions>
            <Button appearance="secondary" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              appearance="primary"
              disabled={!selectedIri || opening}
              icon={opening ? <Spinner size="tiny" /> : undefined}
              onClick={() => {
                void onConfirmOpen();
              }}
            >
              Open
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}

interface ResultCardProps {
  item: PublishedNotebookSummary;
  selected: boolean;
  onSelect: () => void;
  onActivate: () => void;
}

function ResultCard({ item, selected, onSelect, onActivate }: ResultCardProps) {
  const styles = useStyles();
  const [expanded, setExpanded] = useState(false);
  const isLong = item.description.length > DESCRIPTION_CLAMP_LENGTH;

  return (
    <div
      role="option"
      aria-selected={selected}
      tabIndex={0}
      className={mergeClasses(styles.card, selected && styles.cardSelected)}
      onClick={onSelect}
      onDoubleClick={onActivate}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
    >
      <div className={styles.cardTitleRow}>
        <Body1Strong className={styles.cardTitle}>{item.title}</Body1Strong>
        <Caption1 className={styles.cardSaved}>
          {formatModified(item.modified)}
        </Caption1>
      </div>
      {item.description
        ? (
          <Body1
            className={mergeClasses(
              styles.cardDescription,
              expanded && styles.cardDescriptionExpanded,
            )}
          >
            {item.description}
          </Body1>
        )
        : <Body1 className={styles.cardDescriptionEmpty}>no description</Body1>}
      {isLong && (
        <Button
          appearance="transparent"
          size="small"
          className={styles.showMoreButton}
          onClick={(e) => {
            e.stopPropagation();
            setExpanded((v) => !v);
          }}
        >
          {expanded ? "Show less" : "Show more"}
        </Button>
      )}
      <Caption1 className={styles.cardIri}>{item.iri}</Caption1>
    </div>
  );
}

function formatModified(iso: string): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toISOString().replace("T", " ").slice(0, 16) + " UTC";
}
