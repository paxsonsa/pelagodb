function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

export function copyJson(data: unknown): Promise<void> {
  const text = JSON.stringify(data, null, 2);
  return navigator.clipboard.writeText(text);
}

export function downloadJson(data: unknown, filename = "export.json") {
  const text = JSON.stringify(data, null, 2);
  const blob = new Blob([text], { type: "application/json" });
  downloadBlob(blob, filename);
}

export function downloadCsv(rows: Record<string, unknown>[], filename = "export.csv") {
  if (rows.length === 0) return;

  // Flatten nested properties
  const flatRows = rows.map((row) => {
    const flat: Record<string, string> = {};
    for (const [key, value] of Object.entries(row)) {
      if (value !== null && typeof value === "object") {
        for (const [subKey, subValue] of Object.entries(value as Record<string, unknown>)) {
          flat[`${key}.${subKey}`] = String(subValue ?? "");
        }
      } else {
        flat[key] = String(value ?? "");
      }
    }
    return flat;
  });

  const headers = [...new Set(flatRows.flatMap((r) => Object.keys(r)))];

  const escape = (v: string) => {
    if (v.includes(",") || v.includes('"') || v.includes("\n")) {
      return `"${v.replace(/"/g, '""')}"`;
    }
    return v;
  };

  const lines = [
    headers.map(escape).join(","),
    ...flatRows.map((row) => headers.map((h) => escape(row[h] ?? "")).join(",")),
  ];

  const blob = new Blob([lines.join("\n")], { type: "text/csv" });
  downloadBlob(blob, filename);
}
