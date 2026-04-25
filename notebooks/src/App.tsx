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

import {
  FluentProvider,
  webLightTheme,
} from "@fluentui/react-components";
import { Notebook } from "./components/Notebook";
import { parseNotebook } from "./persistence/notebook-format";
import patentDemo from "../examples/patent-analysis.json";

/**
 * Phase 2 — static viewer.
 *
 * Hardcodes the patent-analysis demo at the React root so `vite dev`
 * renders something on first load. Phase 4 replaces this with a file
 * picker (open from disk) + new-notebook flows.
 */
export function App() {
  // parseNotebook validates the shape; throws on malformed data.
  const notebook = parseNotebook(patentDemo);

  return (
    <FluentProvider theme={webLightTheme}>
      <Notebook notebook={notebook} />
    </FluentProvider>
  );
}
