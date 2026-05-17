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
 * Workspace shell — D34 §3.1.
 *
 * Three regions:
 *
 * 1. **Workspace header** (always visible): the `<BranchBar />` carrying
 *    branch picker + tip indicator + unsaved-dot. The notebook's own
 *    title/toolbar stays inside `<Notebook />` and is only visible
 *    when the Notebook destination is active.
 * 2. **Rail** (left): Fluent UI `Nav` with the D34 §3.1 hierarchy —
 *    Notebook at the top, then Chain / Workspace / Admin categories,
 *    then Health at the bottom. Each rail entry routes to a
 *    destination component.
 * 3. **Main pane** (right): the active destination's content.
 *
 * Destinations land in phases:
 *
 * - **Notebook**: Phase 1/2 (the existing `<Notebook />`).
 * - **Branches**: Phase 3 (`<BranchesPanel />`).
 * - **History / Tags / Merge / Topology / Institutions / Tasks /
 *   Compaction / GC / Health**: placeholder until their phase lands;
 *   the rail surface for each is registered from day one so the IA
 *   doesn't shift later (D34 §3.1).
 */

import { useState } from "react";
import { useNotebookStore } from "../../runtime/notebookStore";
import type { WorkspaceDestination } from "../../runtime/notebookStore";
import {
  NavCategory,
  NavCategoryItem,
  NavDivider,
  NavDrawer,
  NavDrawerBody,
  NavItem,
  NavSectionHeader,
  NavSubItem,
  NavSubItemGroup,
} from "@fluentui/react-nav";
import {
  Body1,
  Button,
  makeStyles,
  mergeClasses,
  tokens,
  Tooltip,
} from "@fluentui/react-components";
import {
  Apps20Filled,
  Apps20Regular,
  Archive20Filled,
  Archive20Regular,
  ArrowLeft20Regular,
  ArrowRight20Regular,
  BranchFork20Filled,
  BranchFork20Regular,
  bundleIcon,
  CheckmarkCircle20Filled,
  CheckmarkCircle20Regular,
  ClipboardTaskListLtr20Filled,
  ClipboardTaskListLtr20Regular,
  DocumentBulletList20Filled,
  DocumentBulletList20Regular,
  History20Filled,
  History20Regular,
  Merge20Filled,
  Merge20Regular,
  Notebook20Filled,
  Notebook20Regular,
  PanelLeftContract20Regular,
  PanelLeftExpand20Regular,
  Stack20Filled,
  Stack20Regular,
  Tag20Filled,
  Tag20Regular,
} from "@fluentui/react-icons";
import eigeniusLogoUrl from "../../assets/eigenius_logo_bw_24px.png";
import { BranchBar } from "../BranchBar";
import { Notebook } from "../Notebook";
import { BranchesPanel } from "./BranchesPanel";
import { CompactionPanel } from "./CompactionPanel";
import { GcPanel } from "./GcPanel";
import { HealthPanel } from "./HealthPanel";
import { HistoryPanel } from "./HistoryPanel";
import { InstitutionsPanel } from "./InstitutionsPanel";
import { LayerPanel } from "./LayerPanel";
import { MergePanel } from "./MergePanel";
import { TagsPanel } from "./TagsPanel";
import { TasksPanel } from "./TasksPanel";
import { TopologyPanel } from "./TopologyPanel";

/**
 * Destination keys driving the active main-pane content. Strings
 * (rather than numbers) so debugger output / route-state are
 * self-describing if we later add URL routing.
 */
// `Destination` is exported from the store as `WorkspaceDestination`
// so non-shell components (panels, dialogs) can navigate by key
// without importing this file's internals. The local alias keeps
// this file's call sites short.
type Destination = WorkspaceDestination;

// Icon bundles for the rail. Fluent v9's `bundleIcon` pairs a filled
// variant (selected state) with a regular variant (idle).
const NotebookIcon = bundleIcon(Notebook20Filled, Notebook20Regular);
const BranchesIcon = bundleIcon(BranchFork20Filled, BranchFork20Regular);
const HistoryIcon = bundleIcon(History20Filled, History20Regular);
const TagsIcon = bundleIcon(Tag20Filled, Tag20Regular);
const MergeIcon = bundleIcon(Merge20Filled, Merge20Regular);
const TopologyIcon = bundleIcon(Apps20Filled, Apps20Regular);
const InstitutionsIcon = bundleIcon(
  DocumentBulletList20Filled,
  DocumentBulletList20Regular,
);
const TasksIcon = bundleIcon(
  ClipboardTaskListLtr20Filled,
  ClipboardTaskListLtr20Regular,
);
const CompactionIcon = bundleIcon(Stack20Filled, Stack20Regular);
const GcIcon = bundleIcon(Archive20Filled, Archive20Regular);
const HealthIcon = bundleIcon(
  CheckmarkCircle20Filled,
  CheckmarkCircle20Regular,
);

/** Eigenius brand mark in the rail's `AppItem` slot. Pre-sized to 24px
 *  to match Fluent's nav-header iconography; rendered as a plain
 *  `<img>` so we don't pull in an extra SVG-icon dependency. */
function EigeniusLogo() {
  return (
    <img
      src={eigeniusLogoUrl}
      alt="Eigenius"
      width={24}
      height={24}
      style={{ display: "block" }}
    />
  );
}

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    height: "100vh",
    overflow: "hidden",
    background: tokens.colorNeutralBackground2,
  },
  header: {
    flexShrink: 0,
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalM,
    padding: `${tokens.spacingVerticalS} ${tokens.spacingHorizontalM}`,
    background: tokens.colorNeutralBackground1,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  // Brand block at top-left: logo + "Eigenius" wordmark. Anchors
  // the app's identity. The Hamburger sits right after it (the
  // canonical "rail toggle adjacent to brand" pattern).
  headerBrand: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    flexShrink: 0,
  },
  headerBrandText: {
    fontWeight: tokens.fontWeightSemibold,
  },
  headerBranch: {
    flex: 1,
    minWidth: 0,
  },
  body: {
    display: "flex",
    flex: 1,
    minHeight: 0,
  },
  drawer: {
    flexShrink: 0,
    borderRight: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  // Collapsed-rail surface. We don't try to narrow Fluent's NavDrawer
  // here (its internal Drawer surface owns its own width via the
  // `size` prop and resists CSS overrides — Phase 3 spent time on
  // that and bounced) — instead we render a plain icon-button column
  // when collapsed and the full NavDrawer only when expanded. Same
  // destinations, same click handler, much less Fluent CSS to fight.
  railCollapsed: {
    flexShrink: 0,
    width: "48px",
    background: tokens.colorNeutralBackground1,
    borderRight: `1px solid ${tokens.colorNeutralStroke2}`,
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    paddingTop: tokens.spacingVerticalS,
    gap: tokens.spacingVerticalXXS,
    overflowY: "auto",
  },
  railIconBtn: {
    // Square icon button, sized to match the surrounding rail width
    // so the visual rhythm doesn't break.
    minWidth: "40px",
    width: "40px",
    height: "40px",
    padding: 0,
  },
  railIconBtnActive: {
    // Active destination highlight — Fluent's `appearance="subtle"`
    // doesn't carry a selected state, so we tint the bg ourselves.
    background: tokens.colorNeutralBackground2Selected,
  },
  // Tight 1px separator for the collapsed rail. Fluent's `Divider`
  // adds ~8px of vertical padding on each side plus a min-height,
  // which inflates the spacing to a full icon-row between groups.
  // A hand-styled hr (or just a top-border div) keeps the gap to a
  // few pixels.
  railDivider: {
    width: "60%",
    height: "1px",
    background: tokens.colorNeutralStroke2,
    border: 0,
    margin: `${tokens.spacingVerticalXXS} 0`,
    flexShrink: 0,
  },
  hamburger: {
    // Lives in the workspace top bar (left of BranchBar), not in the
    // rail itself. Standard app-shell location — every modern Fluent
    // app puts the rail-toggle here, not nested inside the rail.
    flexShrink: 0,
  },
  navBtn: {
    // Match the hamburger's footprint so the row of icon-buttons in
    // the top bar lines up cleanly.
    flexShrink: 0,
  },
  main: {
    flex: 1,
    minWidth: 0,
    minHeight: 0,
    overflow: "hidden",
    background: tokens.colorNeutralBackground1,
    // Each destination decides its own scroll behaviour. The shell
    // contains them to the bounds set here.
    display: "flex",
    flexDirection: "column",
  },
});

/** Single source of truth for the rail's destinations. Expanded
 *  renders the label and section headers; collapsed renders icon-only
 *  buttons wrapped in Tooltips. Adding a destination is a one-row edit.
 */
const RAIL_ITEMS: Array<
  | { kind: "item"; value: Destination; label: string; icon: JSX.Element }
  | { kind: "section"; label: string }
  | { kind: "divider" }
> = [
  {
    kind: "item",
    value: "notebook",
    label: "Notebook",
    icon: <NotebookIcon />,
  },
  // Divider here too so collapsed mode (which drops the "Chain"
  // section header) still visually separates Notebook from the chain
  // group below it.
  { kind: "divider" },
  { kind: "section", label: "Chain" },
  {
    kind: "item",
    value: "branches",
    label: "Branches",
    icon: <BranchesIcon />,
  },
  { kind: "item", value: "history", label: "History", icon: <HistoryIcon /> },
  { kind: "item", value: "tags", label: "Tags", icon: <TagsIcon /> },
  { kind: "item", value: "merge", label: "Merge", icon: <MergeIcon /> },
  { kind: "divider" },
  { kind: "section", label: "Workspace" },
  {
    kind: "item",
    value: "topology",
    label: "Topology",
    icon: <TopologyIcon />,
  },
  {
    kind: "item",
    value: "institutions",
    label: "Institutions",
    icon: <InstitutionsIcon />,
  },
  { kind: "item", value: "tasks", label: "Tasks", icon: <TasksIcon /> },
  { kind: "divider" },
  { kind: "section", label: "Admin" },
  {
    kind: "item",
    value: "compaction",
    label: "Compaction",
    icon: <CompactionIcon />,
  },
  { kind: "item", value: "gc", label: "GC", icon: <GcIcon /> },
  { kind: "divider" },
  { kind: "item", value: "health", label: "Health", icon: <HealthIcon /> },
];

export function WorkspaceShell() {
  const styles = useStyles();
  // Destination state lives in the store (D34 §3.1 — any rail
  // destination can navigate to any other, e.g. BranchesPanel's
  // "View history" action jumps to History without prop-drilling).
  const destination = useNotebookStore((s) => s.destination);
  const setDestination = useNotebookStore((s) => s.setDestination);
  // Browser-style back/forward over destination history. The store
  // owns the stack so navigations triggered by any panel (not just
  // the rail) participate.
  const goBackDestination = useNotebookStore((s) => s.goBackDestination);
  const goForwardDestination = useNotebookStore(
    (s) => s.goForwardDestination,
  );
  const canGoBack = useNotebookStore((s) => s.destinationCursor > 0);
  const canGoForward = useNotebookStore(
    (s) => s.destinationCursor < s.destinationHistory.length - 1,
  );
  // `railCollapsed`: icon-only mode. Section headers, dividers, and
  // item labels are hidden; each icon gets a hover tooltip. Default
  // expanded so a first-time user sees the IA; subsequent toggles
  // remembered for the session only (no persistence yet).
  const [railCollapsed, setRailCollapsed] = useState(false);

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        {
          /* App identity anchored at the top-left — the canonical
            position. The Hamburger immediately follows it as the
            rail-toggle control. */
        }
        <div className={styles.headerBrand}>
          <EigeniusLogo />
          <Body1 className={styles.headerBrandText}>Eigenius</Body1>
        </div>
        <Tooltip
          relationship="label"
          content={railCollapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          {
            /* Stateful PanelLeft icon — Contract while expanded
              (clicking will close), Expand while collapsed (clicking
              will open). Conveys both current state and the click's
              effect, which a static Hamburger doesn't — Hamburger
              traditionally implies a menu, not a sidebar toggle. */
          }
          <Button
            appearance="subtle"
            className={styles.hamburger}
            icon={railCollapsed
              ? <PanelLeftExpand20Regular />
              : <PanelLeftContract20Regular />}
            onClick={() => setRailCollapsed((v) => !v)}
            aria-label={railCollapsed ? "Expand sidebar" : "Collapse sidebar"}
          />
        </Tooltip>
        <Tooltip relationship="label" content="Back">
          <Button
            appearance="subtle"
            className={styles.navBtn}
            icon={<ArrowLeft20Regular />}
            disabled={!canGoBack}
            onClick={goBackDestination}
            aria-label="Back"
          />
        </Tooltip>
        <Tooltip relationship="label" content="Forward">
          <Button
            appearance="subtle"
            className={styles.navBtn}
            icon={<ArrowRight20Regular />}
            disabled={!canGoForward}
            onClick={goForwardDestination}
            aria-label="Forward"
          />
        </Tooltip>
        <div className={styles.headerBranch}>
          <BranchBar />
        </div>
      </div>
      <div className={styles.body}>
        {railCollapsed
          ? (
            <CollapsedRail
              destination={destination}
              onSelect={setDestination}
              styles={styles}
            />
          )
          : (
            <NavDrawer
              // Inline rendering — the rail is the workspace, not a
              // transient overlay. The expanded mode owns
              // section headers, the AppItem, and dividers.
              open
              type="inline"
              selectedValue={destination}
              onNavItemSelect={(_e, data) => {
                setDestination(data.value as Destination);
              }}
              className={styles.drawer}
            >
              {
                /* No NavDrawerHeader — brand identity lives in the
                  workspace top bar (left of Hamburger). Duplicating
                  it inside the rail would just consume space and
                  doubt the user's sense of "where am I?". */
              }
              <NavDrawerBody>
                {RAIL_ITEMS.map((entry, idx) => {
                  if (entry.kind === "section") {
                    return (
                      <NavSectionHeader key={`section-${idx}`}>
                        {entry.label}
                      </NavSectionHeader>
                    );
                  }
                  if (entry.kind === "divider") {
                    return <NavDivider key={`divider-${idx}`} />;
                  }
                  return (
                    <NavItem
                      key={entry.value}
                      value={entry.value}
                      icon={entry.icon}
                    >
                      {entry.label}
                    </NavItem>
                  );
                })}
              </NavDrawerBody>
            </NavDrawer>
          )}
        <main className={styles.main}>
          <DestinationView destination={destination} />
        </main>
      </div>
    </div>
  );
}

interface CollapsedRailProps {
  destination: Destination;
  onSelect: (d: Destination) => void;
  styles: ReturnType<typeof useStyles>;
}

/**
 * Icon-only rail used when the workspace's rail is collapsed.
 *
 * Deliberately not built on Fluent's `NavDrawer` — the v9 NavDrawer's
 * Drawer surface owns its own width (`size` prop) and refuses to
 * narrow via outer CSS, so building the collapsed rail out of plain
 * `<Button>` icons sidesteps the whole problem and keeps the click
 * surface trivially functional. The destinations + dividers come
 * from the same `RAIL_ITEMS` table the expanded NavDrawer uses, so
 * the two modes stay in sync.
 */
function CollapsedRail({ destination, onSelect, styles }: CollapsedRailProps) {
  return (
    <aside className={styles.railCollapsed} aria-label="Sidebar navigation">
      {
        /* No brand block — identity lives in the workspace top bar.
          Items start directly. */
      }
      {RAIL_ITEMS.map((entry, idx) => {
        if (entry.kind === "section") {
          // Section text-labels don't show in collapsed mode — the
          // dividers between groups carry the grouping signal.
          return null;
        }
        if (entry.kind === "divider") {
          return <hr key={`divider-${idx}`} className={styles.railDivider} />;
        }
        const isActive = entry.value === destination;
        return (
          <Tooltip
            key={entry.value}
            content={entry.label}
            relationship="label"
            positioning="after"
          >
            <Button
              appearance="subtle"
              icon={entry.icon}
              aria-label={entry.label}
              aria-current={isActive ? "page" : undefined}
              className={mergeClasses(
                styles.railIconBtn,
                isActive && styles.railIconBtnActive,
              )}
              onClick={() => onSelect(entry.value)}
            />
          </Tooltip>
        );
      })}
    </aside>
  );
}

function DestinationView({ destination }: { destination: Destination }) {
  switch (destination) {
    case "notebook":
      return <Notebook />;
    case "branches":
      return <BranchesPanel />;
    case "history":
      return <HistoryPanel />;
    case "merge":
      return <MergePanel />;
    case "compaction":
      return <CompactionPanel />;
    case "tasks":
      return <TasksPanel />;
    case "tags":
      return <TagsPanel />;
    case "gc":
      return <GcPanel />;
    case "institutions":
      return <InstitutionsPanel />;
    case "topology":
      return <TopologyPanel />;
    case "layer":
      return <LayerPanel />;
    case "health":
      return <HealthPanel />;
  }
}

// Suppress unused-import warnings for the v9 Nav primitives that
// later phases will introduce as sub-items (NavCategory etc.). They
// stay imported so the file documents the full vocabulary the rail
// will need; removing them now and re-adding later just churns the
// import list.
void [NavCategory, NavCategoryItem, NavSubItem, NavSubItemGroup];
