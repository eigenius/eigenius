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

import { useMemo } from "react";
import {
  Body1Strong,
  Caption1,
  makeStyles,
  Tag,
  tokens,
} from "@fluentui/react-components";
import { decode as cborDecode } from "cbor-x";

const ID = "@id";
const IS_A = "urn:eigenius:core:is_a";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXS,
    fontFamily: tokens.fontFamilyBase,
  },
  idRow: {
    display: "flex",
    flexWrap: "wrap",
    gap: tokens.spacingHorizontalXS,
    alignItems: "center",
  },
  iri: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground2,
    wordBreak: "break-all",
  },
  classes: {
    display: "flex",
    flexWrap: "wrap",
    gap: tokens.spacingHorizontalXS,
  },
  table: {
    borderCollapse: "collapse",
    marginTop: tokens.spacingVerticalS,
    width: "100%",
  },
  th: {
    textAlign: "left",
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground2,
    padding: `${tokens.spacingVerticalXXS} ${tokens.spacingHorizontalS}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    width: "40%",
    verticalAlign: "top",
  },
  td: {
    padding: `${tokens.spacingVerticalXXS} ${tokens.spacingHorizontalS}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke3}`,
    verticalAlign: "top",
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
  },
});

export interface ResourceInspectorProps {
  /** CBOR-encoded Eigon resource bytes. */
  resource: Uint8Array;
  /** Optional trace IRI shown alongside (program-output flow). */
  traceIri?: string;
}

interface CborResource {
  [key: string]: unknown;
}

function isResource(value: unknown): value is CborResource {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function ResourceInspector(
  { resource, traceIri }: ResourceInspectorProps,
) {
  const styles = useStyles();
  const decoded = useMemo(() => decodeResource(resource), [resource]);

  if (!decoded) {
    return <Caption1>resource bytes did not decode to a resource</Caption1>;
  }

  const id = typeof decoded[ID] === "string" ? (decoded[ID] as string) : null;
  const classes = Array.isArray(decoded[IS_A])
    ? (decoded[IS_A] as unknown[]).filter((c): c is string =>
      typeof c === "string"
    )
    : [];

  const properties = Object.entries(decoded)
    .filter(([k]) => k !== ID && k !== IS_A)
    .sort(([a], [b]) => a.localeCompare(b));

  return (
    <div className={styles.root}>
      <div className={styles.idRow}>
        <Body1Strong>@id</Body1Strong>
        <span className={styles.iri}>{id ?? "(embedded resource)"}</span>
      </div>
      {classes.length > 0 && (
        <div className={styles.classes}>
          {classes.map((c) => (
            <Tag key={c} appearance="brand" size="small">
              {shortenIri(c)}
            </Tag>
          ))}
        </div>
      )}
      {traceIri && (
        <Caption1>
          trace: <span className={styles.iri}>{traceIri}</span>
        </Caption1>
      )}
      {properties.length === 0
        ? <Caption1>no properties beyond @id / is_a</Caption1>
        : (
          <table className={styles.table}>
            <tbody>
              {properties.map(([iri, value]) => (
                <tr key={iri}>
                  <th className={styles.th}>{shortenIri(iri)}</th>
                  <td className={styles.td}>{formatValue(value)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
    </div>
  );
}

function decodeResource(bytes: Uint8Array): CborResource | null {
  try {
    const decoded = cborDecode(bytes) as unknown;
    // The kernel's run-program output and Inspect both wrap a single
    // resource (not a document). But be lenient: if we get an array,
    // take the first resource entry.
    if (Array.isArray(decoded)) {
      const first = decoded.find(isResource);
      return first ?? null;
    }
    return isResource(decoded) ? decoded : null;
  } catch {
    return null;
  }
}

function shortenIri(iri: string): string {
  // Display the local-name + last namespace segment for readability.
  const lastColon = iri.lastIndexOf(":");
  if (lastColon < 0) return iri;
  const local = iri.slice(lastColon + 1);
  const before = iri.slice(0, lastColon);
  const prevColon = before.lastIndexOf(":");
  const ns = prevColon >= 0 ? before.slice(prevColon + 1) : before;
  return `${ns}:${local}`;
}

function formatValue(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  if (typeof value === "bigint") return value.toString();
  if (Array.isArray(value)) {
    return `[ ${value.map(formatValue).join(", ")} ]`;
  }
  if (isResource(value)) {
    const id = typeof value[ID] === "string" ? value[ID] : "<embedded>";
    return `→ ${shortenIri(id as string)}`;
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}
