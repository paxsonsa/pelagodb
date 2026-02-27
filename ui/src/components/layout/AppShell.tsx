import { ReactNode } from "react";
import { NavLink } from "react-router-dom";
import {
  Activity,
  Compass,
  Database,
  Radar,
  Settings,
  ShieldAlert,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { cn } from "@/lib/utils";

type ShellNavItem = {
  to: string;
  label: string;
  icon: ReactNode;
};

const navItems: ShellNavItem[] = [
  { to: "/explorer", label: "Explorer", icon: <Compass className="h-4 w-4" /> },
  { to: "/query", label: "Query", icon: <Database className="h-4 w-4" /> },
  { to: "/ops", label: "Ops", icon: <Activity className="h-4 w-4" /> },
  { to: "/admin", label: "Admin", icon: <ShieldAlert className="h-4 w-4" /> },
  { to: "/watch", label: "Watch", icon: <Radar className="h-4 w-4" /> },
];

export function AppShell({
  routeTitle,
  authStatus,
  scopeControls,
  authControls,
  schemaMeta,
  children,
}: {
  routeTitle: string;
  authStatus: string;
  scopeControls: ReactNode;
  authControls: ReactNode;
  schemaMeta: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="mx-auto min-h-screen w-full max-w-[1700px] p-3 lg:p-4">
      <header className="glass-panel rounded-lg border px-3 py-2">
        <div className="flex flex-wrap items-center gap-2">
          <div className="flex items-center gap-2">
            <div className="rounded-md border border-border bg-slate-100 p-1.5">
              <Settings className="h-4 w-4 text-accent" />
            </div>
            <div>
              <h1 className="text-sm font-semibold">PelagoDB Console</h1>
              <p className="text-xs text-muted">{routeTitle}</p>
            </div>
          </div>

          <div className="ml-auto flex flex-wrap items-center gap-2">
            <div className="min-w-[280px] flex-1">{scopeControls}</div>
            <div className="flex items-center gap-2">
              <Badge variant={authStatus === "No Auth" ? "warning" : "accent"}>{authStatus}</Badge>
              <div>{schemaMeta}</div>
              {authControls}
            </div>
          </div>
        </div>
      </header>

      <div className="mt-3 grid gap-3 lg:grid-cols-[205px_minmax(0,1fr)]">
        <aside className="glass-panel rounded-lg border p-2 lg:sticky lg:top-3 lg:h-fit">
          <nav className="space-y-1">
            {navItems.map((item) => (
              <NavLink
                key={item.to}
                to={item.to}
                className={({ isActive }) =>
                  cn(
                    "flex items-center gap-2 rounded-md px-2.5 py-1.5 text-sm font-medium text-muted transition-colors hover:bg-slate-100 hover:text-foreground",
                    isActive && "bg-blue-50 text-accent",
                  )
                }
              >
                {item.icon}
                {item.label}
              </NavLink>
            ))}
          </nav>
          <Separator className="my-2" />
          <p className="text-xs leading-relaxed text-muted">Graph workflows and operations tooling.</p>
        </aside>

        <main className="space-y-4">{children}</main>
      </div>
    </div>
  );
}
