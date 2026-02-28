import { useMemo, useState } from "react";
import { BracesIcon, Rows3 } from "lucide-react";

import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";

function stringify(value: unknown, spacing = 2): string {
  try {
    return JSON.stringify(value, null, spacing);
  } catch {
    return "[unserializable value]";
  }
}

export function JsonInspector({
  value,
  className,
  maxHeight = 280,
  fontSizeClass = "text-sm",
}: {
  value: unknown;
  className?: string;
  maxHeight?: number;
  fontSizeClass?: string;
}) {
  const [tab, setTab] = useState<"pretty" | "compact">("pretty");

  const pretty = useMemo(() => stringify(value, 2), [value]);
  const compact = useMemo(() => stringify(value, 0), [value]);

  return (
    <div className={cn("rounded-md border border-border bg-panel", className)}>
      <Tabs value={tab} onValueChange={(next) => setTab(next as "pretty" | "compact")}>
        <div className="flex items-center justify-between border-b border-border/80 px-3 py-2">
          <p className="text-xs font-semibold uppercase tracking-wide text-muted">Inspector</p>
          <TabsList>
            <TabsTrigger value="pretty" className="text-xs">
              <BracesIcon className="mr-1 h-3.5 w-3.5" /> Pretty
            </TabsTrigger>
            <TabsTrigger value="compact" className="text-xs">
              <Rows3 className="mr-1 h-3.5 w-3.5" /> Compact
            </TabsTrigger>
          </TabsList>
        </div>
        <TabsContent value="pretty">
          <div className="w-full overflow-auto scrollbar-thin" style={{ maxHeight }}>
            <pre className={cn("overflow-x-auto p-3 font-mono leading-5 text-foreground/80", fontSizeClass)}>
              {pretty}
            </pre>
          </div>
        </TabsContent>
        <TabsContent value="compact">
          <div className="w-full overflow-auto scrollbar-thin" style={{ maxHeight }}>
            <pre className={cn("overflow-x-auto p-3 font-mono leading-5 text-foreground/80", fontSizeClass)}>
              {compact}
            </pre>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  );
}
