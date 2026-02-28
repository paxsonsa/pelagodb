import { useMemo, useState } from "react";
import { AlertTriangle, ShieldCheck } from "lucide-react";

import { useConsoleContext } from "@/App";
import { EmptyState } from "@/components/shared/EmptyState";
import { JsonInspector } from "@/components/shared/JsonInspector";
import { SectionHeader } from "@/components/shared/SectionHeader";
import { StatusBadge } from "@/components/shared/StatusBadge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { apiRequest, normalizeApiError } from "@/lib/api";
import type { AdminMutationResult } from "@/lib/types";

type AdminRisk = "low" | "medium" | "high";

type ActionField = {
  name: string;
  label: string;
  placeholder: string;
};

type MutationConfig = {
  id: string;
  title: string;
  description: string;
  risk: AdminRisk;
  endpoint: string;
  confirmToken: string;
  fields: ActionField[];
};

const mutationConfigs: MutationConfig[] = [
  {
    id: "drop-index",
    title: "Drop Index",
    description: "Removes index entries for selected property.",
    risk: "medium",
    endpoint: "/admin/drop-index",
    confirmToken: "DROP_INDEX",
    fields: [
      { name: "entity_type", label: "Entity Type", placeholder: "Person" },
      { name: "property_name", label: "Property", placeholder: "age" },
    ],
  },
  {
    id: "strip-property",
    title: "Strip Property",
    description: "Schedules job to remove property from all nodes of entity type.",
    risk: "high",
    endpoint: "/admin/strip-property",
    confirmToken: "STRIP_PROPERTY",
    fields: [
      { name: "entity_type", label: "Entity Type", placeholder: "Person" },
      { name: "property_name", label: "Property", placeholder: "legacy_field" },
    ],
  },
  {
    id: "set-owner",
    title: "Set Namespace Schema Owner",
    description: "Assigns (or clears) namespace schema owner site.",
    risk: "medium",
    endpoint: "/admin/set-namespace-schema-owner",
    confirmToken: "SET_NAMESPACE_SCHEMA_OWNER",
    fields: [{ name: "site_id", label: "Site ID", placeholder: "1 (empty clears owner)" }],
  },
  {
    id: "transfer-owner",
    title: "Transfer Namespace Owner",
    description: "Moves namespace ownership after expected-site validation.",
    risk: "high",
    endpoint: "/admin/transfer-namespace-schema-owner",
    confirmToken: "TRANSFER_NAMESPACE_SCHEMA_OWNER",
    fields: [
      { name: "expected_site_id", label: "Expected Site", placeholder: "1" },
      { name: "target_site_id", label: "Target Site", placeholder: "2" },
    ],
  },
  {
    id: "transfer-node",
    title: "Transfer Node Ownership",
    description: "Transfers a single node's owner site.",
    risk: "high",
    endpoint: "/admin/transfer-node-ownership",
    confirmToken: "TRANSFER_NODE_OWNERSHIP",
    fields: [
      { name: "entity_type", label: "Entity Type", placeholder: "Person" },
      { name: "node_id", label: "Node ID", placeholder: "1_0" },
      { name: "target_site_id", label: "Target Site", placeholder: "2" },
    ],
  },
];

function defaultValues(): Record<string, Record<string, string>> {
  const entries = mutationConfigs.map((config) => {
    const fieldMap = Object.fromEntries(config.fields.map((field) => [field.name, ""]));
    return [config.id, { ...fieldMap, confirm: "" }];
  });

  return Object.fromEntries(entries);
}

export default function AdminView() {
  const { session, scope, notify } = useConsoleContext();
  const [values, setValues] = useState<Record<string, Record<string, string>>>(() => defaultValues());
  const [openDialogId, setOpenDialogId] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState<string | null>(null);
  const [result, setResult] = useState<AdminMutationResult | null>(null);

  const selectedConfig = useMemo(
    () => mutationConfigs.find((config) => config.id === openDialogId) ?? null,
    [openDialogId],
  );

  async function executeMutation(config: MutationConfig) {
    const state = values[config.id];
    const payload: Record<string, string> = {
      ...Object.fromEntries(config.fields.map((field) => [field.name, state[field.name] ?? ""])),
      confirm: state.confirm ?? "",
    };

    setSubmitting(config.id);

    try {
      const response = await apiRequest<unknown>(config.endpoint, {
        method: "POST",
        session,
        scope,
        body: payload,
      });
      setResult({ title: config.title, payload: response });
      notify(`${config.title} succeeded`);
      setOpenDialogId(null);
    } catch (error) {
      notify(normalizeApiError(error), "error");
    } finally {
      setSubmitting(null);
    }
  }

  return (
    <section className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>Safe Admin Mutations</CardTitle>
          <CardDescription>
            Risk-tiered actions with explicit confirmation phrases. Destructive namespace/type drops remain intentionally excluded.
          </CardDescription>
        </CardHeader>
      </Card>

      <div className="grid gap-4 xl:grid-cols-2">
        {mutationConfigs.map((config) => {
          const state = values[config.id];
          const badgeState = config.risk === "low" ? "ok" : config.risk === "medium" ? "warning" : "error";

          return (
            <Card key={config.id}>
              <CardHeader>
                <SectionHeader
                  title={config.title}
                  description={config.description}
                  actions={
                    <StatusBadge state={badgeState}>{config.risk.toUpperCase()} RISK</StatusBadge>
                  }
                />
              </CardHeader>
              <CardContent className="space-y-3">
                {config.fields.map((field) => (
                  <label key={field.name} className="space-y-1">
                    <Label>{field.label}</Label>
                    <Input
                      value={state[field.name] ?? ""}
                      onChange={(event) =>
                        setValues((prev) => ({
                          ...prev,
                          [config.id]: {
                            ...prev[config.id],
                            [field.name]: event.target.value,
                          },
                        }))
                      }
                      placeholder={field.placeholder}
                    />
                  </label>
                ))}

                <label className="space-y-1">
                  <Label>Confirmation Phrase ({config.confirmToken})</Label>
                  <Input
                    value={state.confirm ?? ""}
                    onChange={(event) =>
                      setValues((prev) => ({
                        ...prev,
                        [config.id]: {
                          ...prev[config.id],
                          confirm: event.target.value,
                        },
                      }))
                    }
                    placeholder={`Type ${config.confirmToken}`}
                    className="font-mono text-xs"
                  />
                </label>

                <Dialog
                  open={openDialogId === config.id}
                  onOpenChange={(open) => setOpenDialogId(open ? config.id : null)}
                >
                  <DialogTrigger asChild>
                    <Button type="button" variant={config.risk === "high" ? "danger" : "secondary"}>
                      Execute
                    </Button>
                  </DialogTrigger>
                  <DialogContent>
                    <DialogHeader>
                      <DialogTitle>{config.title}</DialogTitle>
                      <DialogDescription>
                        Verify this impact and confirmation phrase before execution.
                      </DialogDescription>
                    </DialogHeader>
                    <div className="rounded-lg border border-danger/35 bg-danger/12 p-3 text-sm text-danger">
                      <div className="mb-1 flex items-center gap-2">
                        <AlertTriangle className="h-4 w-4" />
                        <span className="font-semibold">Risk Notice</span>
                      </div>
                      <p>
                        This action is irreversible from the UI path. Ensure runbook approval and correct namespace selection.
                      </p>
                    </div>
                    <div className="rounded-md border border-border bg-surface-subtle p-3 text-xs">
                      <p className="mb-2 font-semibold uppercase tracking-wide text-muted">Payload Preview</p>
                      <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-[11px] text-foreground/80">
                        {JSON.stringify(
                          {
                            ...Object.fromEntries(
                              config.fields.map((field) => [field.name, state[field.name] ?? ""]),
                            ),
                            confirm: state.confirm,
                          },
                          null,
                          2,
                        )}
                      </pre>
                    </div>
                    <DialogFooter>
                      <Button type="button" variant="ghost" onClick={() => setOpenDialogId(null)}>
                        Cancel
                      </Button>
                      <Button
                        type="button"
                        variant={config.risk === "high" ? "danger" : "default"}
                        onClick={() => void executeMutation(config)}
                        disabled={submitting === config.id || state.confirm !== config.confirmToken}
                      >
                        {submitting === config.id ? "Executing..." : "Confirm and Execute"}
                      </Button>
                    </DialogFooter>
                  </DialogContent>
                </Dialog>
              </CardContent>
            </Card>
          );
        })}
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Mutation Result Inspector</CardTitle>
          <CardDescription>Most recent admin mutation response payload.</CardDescription>
        </CardHeader>
        <CardContent>
          {result ? (
            <>
              <div className="mb-3 flex items-center gap-2">
                <ShieldCheck className="h-4 w-4 text-accent" />
                <span className="text-sm font-semibold">{result.title}</span>
              </div>
              <JsonInspector value={result.payload} maxHeight={340} />
            </>
          ) : (
            <EmptyState
              title="No Mutation Executed"
              description="Execute one admin action to inspect response payloads here."
            />
          )}
        </CardContent>
      </Card>
    </section>
  );
}
