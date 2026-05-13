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
 * Topology rail destination — D34 §9.3.
 *
 * Wraps the existing `TopologyGraphView` (previously a cell-output-
 * only renderer) as a top-level workspace panel. Root selection
 * defaults to the active branch's tip; a branch-picker Combobox
 * lets the operator re-root at any other branch's tip without
 * leaving the panel. The `Include resources` toggle controls whether
 * Class / Property / Resource / Institution nodes are surfaced — by
 * default they are (this is the "full topology" view), but turning
 * them off yields a pure layer-chain view useful for long chains.
 *
 * The cross-branch highlight pass from §9.3 (colour the active
 * branch's chain, render others in grey) needs a per-node attribution
 * to which branch a layer is reachable from — that's a topology-
 * walker enrichment, deferred alongside the §G.6 cursored-history
 * endpoint. The current panel renders one rooted graph at a time;
 * switching branches re-fetches against the new root.
 */

import { useEffect, useMemo, useState } from "react";
import {
  Button,
  Caption1,
  Combobox,
  Field,
  makeStyles,
  MessageBar,
  MessageBarBody,
  Option,
  Spinner,
  Subtitle1,
  Switch,
  tokens,
} from "@fluentui/react-components";
import { Apps20Regular, ArrowSync20Regular } from "@fluentui/react-icons";
import type { LayerTopologyResponse } from "@eigenius/client";
import { Pin16Regular } from "@fluentui/react-icons";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";
import { TopologyGraphView } from "../output/TopologyGraphView";

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
  headerSpacer: { flex: 1 },
  toolbar: {
    display: "flex",
    alignItems: "flex-end",
    gap: tokens.spacingHorizontalM,
    padding: `${tokens.spacingVerticalS} ${tokens.spacingHorizontalXXL}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    flexWrap: "wrap",
  },
  body: {
    flex: 1,
    minHeight: 0,
    overflow: "hidden",
    display: "flex",
    flexDirection: "column",
    padding: tokens.spacingVerticalM,
  },
  graphWrap: {
    // TopologyGraphView's root is a fixed 560px-tall card; let the
    // panel grow to fill the rail-pane height instead by letting the
    // wrapper drive the size and stretching the child to 100%.
    flex: 1,
    minHeight: 0,
    display: "flex",
  },
  loadingState: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    padding: tokens.spacingVerticalXXL,
    color: tokens.colorNeutralForeground3,
  },
  noBranches: {
    padding: tokens.spacingVerticalXXL,
    color: tokens.colorNeutralForeground3,
  },
  pinNotice: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalXS,
    color: tokens.colorPaletteYellowForeground1,
    padding: `${tokens.spacingVerticalXS} ${tokens.spacingHorizontalS}`,
    borderLeft: `1px solid ${tokens.colorNeutralStroke2}`,
  },
});

/** Render a hex `LayerId` as `aaaa…bbbb` for the toolbar pin label. */
function shortHash(hex: string): string {
  if (hex.length <= 10) return hex;
  return `${hex.slice(0, 4)}…${hex.slice(-4)}`;
}

export function TopologyPanel() {
  const styles = useStyles();
  const eigen = useEigen();

  const activeBranch = useNotebookStore((s) => s.activeBranch);
  const branches = useNotebookStore((s) => s.branches);
  const refreshBranches = useNotebookStore((s) => s.refreshBranches);
  const readPinLayerId = useNotebookStore((s) => s.readPinLayerId);
  const setReadPin = useNotebookStore((s) => s.setReadPin);

  // Root the graph at the chosen branch's tip. Defaults to the
  // active branch and tracks it when the user switches in the header.
  const [rootBranch, setRootBranch] = useState<string>(activeBranch);
  const [includeResources, setIncludeResources] = useState(true);
  const [topology, setTopology] = useState<LayerTopologyResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    void refreshBranches(eigen);
  }, [eigen, refreshBranches]);

  // When the workspace's active branch changes the user is implicitly
  // switching context; track that here too unless the user has
  // explicitly chosen a different rootBranch.
  useEffect(() => {
    setRootBranch((current) =>
      branches?.some((b) => b.name === current) ? current : activeBranch
    );
  }, [activeBranch, branches]);

  const rootBranchInfo = useMemo(
    () => branches?.find((b) => b.name === rootBranch) ?? null,
    [branches, rootBranch],
  );
  // Read-pin wins over the branch tip: this is how the History
  // panel's "Inspect resources" affordance routes the user here
  // with a specific layer in mind. Clearing the pin (via this
  // panel's "Use branch tip" button, or the header's "Return to
  // tip") returns to the branch-rooted default.
  const rootLayer = readPinLayerId ?? rootBranchInfo?.headLayer ?? null;

  const fetchTopology = async () => {
    if (!rootLayer) return;
    setLoading(true);
    setError(null);
    try {
      const resp = await eigen.layerTopology({
        rootLayer,
        maxDepth: 0,
        includeResources,
      });
      setTopology(resp);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void fetchTopology();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eigen, rootLayer, includeResources]);

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <Apps20Regular />
        <Subtitle1 as="h2">Topology</Subtitle1>
        <span className={styles.headerSpacer} />
        <Button
          size="small"
          appearance="subtle"
          icon={<ArrowSync20Regular />}
          disabled={loading || !rootLayer}
          onClick={() => void fetchTopology()}
        >
          {loading ? "Loading…" : "Refresh"}
        </Button>
      </div>
      <div className={styles.toolbar}>
        <Field
          label="Root branch"
          hint={readPinLayerId
            ? "Overridden by read-pin — see the pin notice below"
            : "Tip of this branch becomes the graph root"}
        >
          <Combobox
            value={rootBranch}
            selectedOptions={rootBranch ? [rootBranch] : []}
            onOptionSelect={(_e, data) => {
              if (data.optionValue) setRootBranch(data.optionValue);
            }}
            disabled={!branches || branches.length === 0 ||
              readPinLayerId !== null}
            placeholder={branches ? "Select a branch" : "(no branches)"}
          >
            {(branches ?? []).map((b) => (
              <Option key={b.name} value={b.name}>
                {b.name}
              </Option>
            ))}
          </Combobox>
        </Field>
        <Switch
          checked={includeResources}
          onChange={(_e, data) => setIncludeResources(data.checked === true)}
          label="Include resources"
        />
        <Caption1>
          Class, Property, Resource, and Institution nodes; off renders a pure
          layer chain.
        </Caption1>
        {readPinLayerId && (
          <div className={styles.pinNotice}>
            <Pin16Regular />
            <Caption1>
              Rooted at read-pin <code>{shortHash(readPinLayerId)}</code>
            </Caption1>
            <Button
              size="small"
              appearance="subtle"
              onClick={() => setReadPin(null)}
            >
              Use branch tip
            </Button>
          </div>
        )}
      </div>
      <div className={styles.body}>
        {error && (
          <MessageBar intent="error">
            <MessageBarBody>{error}</MessageBarBody>
          </MessageBar>
        )}
        {!rootLayer && !error && (
          <div className={styles.noBranches}>
            Loading active branch…
          </div>
        )}
        {rootLayer && topology === null && !error && (
          <div className={styles.loadingState}>
            <Spinner size="tiny" />
            <Caption1>fetching topology…</Caption1>
          </div>
        )}
        {rootLayer && topology && (
          <div className={styles.graphWrap}>
            <TopologyGraphView
              topology={topology}
              hideParentLayerEdges={false}
              title={readPinLayerId
                ? `Topology · ${shortHash(readPinLayerId)} (read-pin)`
                : `Topology · ${rootBranch}`}
            />
          </div>
        )}
      </div>
    </div>
  );
}
