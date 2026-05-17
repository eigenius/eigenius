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
  tokens,
  Tooltip,
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
import { formatAbsoluteIso, formatRelative } from "../runtime/relativeTime";
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
    // Room for `branch-name` + `tip aabb…ccdd · NN units ago` without
    // truncation. Previously 320px clipped the relative-time suffix
    // once the menu's content density grew past the active branch.
    minWidth: "440px",
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
    // Keep `tip <hash> · <relative-time>` on one line so the time
    // suffix never wraps off the right edge of the menu.
    whiteSpace: "nowrap",
  },
  menuTime: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
    fontFamily: tokens.fontFamilyBase,
    whiteSpace: "nowrap",
  },
  // Read-pin indicator: `· reading at <hash>` next to the tip. Drawn
  // in a warning-tint so the user notices reads aren't at the tip.
  readPin: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalXS,
    color: tokens.colorPaletteYellowForeground1,
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    padding: `0 ${tokens.spacingHorizontalXS}`,
    borderLeft: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  returnToTipBtn: {
    minHeight: "auto",
    paddingTop: tokens.spacingVerticalXXS,
    paddingBottom: tokens.spacingVerticalXXS,
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
  const readPinLayerId = useNotebookStore((s) => s.readPinLayerId);
  const refreshBranches = useNotebookStore((s) => s.refreshBranches);
  const switchBranch = useNotebookStore((s) => s.switchBranch);
  const setReadPin = useNotebookStore((s) => s.setReadPin);

  const [createOpen, setCreateOpen] = useState(false);

  // Best-effort refresh on mount so the picker has a populated menu
  // the first time the user opens it. Failures (in-memory kernel)
  // leave the cache `null`; the picker degrades to a single static
  // row for the active branch.
  useEffect(() => {
    void refreshBranches(eigen);
  }, [eigen, refreshBranches]);

  const activeInfo = useMemo(() => {
    if (!branches) return null;
    return branches.find((b) => b.name === activeBranch) ?? null;
  }, [branches, activeBranch]);
  const activeHead = activeInfo?.headLayer ?? null;
  // BigInt because the proto codec maps `int64` → bigint. Convert
  // once so callers can pass it to plain `number`-shaped helpers.
  const activeHeadCommittedAtMs = activeInfo
    ? Number(activeInfo.headCommittedAtMs)
    : 0;

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
        headCommittedAtMs={activeHeadCommittedAtMs}
        knownBranches={branches}
        styles={styles}
      />

      {readPinLayerId && (
        <ReadPinIndicator
          layerId={readPinLayerId}
          onReturn={() => setReadPin(null)}
          styles={styles}
        />
      )}

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
                {b.headCommittedAtMs > 0n && (
                  <span className={styles.menuTime}>
                    {" · "}
                    {formatRelative(Number(b.headCommittedAtMs))}
                  </span>
                )}
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
  /** `0` when the kernel didn't report a timestamp (no backend, or
   *  the head's handle was reclaimed). */
  headCommittedAtMs: number;
  knownBranches: readonly BranchInfo[] | null;
  styles: ReturnType<typeof useStyles>;
}

function TipIndicator({
  active,
  head,
  headCommittedAtMs,
  knownBranches,
  styles,
}: TipIndicatorProps) {
  // Three states:
  // - `knownBranches === null`: in-memory mode. No tip to show.
  // - `head === null`: branches list loaded but doesn't contain
  //   `active` (shouldn't happen in practice, but the SDK doesn't
  //   guarantee it — show a placeholder rather than crash).
  // - `head` present: render short-hash + tooltip with full id +
  //   commit timestamp (relative on the hover, absolute on the
  //   second line).
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
  const relative = formatRelative(headCommittedAtMs);
  const absolute = formatAbsoluteIso(headCommittedAtMs);
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
          {absolute && (
            <div style={{ fontSize: 12, marginTop: 4 }}>
              committed: {absolute}
            </div>
          )}
        </div>
      }
      withArrow
    >
      <span className={styles.tipBlock}>
        <Body1 as="span" className={styles.tipLabel}>
          tip:
        </Body1>
        <span>{shortHash(head)}</span>
        {relative && <span className={styles.tipLabel}>· {relative}</span>}
      </span>
    </Tooltip>
  );
}

interface ReadPinIndicatorProps {
  layerId: string;
  onReturn: () => void;
  styles: ReturnType<typeof useStyles>;
}

/** Renders the `· reading at <hash>` strip with a "Return to tip"
 *  button when the user has time-travelled via the History panel
 *  (D34 §5.2). The pin is per-session, not a kernel concept; "Return
 *  to tip" just clears the local pin and re-routes reads to the
 *  branch's current head. */
function ReadPinIndicator(
  { layerId, onReturn, styles }: ReadPinIndicatorProps,
) {
  return (
    <Tooltip
      relationship="description"
      withArrow
      content={
        <div>
          <div>
            Reads are pinned to this layer ("Time-travel here"). Writes still go
            to the branch tip.
          </div>
          <div style={{ fontFamily: "monospace", fontSize: 12, marginTop: 4 }}>
            {layerId}
          </div>
        </div>
      }
    >
      <span className={styles.readPin}>
        <span>· reading at {shortHash(layerId)}</span>
        <Button
          size="small"
          appearance="subtle"
          className={styles.returnToTipBtn}
          onClick={onReturn}
        >
          Return to tip
        </Button>
      </span>
    </Tooltip>
  );
}

/** Render a `LayerId` hex string as `aaaa…bbbb` (4 + 4). */
function shortHash(hex: string): string {
  if (hex.length <= 10) return hex;
  return `${hex.slice(0, 4)}…${hex.slice(-4)}`;
}
