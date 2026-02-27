import { useMemo } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { SchemaDefinition } from "@/lib/types";

export function EntityTypeInput({
  value,
  onChange,
  schemaCatalog,
  label = "Entity Type",
}: {
  value: string;
  onChange: (value: string) => void;
  schemaCatalog: SchemaDefinition[];
  label?: string;
}) {
  const schemaNames = useMemo(
    () => Array.from(new Set(schemaCatalog.map((schema) => schema.name))).sort(),
    [schemaCatalog],
  );

  if (schemaNames.length === 0) {
    return (
      <label className="space-y-1">
        <Label>{label}</Label>
        <Input value={value} onChange={(event) => onChange(event.target.value)} />
      </label>
    );
  }

  const hasExisting = schemaNames.includes(value);

  return (
    <div className="space-y-2">
      <label className="space-y-1">
        <Label>{label}</Label>
        <Select
          value={hasExisting ? value : "__custom__"}
          onValueChange={(next) => onChange(next === "__custom__" ? "" : next)}
        >
          <SelectTrigger>
            <SelectValue placeholder="Select schema entity" />
          </SelectTrigger>
          <SelectContent>
            {schemaNames.map((name) => (
              <SelectItem key={name} value={name}>
                {name}
              </SelectItem>
            ))}
            <SelectItem value="__custom__">Custom Entity</SelectItem>
          </SelectContent>
        </Select>
      </label>
      {!hasExisting ? (
        <label className="space-y-1">
          <Label>Custom Entity Name</Label>
          <Input value={value} onChange={(event) => onChange(event.target.value)} />
        </label>
      ) : null}
    </div>
  );
}
