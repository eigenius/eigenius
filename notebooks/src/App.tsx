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

import { useEffect } from "react";
import {
  FluentProvider,
  Toaster,
  webLightTheme,
} from "@fluentui/react-components";
import { MergeEventToaster, TOASTER_ID } from "./components/MergeEventToaster";
import { WorkspaceShell } from "./components/workspace/WorkspaceShell";
import { parseNotebook } from "./persistence/notebook-format";
import { EigenProvider } from "./runtime/EigenProvider";
import { useNotebookStore } from "./runtime/notebookStore";
import patentDemo from "../examples/patent-analysis.json";

/**
 * Phase 4a — authoring.
 *
 * On first mount the patent-analysis demo is loaded into the store so
 * `vite dev` renders something on first open. Subsequent loads come from
 * the Open… file picker in the Notebook toolbar.
 */
export function App() {
  const loadNotebook = useNotebookStore((s) => s.loadNotebook);
  const cellCount = useNotebookStore((s) => s.cells.length);

  useEffect(() => {
    if (cellCount === 0) {
      loadNotebook(parseNotebook(patentDemo));
    }
    // Empty deps — only the very first mount seeds the demo. After that
    // the user is in control via Open… / Save.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <FluentProvider theme={webLightTheme}>
      <EigenProvider>
        <WorkspaceShell />
        {
          /*
           * D34 §6.1 trivial-merge toast surface. `Toaster` mounts the
           * portal at the root; `MergeEventToaster` watches cell outputs
           * and dispatches into it. Both live inside FluentProvider so
           * they pick up the active theme.
           */
        }
        <Toaster toasterId={TOASTER_ID} position="top-end" />
        <MergeEventToaster />
      </EigenProvider>
    </FluentProvider>
  );
}
