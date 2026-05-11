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
  Accordion,
  AccordionHeader,
  AccordionItem,
  AccordionPanel,
  Body1,
  Caption1,
  makeStyles,
  MessageBar,
  MessageBarBody,
  MessageBarTitle,
  tokens,
} from "@fluentui/react-components";
import type { CellOutput } from "../../runtime/notebookStore";
import { CommitStatusBadge } from "./CommitStatusBadge";
import { LayerStackPanel } from "./LayerStackPanel";
import { ProgramRunOutputView } from "./ProgramRunOutputView";
import { ResourceInspector } from "./ResourceInspector";
import { ResultTable } from "./ResultTable";
import { TypeScriptValueView } from "./TypeScriptValueView";

const useStyles = makeStyles({
  root: {
    marginTop: tokens.spacingVerticalS,
    paddingTop: tokens.spacingVerticalS,
    borderTop: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  loadStatus: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXXS,
    color: tokens.colorPaletteGreenForeground2,
  },
  meta: {
    color: tokens.colorNeutralForeground3,
    fontFamily: tokens.fontFamilyMonospace,
  },
  // The accordion header bundles its own padding; trim it so the
  // expandable "View layer stack" affordance sits flush with the load
  // summary above it.
  stackAccordion: {
    marginTop: tokens.spacingVerticalXS,
  },
  errorPre: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    margin: 0,
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
  },
});

export interface CellOutputViewProps {
  output: CellOutput;
}

export function CellOutputView({ output }: CellOutputViewProps) {
  const styles = useStyles();
  return (
    <div className={styles.root}>
      {renderBody(output, styles)}
    </div>
  );
}

function renderBody(
  output: CellOutput,
  styles: ReturnType<typeof useStyles>,
) {
  switch (output.kind) {
    case "load":
      return (
        <>
          <div className={styles.loadStatus}>
            <Body1>
              Loaded {output.resourceCount} resource
              {output.resourceCount === 1 ? "" : "s"}
            </Body1>
            <Caption1 className={styles.meta}>
              layer = {output.layerId}
            </Caption1>
            {output.warnings.length > 0 && (
              <Caption1>{output.warnings.length} warning(s)</Caption1>
            )}
            {output.commit && <CommitStatusBadge commit={output.commit} />}
          </div>
          <Accordion collapsible className={styles.stackAccordion}>
            <AccordionItem value="stack">
              <AccordionHeader size="small">View layer stack</AccordionHeader>
              <AccordionPanel>
                <LayerStackPanel />
              </AccordionPanel>
            </AccordionItem>
          </Accordion>
        </>
      );

    case "validate":
      return (
        <div className={styles.loadStatus}>
          <Body1>
            {output.valid ? "Program valid" : "Program invalid"}
          </Body1>
          {output.programType && (
            <Caption1 className={styles.meta}>
              type: {output.programType}
            </Caption1>
          )}
          {output.errors.length > 0 && (
            <pre className={styles.errorPre}>{output.errors.join("\n")}</pre>
          )}
        </div>
      );

    case "resultset":
      return (
        <>
          <ResultTable document={output.document} />
          {output.commit && <CommitStatusBadge commit={output.commit} />}
        </>
      );

    case "resource":
      return (
        <>
          <ResourceInspector
            resource={output.resource}
            traceIri={output.traceIri}
          />
          {output.commit && <CommitStatusBadge commit={output.commit} />}
        </>
      );

    case "value":
      return <TypeScriptValueView value={output.value} log={output.log} />;

    case "program-run":
      return (
        <ProgramRunOutputView
          programIri={output.programIri}
          results={output.results}
        />
      );

    case "error":
      return (
        <MessageBar intent="error">
          <MessageBarBody>
            <MessageBarTitle>Cell failed</MessageBarTitle>
            <pre className={styles.errorPre}>{output.message}</pre>
          </MessageBarBody>
        </MessageBar>
      );
  }
}
