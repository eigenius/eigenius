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
 * Placeholder for rail destinations whose real component lands in a
 * later phase. Renders the destination's name plus the phase it
 * ships in, so the user understands the IA without us hiding the
 * affordance (D34 §3.1: the rail is the workspace).
 *
 * The rail registers all destinations from day one so the navigation
 * shape doesn't shift as later phases land — each Phase N PR just
 * swaps one of these placeholders for the real component.
 */

import {
  Body1,
  Caption1,
  makeStyles,
  Subtitle1,
  tokens,
} from "@fluentui/react-components";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    justifyContent: "center",
    height: "100%",
    padding: tokens.spacingVerticalXXXL,
    gap: tokens.spacingVerticalS,
    color: tokens.colorNeutralForeground3,
    textAlign: "center",
  },
  title: {
    color: tokens.colorNeutralForeground2,
  },
});

export interface ComingSoonPlaceholderProps {
  destination: string;
  phase: number;
  /** Short single-sentence preview of what this destination will do. */
  description?: string;
}

export function ComingSoonPlaceholder({
  destination,
  phase,
  description,
}: ComingSoonPlaceholderProps) {
  const styles = useStyles();
  return (
    <div className={styles.root}>
      <Subtitle1 as="h2" className={styles.title}>{destination}</Subtitle1>
      <Body1>Coming in Phase {phase}.</Body1>
      {description && <Caption1>{description}</Caption1>}
    </div>
  );
}
