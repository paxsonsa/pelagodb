import { useState } from "react";
import { Copy, Database } from "lucide-react";
import { useNavigate } from "react-router-dom";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableRow } from "@/components/ui/table";
import { JsonInspector } from "@/components/shared/JsonInspector";
import type { NodeModel } from "@/lib/types";

function typeLabel(value: unknown): string {
  if (value === null || value === undefined) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value;
}

function formatValue(value: unknown): string {
  if (value === null || value === undefined) return "null";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

export function NodeDetailPanel({ node }: { node: NodeModel | null }) {
  const navigate = useNavigate();
  const [showRaw, setShowRaw] = useState(false);

  if (!node) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted">
        Select a node in the graph to inspect.
      </div>
    );
  }

  const properties = Object.entries(node.properties ?? {});

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center gap-2">
        <Badge variant="accent">{node.entity_type}</Badge>
        <span className="font-mono text-sm text-foreground">{node.id}</span>
      </div>

      {/* Actions */}
      <div className="flex gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => void navigator.clipboard.writeText(node.id)}
        >
          <Copy className="mr-1.5 h-3.5 w-3.5" /> Copy ID
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => navigate("/query")}
        >
          <Database className="mr-1.5 h-3.5 w-3.5" /> Query this type
        </Button>
      </div>

      {/* Properties */}
      <div>
        <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">
          Properties ({properties.length})
        </h4>
        {properties.length > 0 ? (
          <div className="rounded-md border border-border">
            <Table>
              <TableBody>
                {properties.map(([key, value]) => (
                  <TableRow key={key}>
                    <TableCell className="w-[120px] font-mono text-xs font-medium">{key}</TableCell>
                    <TableCell className="text-xs text-foreground/80">
                      <span className="mr-2 text-[10px] text-muted">{typeLabel(value)}</span>
                      {formatValue(value)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        ) : (
          <p className="text-xs text-muted">No properties</p>
        )}
      </div>

      {/* Metadata */}
      <div>
        <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">Metadata</h4>
        <div className="space-y-1 text-xs">
          <p><span className="text-muted">Locality:</span> {node.locality}</p>
          {node.created_at != null && (
            <p><span className="text-muted">Created:</span> {new Date(node.created_at).toLocaleString()}</p>
          )}
          {node.updated_at != null && (
            <p><span className="text-muted">Updated:</span> {new Date(node.updated_at).toLocaleString()}</p>
          )}
        </div>
      </div>

      {/* Raw JSON */}
      <Button type="button" variant="ghost" size="sm" onClick={() => setShowRaw(!showRaw)}>
        {showRaw ? "Hide Raw JSON" : "Show Raw JSON"}
      </Button>
      {showRaw && <JsonInspector value={node} maxHeight={280} />}
    </div>
  );
}
