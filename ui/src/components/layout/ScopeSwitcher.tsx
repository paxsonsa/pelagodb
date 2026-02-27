import { Database, RefreshCcw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { ScopeContext } from "@/lib/types";

export function ScopeSwitcher({
  scope,
  setScope,
  databaseOptions,
  namespaceOptionsByDatabase,
  schemaCount,
  schemaLoading,
  onRefreshSchema,
}: {
  scope: ScopeContext;
  setScope: (next: ScopeContext) => void;
  databaseOptions: string[];
  namespaceOptionsByDatabase: Record<string, string[]>;
  schemaCount: number;
  schemaLoading: boolean;
  onRefreshSchema: () => Promise<void>;
}) {
  const selectableDatabases = Array.from(new Set([...databaseOptions, scope.database].filter(Boolean))).sort((left, right) =>
    left.localeCompare(right),
  );
  const selectableNamespaces = Array.from(
    new Set([...(namespaceOptionsByDatabase[scope.database] ?? []), scope.namespace].filter(Boolean)),
  ).sort((left, right) => left.localeCompare(right));

  const handleDatabaseChange = (database: string) => {
    const namespaces = namespaceOptionsByDatabase[database] ?? [];
    const nextNamespace =
      namespaces.find((namespace) => namespace === scope.namespace) ?? namespaces[0] ?? scope.namespace ?? "default";
    setScope({
      database,
      namespace: nextNamespace,
    });
  };

  return (
    <div className="rounded-md border border-border bg-panel px-2 py-1.5">
      <div className="mb-1.5 flex items-center justify-between">
        <div className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-muted">
          <Database className="h-4 w-4" /> Context
        </div>
        <div className="flex items-center gap-1.5">
          <Badge variant="default">{schemaCount} schema{schemaCount === 1 ? "" : "s"}</Badge>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => void onRefreshSchema()}
            disabled={schemaLoading}
            className="h-7 w-7 p-0"
          >
            <RefreshCcw className="h-4 w-4" />
          </Button>
        </div>
      </div>

      <div className="grid gap-1.5 sm:grid-cols-2">
        <label className="space-y-1">
          <Label className="text-[11px] text-muted">Database</Label>
          <Select value={scope.database} onValueChange={handleDatabaseChange}>
            <SelectTrigger className="h-8">
              <SelectValue placeholder="Select database" />
            </SelectTrigger>
            <SelectContent>
              {selectableDatabases.map((database) => (
                <SelectItem key={database} value={database}>
                  {database}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </label>
        <label className="space-y-1">
          <Label className="text-[11px] text-muted">Namespace</Label>
          <Select value={scope.namespace} onValueChange={(namespace) => setScope({ ...scope, namespace })}>
            <SelectTrigger className="h-8">
              <SelectValue placeholder="Select namespace" />
            </SelectTrigger>
            <SelectContent>
              {selectableNamespaces.map((namespace) => (
                <SelectItem key={namespace} value={namespace}>
                  {namespace}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </label>
      </div>
    </div>
  );
}
