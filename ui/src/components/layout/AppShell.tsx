import { ReactNode } from "react";
import { NavLink } from "react-router-dom";
import {
  Activity,
  BookOpen,
  ChevronLeft,
  ChevronRight,
  Compass,
  Database,
  Radar,
  ShieldAlert,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { useSidebar } from "@/lib/use-sidebar";

type ShellNavItem = {
  to: string;
  label: string;
  icon: ReactNode;
  shortcut?: string;
};

const navItems: ShellNavItem[] = [
  { to: "/explorer", label: "Explorer", icon: <Compass className="h-4 w-4" />, shortcut: "1" },
  { to: "/query", label: "Query", icon: <Database className="h-4 w-4" />, shortcut: "2" },
  { to: "/ops", label: "Ops", icon: <Activity className="h-4 w-4" />, shortcut: "3" },
  { to: "/admin", label: "Admin", icon: <ShieldAlert className="h-4 w-4" />, shortcut: "4" },
  { to: "/watch", label: "Watch", icon: <Radar className="h-4 w-4" />, shortcut: "5" },
  { to: "/schema", label: "Schema", icon: <BookOpen className="h-4 w-4" />, shortcut: "6" },
];

export { navItems };

const SIDEBAR_EXPANDED = 220;
const SIDEBAR_COLLAPSED = 52;

export function AppShell({
  routeTitle,
  authStatus,
  headerCenter,
  headerRight,
  scopeControls,
  authControls,
  schemaMeta,
  children,
}: {
  routeTitle: string;
  authStatus: string;
  headerCenter?: ReactNode;
  headerRight?: ReactNode;
  scopeControls: ReactNode;
  authControls: ReactNode;
  schemaMeta: ReactNode;
  children: ReactNode;
}) {
  const { collapsed, toggle } = useSidebar();
  const sidebarWidth = collapsed ? SIDEBAR_COLLAPSED : SIDEBAR_EXPANDED;

  return (
    <div className="min-h-screen">
      {/* Skip to main content — accessibility */}
      <a
        href="#main-content"
        className="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-50 focus:rounded-md focus:bg-accent focus:px-4 focus:py-2 focus:text-accent-foreground"
      >
        Skip to main content
      </a>

      {/* ── Sidebar ────────────────────────────────────── */}
      <aside
        aria-label="Main navigation"
        className="fixed inset-y-0 left-0 z-30 flex flex-col border-r border-border bg-panel transition-[width] duration-200 max-lg:!w-[52px]"
        style={{ width: sidebarWidth }}
      >
        {/* Logo */}
        <div className="flex h-12 shrink-0 items-center gap-2 border-b border-border px-3">
          <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-accent text-accent-foreground">
            <Database className="h-3.5 w-3.5" />
          </div>
          {!collapsed && (
            <span className="truncate text-sm font-semibold tracking-tight">PelagoDB</span>
          )}
        </div>

        {/* Nav */}
        <TooltipProvider>
          <nav className="flex-1 space-y-1 overflow-y-auto p-2">
            {navItems.map((item) => (
              <Tooltip key={item.to}>
                <TooltipTrigger asChild>
                  <NavLink
                    to={item.to}
                    className={({ isActive }) =>
                      cn(
                        "flex items-center gap-2 rounded-md px-2.5 py-1.5 text-sm font-medium text-muted transition-colors hover:bg-foreground/5 hover:text-foreground",
                        isActive && "bg-accent/10 text-accent",
                        collapsed && "justify-center px-0",
                      )
                    }
                  >
                    {item.icon}
                    {!collapsed && <span className="truncate">{item.label}</span>}
                    {!collapsed && item.shortcut && (
                      <kbd className="ml-auto text-[10px] text-muted/60">{item.shortcut}</kbd>
                    )}
                  </NavLink>
                </TooltipTrigger>
                {collapsed && (
                  <TooltipContent side="right">
                    {item.label}
                  </TooltipContent>
                )}
              </Tooltip>
            ))}
          </nav>
        </TooltipProvider>

        {/* Scope Switcher (expanded) / hidden (collapsed) */}
        {!collapsed && (
          <div className="border-t border-border p-2">
            {scopeControls}
          </div>
        )}

        {/* Toggle */}
        <button
          type="button"
          onClick={toggle}
          className="flex h-9 items-center justify-center border-t border-border text-muted transition-colors hover:text-foreground"
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          {collapsed ? <ChevronRight className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}
        </button>
      </aside>

      {/* ── Main Area ─────────────────────────────────── */}
      <div
        className="transition-[margin-left] duration-200 max-lg:!ml-[52px]"
        style={{ marginLeft: sidebarWidth }}
      >
        {/* Header */}
        <header className="sticky top-0 z-20 flex h-12 items-center gap-3 border-b border-border bg-panel/95 px-4 backdrop-blur-sm">
          {/* Left: breadcrumbs */}
          <div className="flex items-center gap-2 text-sm">
            <span className="text-muted">Console</span>
            <span className="text-muted/50">/</span>
            <span className="font-medium">{routeTitle}</span>
          </div>

          {/* Center: command palette trigger */}
          <div className="ml-auto flex items-center gap-3">
            {headerCenter}
          </div>

          {/* Right */}
          <div className="flex items-center gap-2">
            <Badge variant={authStatus === "No Auth" ? "warning" : "accent"}>{authStatus}</Badge>
            {schemaMeta}
            {headerRight}
            {authControls}
          </div>
        </header>

        {/* Content */}
        <main id="main-content" className="space-y-4 p-4">{children}</main>
      </div>
    </div>
  );
}
