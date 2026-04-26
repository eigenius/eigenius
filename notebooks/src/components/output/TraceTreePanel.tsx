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
 * Self-fetching trace-tree panel — wraps `TraceTree` with the
 * `eigen.inspect(traceIri)` call so output panels for `runProgram`
 * results can show the trace right there without the caller plumbing
 * data through. Fetches once per traceIri; if the IRI changes (re-run)
 * the fetch re-runs.
 */

import { useEffect, useState } from "react";
import {
  Caption1,
  makeStyles,
  Spinner,
  tokens,
} from "@fluentui/react-components";
import { useEigen } from "../../runtime/EigenProvider";
import { TraceTree } from "./TraceTree";

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

export interface TraceTreePanelProps {
  traceIri: string;
}

export function TraceTreePanel({ traceIri }: TraceTreePanelProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const [traceBytes, setTraceBytes] = useState<Uint8Array | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setTraceBytes(null);
    setError(null);
    eigen.inspect(traceIri)
      .then((resp) => {
        if (cancelled) return;
        if (!resp.found) {
          setError(`trace not found: ${traceIri}`);
          return;
        }
        setTraceBytes(resp.resource);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [eigen, traceIri]);

  if (error) {
    return <Caption1 className={styles.error}>{error}</Caption1>;
  }
  if (!traceBytes) {
    return (
      <div className={styles.loading}>
        <Spinner size="tiny" />
        <Caption1>fetching trace…</Caption1>
      </div>
    );
  }
  return <TraceTree trace={traceBytes} />;
}
