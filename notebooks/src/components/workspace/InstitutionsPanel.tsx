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
 * Institutions rail destination — D34 §9.2.
 *
 * Left: filterable list of institutions installed at the active
 * branch's tip. Columns: Name · Runtime · Comorphisms · Query
 * classes. Right: detail panel showing the runtime, the institution
 * IRI, the full list of comorphisms (from → to + transformation
 * IRI), and the QueryClass declarations with dispatch roles.
 *
 * Built on the D34 §G.8 `InstitutionInfo` enrichment — a single
 * `eigen.listInstitutions()` call returns everything the panel
 * renders. No per-row `Inspect` fan-out.
 *
 * The "Inspect raw resource" action opens the unfiltered Eigon
 * resource via `eigen.inspect(iri)` + the shared `ResourceInspector`
 * component. "View install layer in history" needs the §G.6
 * cursored-history endpoint (per-layer `defined_iris`) — disabled
 * until that lands.
 */

import { useEffect, useMemo, useState } from "react";
import {
  Badge,
  Body1,
  Body1Strong,
  Button,
  Caption1,
  Dialog,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Divider,
  Input,
  makeStyles,
  MessageBar,
  MessageBarBody,
  MessageBarTitle,
  Spinner,
  Subtitle1,
  tokens,
  Tooltip,
} from "@fluentui/react-components";
import {
  ArrowSync20Regular,
  DocumentBulletList20Regular,
  Open20Regular,
  Search20Regular,
} from "@fluentui/react-icons";
import {
  type ComorphismDecl,
  DispatchRole,
  type InstitutionInfo,
  type QueryClassDecl,
  RuntimeKind,
} from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";
import { ResourceInspector } from "../output/ResourceInspector";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    height: "100%",
    minHeight: 0,
  },
  header: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalM,
    padding: `${tokens.spacingVerticalM} ${tokens.spacingHorizontalXXL}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  headerHint: {
    color: tokens.colorNeutralForeground3,
  },
  headerSpacer: { flex: 1 },
  body: {
    flex: 1,
    minHeight: 0,
    overflow: "hidden",
    display: "grid",
    gridTemplateColumns: "minmax(0, 1fr) minmax(0, 1fr)",
  },
  list: {
    overflowY: "auto",
    borderRight: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  listFilter: {
    padding: tokens.spacingVerticalS,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  table: {
    width: "100%",
    borderCollapse: "collapse",
    fontSize: tokens.fontSizeBase300,
  },
  th: {
    textAlign: "left",
    color: tokens.colorNeutralForeground3,
    fontWeight: tokens.fontWeightSemibold,
    fontSize: tokens.fontSizeBase200,
    textTransform: "uppercase",
    letterSpacing: "0.04em",
    padding: `${tokens.spacingVerticalS} ${tokens.spacingHorizontalM}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    position: "sticky",
    top: 0,
    background: tokens.colorNeutralBackground1,
    zIndex: 1,
  },
  td: {
    padding: `${tokens.spacingVerticalS} ${tokens.spacingHorizontalM}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke3}`,
    verticalAlign: "middle",
    cursor: "pointer",
  },
  rowActive: {
    background: tokens.colorNeutralBackground2Selected,
  },
  numCell: {
    color: tokens.colorNeutralForeground3,
    textAlign: "right",
  },
  detail: {
    overflowY: "auto",
    padding: tokens.spacingVerticalL,
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalM,
  },
  iri: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
    wordBreak: "break-all",
  },
  detailGrid: {
    display: "grid",
    gridTemplateColumns: "max-content 1fr",
    columnGap: tokens.spacingHorizontalM,
    rowGap: tokens.spacingVerticalXS,
  },
  metricLabel: {
    color: tokens.colorNeutralForeground3,
  },
  declList: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  declItem: {
    padding: tokens.spacingVerticalS,
    background: tokens.colorNeutralBackground2,
    borderRadius: tokens.borderRadiusMedium,
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXXS,
  },
  declHeader: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    fontWeight: tokens.fontWeightSemibold,
  },
  declSub: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
  },
  dispatchRoles: {
    display: "flex",
    gap: tokens.spacingHorizontalXS,
    flexWrap: "wrap",
    marginTop: tokens.spacingVerticalXXS,
  },
  actions: {
    display: "flex",
    gap: tokens.spacingHorizontalS,
    flexWrap: "wrap",
  },
  emptyState: {
    padding: tokens.spacingVerticalXXL,
    textAlign: "center",
    color: tokens.colorNeutralForeground3,
  },
  loadingState: {
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    gap: tokens.spacingHorizontalS,
    padding: tokens.spacingVerticalXXL,
    color: tokens.colorNeutralForeground3,
  },
  inspectSurface: {
    width: "min(720px, 95vw)",
    maxWidth: "none",
  },
});

interface InspectState {
  iri: string;
  status: "loading" | { kind: "ready"; bytes: Uint8Array } | {
    kind: "error";
    message: string;
  };
}

export function InstitutionsPanel() {
  const styles = useStyles();
  const eigen = useEigen();
  const activeBranch = useNotebookStore((s) => s.activeBranch);

  const [list, setList] = useState<readonly InstitutionInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [inspect, setInspect] = useState<InspectState | null>(null);

  const refresh = async () => {
    setError(null);
    try {
      const resp = await eigen.listInstitutions();
      setList(resp);
    } catch (err) {
      setList([]);
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  useEffect(() => {
    void refresh();
    // Re-fetch when the active branch changes — institutions are
    // branch-scoped (D34 §9.2.1) and the set legitimately differs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eigen, activeBranch]);

  const filtered = useMemo(() => {
    if (!list) return null;
    const needle = filter.trim().toLowerCase();
    if (!needle) return list;
    return list.filter((i) =>
      i.name.toLowerCase().includes(needle) ||
      i.iri.toLowerCase().includes(needle)
    );
  }, [list, filter]);

  // Auto-select the first row when the list loads or the selection is
  // no longer in the filtered set.
  useEffect(() => {
    if (!filtered || filtered.length === 0) {
      setSelected(null);
      return;
    }
    if (!selected || !filtered.some((i) => i.iri === selected)) {
      setSelected(filtered[0].iri);
    }
  }, [filtered, selected]);

  const selectedInfo = useMemo(
    () => filtered?.find((i) => i.iri === selected) ?? null,
    [filtered, selected],
  );

  const onInspect = async (iri: string) => {
    setInspect({ iri, status: "loading" });
    try {
      const resp = await eigen.inspect(iri);
      if (!resp.found) {
        setInspect({
          iri,
          status: { kind: "error", message: `resource ${iri} not found` },
        });
        return;
      }
      setInspect({ iri, status: { kind: "ready", bytes: resp.resource } });
    } catch (err) {
      setInspect({
        iri,
        status: {
          kind: "error",
          message: err instanceof Error ? err.message : String(err),
        },
      });
    }
  };

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <DocumentBulletList20Regular />
        <Subtitle1 as="h2">Institutions</Subtitle1>
        <Caption1 className={styles.headerHint}>
          installed at <strong>{activeBranch}</strong> · tip
        </Caption1>
        <span className={styles.headerSpacer} />
        <Tooltip content="Refresh" relationship="label">
          <Button
            size="small"
            appearance="subtle"
            icon={<ArrowSync20Regular />}
            onClick={() => void refresh()}
            aria-label="Refresh"
          />
        </Tooltip>
      </div>

      {error && (
        <MessageBar intent="warning">
          <MessageBarBody>{error}</MessageBarBody>
        </MessageBar>
      )}

      <div className={styles.body}>
        <div className={styles.list}>
          <div className={styles.listFilter}>
            <Input
              size="small"
              contentBefore={<Search20Regular />}
              placeholder="Filter by name or IRI"
              value={filter}
              onChange={(_e, data) => setFilter(data.value)}
            />
          </div>
          {filtered === null
            ? (
              <div className={styles.loadingState}>
                <Spinner size="tiny" />
                <Caption1>fetching institutions…</Caption1>
              </div>
            )
            : filtered.length === 0
            ? (
              <div className={styles.emptyState}>
                {(list?.length ?? 0) === 0
                  ? "No institutions installed at this layer."
                  : "No institutions match the filter."}
              </div>
            )
            : (
              <InstitutionsTable
                rows={filtered}
                selectedIri={selected}
                styles={styles}
                onSelect={setSelected}
              />
            )}
        </div>

        <div className={styles.detail}>
          {selectedInfo
            ? (
              <InstitutionDetail
                info={selectedInfo}
                styles={styles}
                onInspect={() => void onInspect(selectedInfo.iri)}
              />
            )
            : (
              <Caption1 className={styles.emptyState}>
                Select an institution to see its details.
              </Caption1>
            )}
        </div>
      </div>

      <InspectDialog
        state={inspect}
        styles={styles}
        onClose={() => setInspect(null)}
      />
    </div>
  );
}

interface InstitutionsTableProps {
  rows: readonly InstitutionInfo[];
  selectedIri: string | null;
  styles: ReturnType<typeof useStyles>;
  onSelect: (iri: string) => void;
}

function InstitutionsTable({
  rows,
  selectedIri,
  styles,
  onSelect,
}: InstitutionsTableProps) {
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th className={styles.th}>Name</th>
          <th className={styles.th}>Runtime</th>
          <th className={styles.th}>Comorphisms</th>
          <th className={styles.th}>Query classes</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => {
          const isActive = row.iri === selectedIri;
          return (
            <tr
              key={row.iri}
              className={isActive ? styles.rowActive : undefined}
              onClick={() => onSelect(row.iri)}
            >
              <td className={styles.td}>
                <Body1Strong>{row.name || shortenIri(row.iri)}</Body1Strong>
              </td>
              <td className={styles.td}>
                <RuntimeBadge kind={row.runtimeKind} />
              </td>
              <td className={`${styles.td} ${styles.numCell}`}>
                {row.comorphisms.length}
              </td>
              <td className={`${styles.td} ${styles.numCell}`}>
                {row.queryClasses.length}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

interface RuntimeBadgeProps {
  kind: RuntimeKind;
}

function RuntimeBadge({ kind }: RuntimeBadgeProps) {
  // Tint by category so the operator can scan the list at a glance.
  switch (kind) {
    case RuntimeKind.IN_PROCESS:
      return (
        <Badge appearance="tint" color="informative" size="small">
          in-process
        </Badge>
      );
    case RuntimeKind.EXTERNAL:
      return (
        <Badge appearance="tint" color="success" size="small">
          external
        </Badge>
      );
    default:
      return (
        <Badge appearance="outline" size="small">
          unspecified
        </Badge>
      );
  }
}

interface InstitutionDetailProps {
  info: InstitutionInfo;
  styles: ReturnType<typeof useStyles>;
  onInspect: () => void;
}

function InstitutionDetail({
  info,
  styles,
  onInspect,
}: InstitutionDetailProps) {
  return (
    <>
      <div>
        <Subtitle1 as="h3">
          {info.name || shortenIri(info.iri)}
        </Subtitle1>
        <Caption1 className={styles.iri}>{info.iri}</Caption1>
      </div>

      <div className={styles.detailGrid}>
        <Caption1 className={styles.metricLabel}>Runtime</Caption1>
        <RuntimeBadge kind={info.runtimeKind} />
        {info.requiresEnvironment && (
          <>
            <Caption1 className={styles.metricLabel}>
              Requires environment
            </Caption1>
            <span className={styles.iri}>{info.requiresEnvironment}</span>
          </>
        )}
        <Caption1 className={styles.metricLabel}>Installed at</Caption1>
        <Caption1>
          current head (per-layer install lineage lands with the §G.6 history
          endpoint)
        </Caption1>
      </div>

      <Divider />

      <div>
        <Body1Strong>Comorphisms ({info.comorphisms.length})</Body1Strong>
        {info.comorphisms.length === 0
          ? <Caption1>None declared.</Caption1>
          : (
            <div className={styles.declList}>
              {info.comorphisms.map((c) => (
                <ComorphismRow key={c.iri} c={c} styles={styles} />
              ))}
            </div>
          )}
      </div>

      <Divider />

      <div>
        <Body1Strong>
          Query classes ({info.queryClasses.length})
        </Body1Strong>
        {info.queryClasses.length === 0
          ? <Caption1>None declared.</Caption1>
          : (
            <div className={styles.declList}>
              {info.queryClasses.map((qc) => (
                <QueryClassRow key={qc.iri} qc={qc} styles={styles} />
              ))}
            </div>
          )}
      </div>

      <Divider />

      <div className={styles.actions}>
        <Button
          appearance="primary"
          icon={<Open20Regular />}
          onClick={onInspect}
        >
          Inspect raw resource
        </Button>
        {
          /* "View install layer in history" needs §G.6 (per-layer
            defined_iris aggregated by the history endpoint) — defer
            with a tooltip rather than ship a broken affordance. */
        }
        <Tooltip
          relationship="description"
          content="Cross-link to History needs the §G.6 history endpoint enrichment — coming alongside the chain/log surface."
        >
          <Button disabled>View install layer…</Button>
        </Tooltip>
      </div>
    </>
  );
}

interface ComorphismRowProps {
  c: ComorphismDecl;
  styles: ReturnType<typeof useStyles>;
}

function ComorphismRow({ c, styles }: ComorphismRowProps) {
  return (
    <div className={styles.declItem}>
      <span className={styles.declHeader}>
        {shortenIri(c.fromClass || "?")} <Caption1 as="span">→</Caption1>{" "}
        {shortenIri(c.toClass || "?")}
        {c.exact && (
          <Badge
            appearance="tint"
            color="brand"
            size="extra-small"
            style={{ marginLeft: 8 }}
          >
            exact
          </Badge>
        )}
      </span>
      <span className={styles.declSub}>
        program: {c.transformation || "(none)"}
      </span>
      <span className={styles.declSub}>{c.iri}</span>
    </div>
  );
}

interface QueryClassRowProps {
  qc: QueryClassDecl;
  styles: ReturnType<typeof useStyles>;
}

function QueryClassRow({ qc, styles }: QueryClassRowProps) {
  return (
    <div className={styles.declItem}>
      <span className={styles.declHeader}>
        {shortenIri(qc.iri)}
      </span>
      <span className={styles.declSub}>
        bound to {qc.queryClass}
      </span>
      <span className={styles.declSub}>
        result: {qc.resultClass}
      </span>
      <span className={styles.declSub}>handler: {qc.queryHandler}</span>
      <div className={styles.dispatchRoles}>
        {qc.dispatchRoles.map((role) => (
          <DispatchRoleBadge key={role} role={role} />
        ))}
      </div>
    </div>
  );
}

interface DispatchRoleBadgeProps {
  role: DispatchRole;
}

function DispatchRoleBadge({ role }: DispatchRoleBadgeProps) {
  switch (role) {
    case DispatchRole.ON_DEMAND:
      return (
        <Badge appearance="tint" color="informative" size="small">
          OnDemand
        </Badge>
      );
    case DispatchRole.AUTO_ON_LOAD:
      return (
        <Badge appearance="tint" color="warning" size="small">
          AutoOnLoad
        </Badge>
      );
    case DispatchRole.DECIDABLE:
      return (
        <Badge appearance="tint" color="success" size="small">
          Decidable
        </Badge>
      );
    default:
      return (
        <Badge appearance="outline" size="small">
          unspecified
        </Badge>
      );
  }
}

interface InspectDialogProps {
  state: InspectState | null;
  styles: ReturnType<typeof useStyles>;
  onClose: () => void;
}

function InspectDialog({ state, styles, onClose }: InspectDialogProps) {
  return (
    <Dialog
      open={state !== null}
      onOpenChange={(_e, data) => {
        if (!data.open) onClose();
      }}
    >
      <DialogSurface className={styles.inspectSurface}>
        <DialogBody>
          <DialogTitle>Inspect resource</DialogTitle>
          <DialogContent>
            {state && (
              <>
                <Caption1 className={styles.iri}>{state.iri}</Caption1>
                <div style={{ marginTop: tokens.spacingVerticalM }}>
                  {state.status === "loading"
                    ? <Spinner size="tiny" label="fetching resource" />
                    : "kind" in state.status && state.status.kind === "error"
                    ? (
                      <MessageBar intent="error">
                        <MessageBarBody>
                          <MessageBarTitle>Inspect failed</MessageBarTitle>
                          {state.status.message}
                        </MessageBarBody>
                      </MessageBar>
                    )
                    : (
                      <ResourceInspector
                        resource={(state.status as {
                          kind: "ready";
                          bytes: Uint8Array;
                        }).bytes}
                      />
                    )}
                </div>
              </>
            )}
          </DialogContent>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}

/** Render an IRI as `…<local part>` if it has a `:`, else verbatim. */
function shortenIri(iri: string): string {
  if (!iri) return "";
  const lastColon = iri.lastIndexOf(":");
  if (lastColon === -1 || lastColon === iri.length - 1) return iri;
  const local = iri.slice(lastColon + 1);
  if (local.length === iri.length) return iri;
  return local;
}

// Suppress unused-import warning for Body1 (used conditionally in
// future surfaces). Removed once the panel grows.
void Body1;
