import type { ReactNode } from "react";

import { Badge } from "@/components/ui/badge";

export function StatusBadge({
  state,
  children,
}: {
  state: "ok" | "warning" | "error" | "active" | "idle";
  children: ReactNode;
}) {
  const variant =
    state === "ok"
      ? "success"
      : state === "warning"
        ? "warning"
        : state === "error"
          ? "danger"
          : state === "active"
            ? "accent"
            : "default";

  return <Badge variant={variant}>{children}</Badge>;
}
