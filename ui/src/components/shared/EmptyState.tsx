import { ReactNode } from "react";
import { AlertCircle } from "lucide-react";

import { cn } from "@/lib/utils";

export function EmptyState({
  title,
  description,
  icon,
  className,
}: {
  title: string;
  description: string;
  icon?: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "rounded-md border border-dashed border-border bg-surface-subtle p-5 text-center",
        className,
      )}
    >
      <div className="mb-2 flex justify-center text-muted">{icon ?? <AlertCircle className="h-5 w-5" />}</div>
      <p className="text-sm font-medium text-foreground">{title}</p>
      <p className="mt-1 text-sm text-muted">{description}</p>
    </div>
  );
}
