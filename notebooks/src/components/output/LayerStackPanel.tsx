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
 * The fetch happens once on mount; the panel remounts each time its
 * containing accordion expands, which means we re-fetch (cheap) and
 * always show the current chain rather than a stale snapshot.
 */

import { useEffect, useState } from "react";
import { Caption1, makeStyles, Spinner, tokens } from "@fluentui/react-components";
import type { LayerTopologyResponse } from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
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
  const [topology, setTopology] = useState<LayerTopologyResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
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
  }, [eigen]);

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
