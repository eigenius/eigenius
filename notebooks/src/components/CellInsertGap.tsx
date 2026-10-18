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
 * Inter-cell insertion affordance — option 3 from the design pass:
 * a persistent thin divider that teaches the user *where* cells can
 * be inserted, plus a hover-revealed `[+]` button that opens a type
 * menu when clicked.
 *
 * Used between every pair of cells and at the very top/bottom of the
 * notebook (N+1 gaps for N cells). The per-cell toolbar's `+` menu
 * remains as a redundant always-visible path for touch / no-hover.
 */

import {
  Button,
  makeStyles,
  Menu,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
  tokens,
} from "@fluentui/react-components";
import { Add16Regular } from "@fluentui/react-icons";
import type { CellType } from "../persistence/notebook-format";
import { useNotebookStore } from "../runtime/notebookStore";

const useStyles = makeStyles({
  // Hit-target wrapper. Fixed height so layout doesn't jump on hover.
  gap: {
    position: "relative",
    height: "20px",
    margin: `${tokens.spacingVerticalXS} 0`,
    // Reveal the button when this gap (or anything inside it,
    // including the open Menu trigger) is hovered.
    ":hover .insert-button": {
      opacity: 1,
      pointerEvents: "auto",
    },
  },
  // Persistent divider line, centered vertically.
  divider: {
    position: "absolute",
    top: "50%",
    left: 0,
    right: 0,
    height: "1px",
    background: tokens.colorNeutralStroke3,
    transform: "translateY(-0.5px)",
  },
  // The button sits centered over the divider, hidden until hover.
  buttonWrap: {
    position: "absolute",
    top: "50%",
    left: "50%",
    transform: "translate(-50%, -50%)",
    background: tokens.colorNeutralBackground1,
    padding: `0 ${tokens.spacingHorizontalXS}`,
    borderRadius: tokens.borderRadiusCircular,
    opacity: 0,
    transition: "opacity 120ms ease",
    pointerEvents: "none",
  },
});

export interface CellInsertGapProps {
  /** IRI to insert after; `null` means insert at the very top. */
  afterCellId: string | null;
}

export function CellInsertGap({ afterCellId }: CellInsertGapProps) {
  const styles = useStyles();
  const insertCell = useNotebookStore((s) => s.insertCell);

  const insert = (type: CellType) => insertCell(afterCellId, type);

  return (
    <div className={styles.gap}>
      <div className={styles.divider} />
      <div className={`${styles.buttonWrap} insert-button`}>
        <Menu>
          <MenuTrigger disableButtonEnhancement>
            <Button
              size="small"
              shape="circular"
              appearance="subtle"
              icon={<Add16Regular />}
              aria-label="Insert cell here"
              title="Insert cell"
            />
          </MenuTrigger>
          <MenuPopover>
            <MenuList>
              <MenuItem onClick={() => insert("markdown")}>Markdown</MenuItem>
              <MenuItem onClick={() => insert("esl")}>ESL</MenuItem>
              <MenuItem onClick={() => insert("eigenql")}>EigenQL</MenuItem>
              <MenuItem onClick={() => insert("typescript")}>
                TypeScript
              </MenuItem>
              <MenuItem onClick={() => insert("program-run")}>
                Program run
              </MenuItem>
              <MenuItem onClick={() => insert("chart")}>Chart</MenuItem>
            </MenuList>
          </MenuPopover>
        </Menu>
      </div>
    </div>
  );
}
