import { useMemo, useState } from "react";
import { BookOpen, Search } from "lucide-react";

import { useConsoleContext } from "@/App";
import { EmptyState } from "@/components/shared/EmptyState";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import type { SchemaDefinition } from "@/lib/types";
import { SchemaDetail } from "./SchemaDetail";

export default function SchemaView() {
  const { schemaCatalog, schemaLoading } = useConsoleContext();
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<SchemaDefinition | null>(null);

  const filtered = useMemo(() => {
    if (!search.trim()) return schemaCatalog;
    const q = search.toLowerCase();
    return schemaCatalog.filter((s) => s.name.toLowerCase().includes(q));
  }, [schemaCatalog, search]);

  return (
    <section className="space-y-4">
      <div className="grid gap-4 lg:grid-cols-[320px_minmax(0,1fr)]">
        {/* Schema list */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <BookOpen className="h-4 w-4" />
              Schema Catalog
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="relative mb-3">
              <Search className="pointer-events-none absolute top-2.5 left-2 h-4 w-4 text-muted" />
              <Input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="Filter schemas..."
                className="pl-8"
              />
            </div>

            {schemaLoading ? (
              <p className="py-4 text-center text-sm text-muted">Loading schemas...</p>
            ) : filtered.length > 0 ? (
              <ScrollArea className="h-[500px]">
                <div className="space-y-1">
                  {filtered.map((schema) => {
                    const propCount = Object.keys(schema.properties ?? {}).length;
                    const edgeCount = Object.keys(schema.edges ?? {}).length;
                    return (
                      <button
                        key={schema.name}
                        type="button"
                        onClick={() => setSelected(schema)}
                        className={cn(
                          "flex w-full items-center justify-between rounded-md px-3 py-2 text-left text-sm transition-colors hover:bg-foreground/5",
                          selected?.name === schema.name && "bg-accent/10 text-accent",
                        )}
                      >
                        <span className="truncate font-medium">{schema.name}</span>
                        <span className="ml-2 shrink-0 text-[10px] text-muted">
                          {propCount}p · {edgeCount}e
                        </span>
                      </button>
                    );
                  })}
                </div>
              </ScrollArea>
            ) : (
              <EmptyState
                title="No Schemas Found"
                description={search ? "No schemas match your filter." : "No schemas in current scope."}
                icon={<BookOpen className="h-5 w-5" />}
              />
            )}
          </CardContent>
        </Card>

        {/* Schema detail */}
        <Card>
          <CardContent className="pt-6">
            <SchemaDetail schema={selected} />
          </CardContent>
        </Card>
      </div>
    </section>
  );
}
