import { Maximize2, Minimize2, RotateCcw, ZoomIn, ZoomOut, Focus, Tag } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";

type CyInstance = {
  zoom: (level?: number) => number;
  fit: (padding?: number) => void;
  center: () => void;
  layout: (options: Record<string, unknown>) => { run: () => void };
  getElementById: (id: string) => { length: number; position: () => { x: number; y: number } };
};

export function GraphControls({
  cy,
  rootId,
  showLabels,
  onToggleLabels,
  onFullscreen,
  isFullscreen,
}: {
  cy: CyInstance | null;
  rootId: string;
  showLabels: boolean;
  onToggleLabels: () => void;
  onFullscreen: () => void;
  isFullscreen: boolean;
}) {
  if (!cy) return null;

  const actions = [
    { label: "Zoom In", icon: ZoomIn, onClick: () => cy.zoom(cy.zoom() * 1.2) },
    { label: "Zoom Out", icon: ZoomOut, onClick: () => cy.zoom(cy.zoom() / 1.2) },
    { label: "Fit to Screen", icon: Focus, onClick: () => cy.fit(30) },
    {
      label: "Center on Root",
      icon: RotateCcw,
      onClick: () => {
        const node = cy.getElementById(rootId);
        if (node.length) cy.center();
      },
    },
    {
      label: "Reset Layout",
      icon: RotateCcw,
      onClick: () => cy.layout({ name: "cose", animate: true, nodeRepulsion: 9000 }).run(),
    },
    { label: showLabels ? "Hide Labels" : "Show Labels", icon: Tag, onClick: onToggleLabels },
    { label: isFullscreen ? "Exit Fullscreen" : "Fullscreen", icon: isFullscreen ? Minimize2 : Maximize2, onClick: onFullscreen },
  ];

  return (
    <TooltipProvider>
      <div className="absolute top-3 right-3 z-10 flex flex-col gap-1 rounded-lg border border-border bg-panel/90 p-1 shadow-lg backdrop-blur-sm">
        {actions.map((action) => (
          <Tooltip key={action.label}>
            <TooltipTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-8 w-8 p-0"
                onClick={action.onClick}
                aria-label={action.label}
              >
                <action.icon className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="left">{action.label}</TooltipContent>
          </Tooltip>
        ))}
      </div>
    </TooltipProvider>
  );
}
