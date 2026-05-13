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
 * Self-fetching layer-stack panel — wraps `LayerStackView` with the
 * `eigen.layerTopology()` call so the load-output accordion can show
 * the stack right there without the caller plumbing data through.
 *
 * Refetches whenever the active branch's head moves (compaction,
 * commit, branch switch) so the displayed chain matches what the
 * branch actually points at. Without that, a panel mounted before a
 * commit/compaction keeps showing its pre-event snapshot — which was
 * the source of "layer stack still shows the consolidated layers"
 * confusion the user hit during D34 Phase 6.
 */

import { useEffect, useMemo, useState } from "react";
import {
  Caption1,
  makeStyles,
  Spinner,
  tokens,
} from "@fluentui/react-components";
import type { LayerTopologyResponse } from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";
import { LayerStackView } from "./LayerStackView";

const useStyles = makeStyles({
  loading: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    padding: tokens.spacingVerticalS,
    color: tokens.colorNeutralForeground3,
  },
  error: {
    color: tokens.colorPaletteRedForeground1,
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
  },
});

export function LayerStackPanel() {
  const styles = useStyles();
  const eigen = useEigen();
  const activeBranch = useNotebookStore((s) => s.activeBranch);
  const branches = useNotebookStore((s) => s.branches);
  // Track the active branch's head so the fetch re-runs whenever the
  // tip moves. The kernel still resolves "current head" itself when
  // we pass no `rootLayer`; we only read this value as a dependency
  // signal that something has changed since the last fetch.
  const activeHead = useMemo(
    () => branches?.find((b) => b.name === activeBranch)?.headLayer ?? null,
    [branches, activeBranch],
  );
  const [topology, setTopology] = useState<LayerTopologyResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setTopology(null);
    setError(null);
    eigen.layerTopology({ includeResources: false })
      .then((t) => {
        if (!cancelled) setTopology(t);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [eigen, activeBranch, activeHead]);

  if (error) {
    return <Caption1 className={styles.error}>{error}</Caption1>;
  }
  if (!topology) {
    return (
      <div className={styles.loading}>
        <Spinner size="tiny" />
        <Caption1>fetching layer chain…</Caption1>
      </div>
    );
  }
  return <LayerStackView topology={topology} />;
}
