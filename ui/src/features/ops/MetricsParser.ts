export type ParsedMetric = {
  name: string;
  help: string;
  type: string;
  samples: Array<{
    labels: Record<string, string>;
    value: number;
  }>;
};

/**
 * Parse Prometheus text exposition format into structured data.
 */
export function parsePrometheusMetrics(text: string): ParsedMetric[] {
  const metrics: ParsedMetric[] = [];
  let current: ParsedMetric | null = null;

  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) continue;

    if (trimmed.startsWith("# HELP ")) {
      const rest = trimmed.slice(7);
      const spaceIdx = rest.indexOf(" ");
      const name = spaceIdx > 0 ? rest.slice(0, spaceIdx) : rest;
      const help = spaceIdx > 0 ? rest.slice(spaceIdx + 1) : "";

      current = { name, help, type: "untyped", samples: [] };
      metrics.push(current);
      continue;
    }

    if (trimmed.startsWith("# TYPE ")) {
      const rest = trimmed.slice(7);
      const spaceIdx = rest.indexOf(" ");
      const name = spaceIdx > 0 ? rest.slice(0, spaceIdx) : rest;
      const type = spaceIdx > 0 ? rest.slice(spaceIdx + 1) : "untyped";

      if (current && current.name === name) {
        current.type = type;
      } else {
        current = { name, help: "", type, samples: [] };
        metrics.push(current);
      }
      continue;
    }

    if (trimmed.startsWith("#")) continue;

    // Parse: metric_name{label="value",...} value
    const match = trimmed.match(/^([a-zA-Z_:][a-zA-Z0-9_:]*)(\{([^}]*)\})?\s+(.+)$/);
    if (!match) continue;

    const metricName = match[1];
    const labelsRaw = match[3] ?? "";
    const value = parseFloat(match[4]);

    const labels: Record<string, string> = {};
    if (labelsRaw) {
      for (const pair of labelsRaw.split(",")) {
        const eqIdx = pair.indexOf("=");
        if (eqIdx > 0) {
          const k = pair.slice(0, eqIdx).trim();
          const v = pair.slice(eqIdx + 1).trim().replace(/^"(.*)"$/, "$1");
          labels[k] = v;
        }
      }
    }

    // Find or create matching metric
    let target = current?.name === metricName ? current : metrics.find((m) => m.name === metricName);
    if (!target) {
      target = { name: metricName, help: "", type: "untyped", samples: [] };
      metrics.push(target);
    }

    target.samples.push({ labels, value });
  }

  return metrics;
}
