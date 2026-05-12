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
 * D34 §3.2 header bar — branch picker, tip indicator, unsaved-dot.
 *
 * Lives between the notebook title row and the toolbar. Always visible
 * so the user can see "where am I writing?" at any point. The three
 * pieces of state on this row are conceptually independent and rendered
 * by sibling components below; this file wires them together.
 *
 * Phase 2 scope (per the D34 §16 rollout):
 *
 * - Branch picker `Menu` with the list from `Eigen.listBranches`. Footer
 *   action opens the Create Branch dialog. Switching reloads the
 *   workspace through `notebookStore.switchBranch` (clears the
 *   session-local cell-output cache; cells stay).
 * - Tip indicator showing the active branch's head as a short hash.
 *   Hover surfaces the full id. Layer name / resource count /
 *   `created_at` belong here too but await a kernel-side
 *   `head_committed_at` on `GetBranch` (queued — see §16).
 * - `●` unsaved-changes dot driven by `notebookStore.dirty`.
 */

import { useEffect, useMemo, useState } from "react";
import {
  Body1,
  Button,
  Caption1,
  Divider,
  makeStyles,
  Menu,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
  Spinner,
  Toast,
  ToastBody,
  Toaster,
  ToastTitle,
  Tooltip,
  tokens,
  useId,
  useToastController,
} from "@fluentui/react-components";
import {
  Add16Regular,
  ChevronDown16Regular,
  Circle12Filled,
} from "@fluentui/react-icons";
import type { BranchInfo } from "@eigenius/client";
import { useEigen } from "../runtime/EigenProvider";
import { useNotebookStore } from "../runtime/notebookStore";
import { CreateBranchDialog } from "./dialogs/CreateBranchDialog";

const BRANCH_TOASTER_ID = "branch-bar-toaster";

/**
 * Matches the D23 §5.4.4 sibling-branch naming convention
 * (`auto-YYYY-MM-DD`). Auto-branches are typically "the chain we
 * couldn't merge back" artefacts, not active development branches —
 * the picker de-emphasises them so the eye lands on the actual
 * working set first.
 */
const AUTO_BRANCH_RE = /^auto-\d{4}-\d{2}-\d{2}/;

const useStyles = makeStyles({
  row: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    flexWrap: "wrap",
  },
  pickerButton: {
    minWidth: "fit-content",
  },
  tipBlock: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalXS,
    color: tokens.colorNeutralForeground3,
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
  },
  tipLabel: {
    color: tokens.colorNeutralForeground3,
    fontFamily: tokens.fontFamilyBase,
  },
  unsavedDot: {
    color: tokens.colorPaletteYellowForeground1,
    display: "flex",
    alignItems: "center",
  },
  menuRow: {
    display: "grid",
    gridTemplateColumns: "1fr auto",
    columnGap: tokens.spacingHorizontalM,
    alignItems: "baseline",
    minWidth: "320px",
  },
  menuName: {
    fontWeight: tokens.fontWeightSemibold,
  },
  menuNameAuto: {
    fontWeight: tokens.fontWeightRegular,
    color: tokens.colorNeutralForeground3,
  },
  menuTip: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
  },
  emptyState: {
    padding: `${tokens.spacingVerticalS} ${tokens.spacingHorizontalM}`,
    color: tokens.colorNeutralForeground3,
  },
});

export function BranchBar() {
  const styles = useStyles();
  const eigen = useEigen();
  const toasterId = useId("toaster", BRANCH_TOASTER_ID);
  const { dispatchToast } = useToastController(toasterId);

  const activeBranch = useNotebookStore((s) => s.activeBranch);
  const branches = useNotebookStore((s) => s.branches);
  const dirty = useNotebookStore((s) => s.dirty);
  const refreshBranches = useNotebookStore((s) => s.refreshBranches);
  const switchBranch = useNotebookStore((s) => s.switchBranch);

  const [createOpen, setCreateOpen] = useState(false);

  // Best-effort refresh on mount so the picker has a populated menu
  // the first time the user opens it. Failures (in-memory kernel)
  // leave the cache `null`; the picker degrades to a single static
  // row for the active branch.
  useEffect(() => {
    void refreshBranches(eigen);
  }, [eigen, refreshBranches]);

  const activeHead = useMemo(() => {
    if (!branches) return null;
    return branches.find((b) => b.name === activeBranch)?.headLayer ?? null;
  }, [branches, activeBranch]);

  const onSwitch = (target: BranchInfo) => {
    if (target.name === activeBranch) return;
    switchBranch(eigen, target.name);
    dispatchToast(
      <Toast>
        <ToastTitle>Switched to {target.name}</ToastTitle>
        <ToastBody>
          Cell outputs cleared. Click Run All to populate against this branch.
        </ToastBody>
      </Toast>,
      { intent: "info", timeout: 6000 },
    );
  };

  return (
    <div className={styles.row}>
      <Menu
        positioning="below-start"
        onOpenChange={(_e, data) => {
          if (data.open) {
            // Refresh on every open so newly-created branches and
            // freshly-advanced tips show up immediately. Cheap on
            // a persistent backend; a no-op on in-memory.
            void refreshBranches(eigen);
          }
        }}
      >
        <MenuTrigger disableButtonEnhancement>
          <Button
            size="small"
            appearance="subtle"
            className={styles.pickerButton}
            iconPosition="after"
            icon={<ChevronDown16Regular />}
          >
            branch: <strong>{activeBranch}</strong>
          </Button>
        </MenuTrigger>
        <MenuPopover>
          <BranchMenu
            branches={branches}
            activeBranch={activeBranch}
            onSwitch={onSwitch}
            onCreate={() => setCreateOpen(true)}
            styles={styles}
          />
        </MenuPopover>
      </Menu>

      <TipIndicator
        active={activeBranch}
        head={activeHead}
        knownBranches={branches}
        styles={styles}
      />

      {dirty && (
        <Tooltip
          content="Notebook has unsaved cell or metadata edits."
          relationship="description"
        >
          <span className={styles.unsavedDot} aria-label="Unsaved changes">
            <Circle12Filled />
          </span>
        </Tooltip>
      )}

      <CreateBranchDialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        onCreated={(name) => {
          dispatchToast(
            <Toast>
              <ToastTitle>Created branch {name}</ToastTitle>
            </Toast>,
            { intent: "success", timeout: 4000 },
          );
        }}
      />

      <Toaster toasterId={toasterId} position="top-end" />
    </div>
  );
}

interface BranchMenuProps {
  branches: readonly BranchInfo[] | null;
  activeBranch: string;
  onSwitch: (b: BranchInfo) => void;
  onCreate: () => void;
  styles: ReturnType<typeof useStyles>;
}

function BranchMenu({
  branches,
  activeBranch,
  onSwitch,
  onCreate,
  styles,
}: BranchMenuProps) {
  if (branches === null) {
    // The kernel rejected `listBranches` — in-memory mode only serves
    // `main`. Surface a static single-row menu instead of a confusing
    // "loading…" state that would never resolve.
    return (
      <MenuList>
        <div className={styles.emptyState}>
          <Caption1>
            In-memory kernel — only <code>main</code> is available.
          </Caption1>
        </div>
      </MenuList>
    );
  }
  if (branches.length === 0) {
    return (
      <MenuList>
        <div className={styles.emptyState}>
          <Spinner size="tiny" /> <Caption1>No branches found.</Caption1>
        </div>
        <Divider />
        <MenuItem icon={<Add16Regular />} onClick={onCreate}>
          Create branch…
        </MenuItem>
      </MenuList>
    );
  }
  return (
    <MenuList>
      {branches.map((b) => {
        const isActive = b.name === activeBranch;
        const isAuto = AUTO_BRANCH_RE.test(b.name);
        return (
          <MenuItem
            key={b.name}
            // Disable the no-op "switch to current branch" item visually
            // so the menu doesn't suggest there's an action where there
            // isn't one.
            disabled={isActive}
            onClick={() => onSwitch(b)}
          >
            <div className={styles.menuRow}>
              <span
                className={isAuto ? styles.menuNameAuto : styles.menuName}
              >
                {isActive ? "● " : "○ "}
                {b.name}
              </span>
              <span className={styles.menuTip}>
                tip {shortHash(b.headLayer)}
              </span>
            </div>
          </MenuItem>
        );
      })}
      <Divider />
      <MenuItem icon={<Add16Regular />} onClick={onCreate}>
        Create branch…
      </MenuItem>
    </MenuList>
  );
}

interface TipIndicatorProps {
  active: string;
  head: string | null;
  knownBranches: readonly BranchInfo[] | null;
  styles: ReturnType<typeof useStyles>;
}

function TipIndicator({ active, head, knownBranches, styles }: TipIndicatorProps) {
  // Three states:
  // - `knownBranches === null`: in-memory mode. No tip to show.
  // - `head === null`: branches list loaded but doesn't contain
  //   `active` (shouldn't happen in practice, but the SDK doesn't
  //   guarantee it — show a placeholder rather than crash).
  // - `head` present: render short-hash + tooltip with full id.
  if (knownBranches === null) {
    return null;
  }
  if (head === null) {
    return (
      <span className={styles.tipBlock}>
        <Body1 as="span" className={styles.tipLabel}>tip:</Body1>
        <Caption1>(unknown — branch not yet listed)</Caption1>
      </span>
    );
  }
  return (
    <Tooltip
      relationship="description"
      content={
        <div>
          <div>
            <strong>{active}</strong>
          </div>
          <div style={{ fontFamily: "monospace", fontSize: 12 }}>
            head: {head}
          </div>
        </div>
      }
      withArrow
    >
      <span className={styles.tipBlock}>
        <Body1 as="span" className={styles.tipLabel}>
          tip:
        </Body1>
        <span>{shortHash(head)}</span>
      </span>
    </Tooltip>
  );
}

/** Render a `LayerId` hex string as `aaaa…bbbb` (4 + 4). */
function shortHash(hex: string): string {
  if (hex.length <= 10) return hex;
  return `${hex.slice(0, 4)}…${hex.slice(-4)}`;
}
