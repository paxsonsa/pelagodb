import { useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { JsonInspector } from "@/components/shared/JsonInspector";
import type { SchemaDefinition } from "@/lib/types";

export function SchemaDetail({ schema }: { schema: SchemaDefinition | null }) {
  const [showRaw, setShowRaw] = useState(false);

  if (!schema) {
    return (
      <div className="flex h-64 items-center justify-center text-sm text-muted">
        Select a schema from the list to inspect.
      </div>
    );
  }

  const properties = Object.entries(schema.properties ?? {});
  const edges = Object.entries(schema.edges ?? {});

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center gap-3">
        <h3 className="text-lg font-semibold">{schema.name}</h3>
        <Badge variant="default">v{schema.version}</Badge>
      </div>

      {/* Metadata */}
      <div className="text-xs text-muted">
        <span>Created by: {schema.created_by}</span>
        <span className="ml-4">
          Created: {new Date(schema.created_at).toLocaleString()}
        </span>
      </div>

      {/* Properties table */}
      <div>
        <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">
          Properties ({properties.length})
        </h4>
        {properties.length > 0 ? (
          <div className="rounded-md border border-border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead>Required</TableHead>
                  <TableHead>Index</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {properties.map(([name, prop]) => (
                  <TableRow key={name}>
                    <TableCell className="font-mono text-xs font-medium">{name}</TableCell>
                    <TableCell className="text-xs">{prop.type}</TableCell>
                    <TableCell className="text-xs">{prop.required ? "yes" : "no"}</TableCell>
                    <TableCell className="text-xs text-muted">{prop.index || "none"}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        ) : (
          <p className="text-xs text-muted">No properties defined</p>
        )}
      </div>

      {/* Edges table */}
      <div>
        <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">
          Edges ({edges.length})
        </h4>
        {edges.length > 0 ? (
          <div className="rounded-md border border-border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Label</TableHead>
                  <TableHead>Direction</TableHead>
                  <TableHead>Sort Key</TableHead>
                  <TableHead>Ownership</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {edges.map(([label, edge]) => (
                  <TableRow key={label}>
                    <TableCell className="font-mono text-xs font-medium">{label}</TableCell>
                    <TableCell className="text-xs">{edge.direction}</TableCell>
                    <TableCell className="text-xs text-muted">{edge.sort_key}</TableCell>
                    <TableCell className="text-xs text-muted">{edge.ownership}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        ) : (
          <p className="text-xs text-muted">No edges defined</p>
        )}
      </div>

      {/* Raw JSON */}
      <div>
        <Button type="button" variant="ghost" size="sm" onClick={() => setShowRaw(!showRaw)}>
          {showRaw ? "Hide Raw JSON" : "Show Raw JSON"}
        </Button>
        {showRaw && <JsonInspector value={schema} maxHeight={400} className="mt-2" />}
      </div>
    </div>
  );
}
