import { useLocation } from "react-router-dom";
import type { ScopeContext } from "@/lib/types";

const routeLabels: Record<string, string> = {
  "/explorer": "Explorer",
  "/query": "Query Studio",
  "/ops": "Operations",
  "/admin": "Admin",
  "/watch": "Watch",
  "/schema": "Schema",
};

export function Breadcrumbs({ scope }: { scope: ScopeContext }) {
  const location = useLocation();
  const pageLabel = routeLabels[location.pathname] ?? "Explorer";

  return (
    <div className="flex items-center gap-1.5 text-sm">
      <span className="text-muted">{scope.database}</span>
      <span className="text-muted/40">/</span>
      <span className="text-muted">{scope.namespace}</span>
      <span className="text-muted/40">/</span>
      <span className="font-medium text-foreground">{pageLabel}</span>
    </div>
  );
}
