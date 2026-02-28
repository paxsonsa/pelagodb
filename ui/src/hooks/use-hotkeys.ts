import { useEffect } from "react";

type HotkeyHandler = (event: KeyboardEvent) => void;

function isInputFocused(): boolean {
  const el = document.activeElement;
  if (!el) return false;
  const tag = el.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    (el as HTMLElement).isContentEditable ||
    el.closest(".cm-editor") !== null
  );
}

/**
 * Register a global keyboard shortcut.
 *
 * Key format: "mod+k", "1", "[", "mod+enter"
 * "mod" maps to Meta on Mac and Ctrl elsewhere.
 */
export function useHotkey(
  key: string,
  handler: HotkeyHandler,
  opts?: { allowInInput?: boolean },
) {
  useEffect(() => {
    const parts = key.toLowerCase().split("+");
    const mainKey = parts[parts.length - 1];
    const needsMod = parts.includes("mod");
    const needsShift = parts.includes("shift");

    function listener(event: KeyboardEvent) {
      if (!opts?.allowInInput && isInputFocused()) return;

      const isMod = event.metaKey || event.ctrlKey;
      if (needsMod && !isMod) return;
      if (needsShift && !event.shiftKey) return;
      if (!needsMod && (event.metaKey || event.ctrlKey)) return;

      if (event.key.toLowerCase() === mainKey || event.code.toLowerCase() === `key${mainKey}`) {
        event.preventDefault();
        handler(event);
      }
    }

    window.addEventListener("keydown", listener);
    return () => window.removeEventListener("keydown", listener);
  }, [key, handler, opts?.allowInInput]);
}
