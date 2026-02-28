import { AlertCircle, CheckCircle2, Info } from "lucide-react";

import { cn } from "@/lib/utils";

export function InlineNotice({
  kind,
  title,
  detail,
  className,
}: {
  kind: "info" | "ok" | "error";
  title: string;
  detail?: string;
  className?: string;
}) {
  const palette =
    kind === "error"
      ? "border-danger/30 bg-danger/10 text-danger"
      : kind === "ok"
        ? "border-ok/30 bg-ok/10 text-ok"
        : "border-accent/30 bg-accent/10 text-accent";

  const Icon = kind === "error" ? AlertCircle : kind === "ok" ? CheckCircle2 : Info;

  return (
    <div className={cn("rounded-md border px-3 py-2", palette, className)} role="status" aria-live="polite">
      <div className="flex items-start gap-2">
        <Icon className="mt-0.5 h-4 w-4 shrink-0" />
        <div>
          <p className="text-sm font-medium">{title}</p>
          {detail ? <p className="mt-0.5 text-xs opacity-90">{detail}</p> : null}
        </div>
      </div>
    </div>
  );
}
