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
  Caption1,
  Card,
  CardHeader,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import type { CellJson } from "../persistence/notebook-format";
import { MarkdownCell } from "./cells/MarkdownCell";
import { ESLCell } from "./cells/ESLCell";
import { EigenQLCell } from "./cells/EigenQLCell";
import { TypeScriptCell } from "./cells/TypeScriptCell";

const useStyles = makeStyles({
  card: {
    marginBottom: tokens.spacingVerticalM,
  },
  body: {
    padding: tokens.spacingVerticalS,
  },
  typeBadge: {
    color: tokens.colorNeutralForeground3,
    textTransform: "uppercase",
    letterSpacing: "0.04em",
  },
});

export interface CellProps {
  cell: CellJson;
}

const TYPE_LABEL: Record<CellJson["type"], string> = {
  markdown: "Markdown",
  esl: "ESL",
  eigenql: "EigenQL",
  typescript: "TypeScript",
};

/**
 * Generic cell wrapper — Fluent `Card` shell with a type-label header
 * and body that delegates to one of the four per-type renderers.
 *
 * Phase 2 is read-only; Phase 4 adds toolbar (Run / delete / move /
 * change-type) and editable content.
 */
export function Cell({ cell }: CellProps) {
  const styles = useStyles();
  return (
    <Card className={styles.card} appearance="filled-alternative">
      <CardHeader
        header={<Caption1 className={styles.typeBadge}>{TYPE_LABEL[cell.type]}</Caption1>}
      />
      <div className={styles.body}>
        {cell.type === "markdown" && <MarkdownCell source={cell.source} />}
        {cell.type === "esl" && <ESLCell source={cell.source} />}
        {cell.type === "eigenql" && <EigenQLCell source={cell.source} />}
        {cell.type === "typescript" && <TypeScriptCell source={cell.source} />}
      </div>
    </Card>
  );
}
