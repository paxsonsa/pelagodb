import { useCallback, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  Activity,
  BookOpen,
  Compass,
  Database,
  Moon,
  Radar,
  RefreshCw,
  Search,
  ShieldAlert,
  Sun,
} from "lucide-react";

import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { useTheme } from "@/lib/use-theme";
import { useHotkey } from "@/hooks/use-hotkeys";

export function CommandPalette({
  onRefreshSchema,
}: {
  onRefreshSchema?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const navigate = useNavigate();
  const { effective, toggleTheme } = useTheme();

  const toggle = useCallback(() => setOpen((prev) => !prev), []);

  useHotkey("mod+k", toggle, { allowInInput: true });

  function go(path: string) {
    navigate(path);
    setOpen(false);
  }

  function action(fn: () => void) {
    fn();
    setOpen(false);
  }

  return (
    <>
      {/* Trigger button for header */}
      <button
        type="button"
        onClick={toggle}
        className="flex h-8 items-center gap-2 rounded-md border border-border bg-surface-subtle px-3 text-sm text-muted transition-colors hover:text-foreground"
      >
        <Search className="h-3.5 w-3.5" />
        <span className="hidden sm:inline">Search or jump to...</span>
        <kbd className="ml-2 hidden rounded border border-border bg-panel px-1.5 py-0.5 text-[10px] font-medium sm:inline">
          ⌘K
        </kbd>
      </button>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="overflow-hidden p-0 sm:max-w-[520px] [&>button]:hidden">
          <Command>
            <CommandInput placeholder="Type a command or search..." />
            <CommandList className="max-h-80">
              <CommandEmpty>No results found.</CommandEmpty>

              <CommandGroup heading="Navigation">
                <CommandItem onSelect={() => go("/explorer")}>
                  <Compass className="mr-2 h-4 w-4" />
                  Explorer
                </CommandItem>
                <CommandItem onSelect={() => go("/query")}>
                  <Database className="mr-2 h-4 w-4" />
                  Query Studio
                </CommandItem>
                <CommandItem onSelect={() => go("/ops")}>
                  <Activity className="mr-2 h-4 w-4" />
                  Operations
                </CommandItem>
                <CommandItem onSelect={() => go("/admin")}>
                  <ShieldAlert className="mr-2 h-4 w-4" />
                  Admin
                </CommandItem>
                <CommandItem onSelect={() => go("/watch")}>
                  <Radar className="mr-2 h-4 w-4" />
                  Watch Streaming
                </CommandItem>
                <CommandItem onSelect={() => go("/schema")}>
                  <BookOpen className="mr-2 h-4 w-4" />
                  Schema Browser
                </CommandItem>
              </CommandGroup>

              <CommandSeparator />

              <CommandGroup heading="Actions">
                <CommandItem onSelect={() => action(toggleTheme)}>
                  {effective === "dark" ? (
                    <Sun className="mr-2 h-4 w-4" />
                  ) : (
                    <Moon className="mr-2 h-4 w-4" />
                  )}
                  Toggle Theme
                </CommandItem>
                {onRefreshSchema && (
                  <CommandItem onSelect={() => action(onRefreshSchema)}>
                    <RefreshCw className="mr-2 h-4 w-4" />
                    Refresh Schema Catalog
                  </CommandItem>
                )}
              </CommandGroup>
            </CommandList>
          </Command>
        </DialogContent>
      </Dialog>
    </>
  );
}
