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
 * Format a timestamp (millis since Unix epoch) as a short relative
 * string: "just now" / "12 min ago" / "3 hr ago" / "yesterday" /
 * "5 days ago" / "2026-04-23". Falls back to the absolute ISO date
 * past a week, so values don't drift into "47 days ago" territory
 * which is uselessly imprecise.
 *
 * Used by the BranchBar's tip hover-card, the Branches panel's "Last
 * commit" column, and the History panel rows.
 *
 * Returns the empty string when `ms === 0` — that's the "missing
 * timestamp" sentinel the kernel returns when a layer's handle has
 * been reclaimed.
 */
export function formatRelative(ms: number, now: number = Date.now()): string {
  if (ms <= 0) return "";
  const delta = Math.max(0, now - ms);
  const SEC = 1000;
  const MIN = 60 * SEC;
  const HR = 60 * MIN;
  const DAY = 24 * HR;
  const WEEK = 7 * DAY;

  if (delta < 45 * SEC) return "just now";
  if (delta < 90 * SEC) return "1 min ago";
  if (delta < 45 * MIN) return `${Math.round(delta / MIN)} min ago`;
  if (delta < 90 * MIN) return "1 hr ago";
  if (delta < 22 * HR) return `${Math.round(delta / HR)} hr ago`;
  if (delta < 36 * HR) return "yesterday";
  if (delta < WEEK) return `${Math.round(delta / DAY)} days ago`;
  // Past a week, show the date so the user gets a non-fuzzy anchor.
  return new Date(ms).toISOString().slice(0, 10);
}

/** Absolute ISO timestamp (UTC). For tooltips that complement the
 *  relative form — relative for at-a-glance, absolute for precise. */
export function formatAbsoluteIso(ms: number): string {
  if (ms <= 0) return "";
  return new Date(ms).toISOString();
}
