import { Badge } from "@/components/ui/badge";
import type { ParsedMetric } from "./MetricsParser";

export function MetricCard({ metric }: { metric: ParsedMetric }) {
  const primaryValue = metric.samples[0]?.value;
  const displayValue = primaryValue !== undefined
    ? Number.isInteger(primaryValue) ? primaryValue.toString() : primaryValue.toFixed(3)
    : "-";

  return (
    <div className="metric-card space-y-2">
      <div className="flex items-center justify-between">
        <span className="truncate text-xs font-semibold">{metric.name}</span>
        <Badge variant="default" className="text-[9px]">{metric.type}</Badge>
      </div>
      {metric.help && (
        <p className="truncate text-[11px] text-muted" title={metric.help}>{metric.help}</p>
      )}
      <p className="text-lg font-semibold tabular-nums">{displayValue}</p>
      {metric.samples.length > 1 && (
        <p className="text-[10px] text-muted">{metric.samples.length} sample(s)</p>
      )}
    </div>
  );
}
