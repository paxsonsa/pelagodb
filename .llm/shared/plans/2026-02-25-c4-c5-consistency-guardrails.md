# C4/C5 Consistency Guardrails and API Limits

**Date:** 2026-02-25  
**Scope:** C4 (single-tx write correctness), C5 (pinned snapshot query correctness)  
**Authority:** `.llm/context/pelagodb-spec-v1.md`

---

## Problem Framing

- C4 and C5 improve correctness, but naive implementations can increase timeout, retry, and oversize risks.
- Large scans/traversals cannot always preserve strict single-snapshot behavior without unacceptable latency/failure rates.
- Client APIs must make these limits explicit rather than silently degrading behavior.

---

## C4 Guardrails (Write Paths)

### Hard Runtime Budgets

- `C4_TX_MAX_ATTEMPTS = 5`
- `C4_TX_TARGET_WALL_MS = 3000`
- `C4_TX_HARD_WALL_MS = 4500`
- `C4_TX_MAX_MUTATED_KEYS = 5000`
- `C4_TX_MAX_WRITE_BYTES = 2 MiB` (estimated, conservative guard)
- `C4_INLINE_CASCADE_MAX_EDGES = 1000`

### Required Behavior

- Read + validate + write for a mutation must occur in one transaction attempt.
- If projected work exceeds inline limits (key count/bytes/edge fanout), operation must:
  - fail fast with structured limit error (`INLINE_STRICT` mode), or
  - convert to async job (`ASYNC_ALLOWED` mode).
- Retryable FDB conflicts are retried with jittered backoff until `C4_TX_MAX_ATTEMPTS`.
- No silent fallback to multi-tx write logic.

### Client/API Surface (C4)

Add mutation execution mode to write-heavy APIs:

- `INLINE_STRICT`: atomic inline only; fail on limit breach.
- `ASYNC_ALLOWED`: server may enqueue background job.

Return/emit structured limit metadata:

- `reason`: `MUTATION_SCOPE_TOO_LARGE` | `TX_CONFLICT_RETRY_EXHAUSTED` | `TX_TIME_BUDGET_EXCEEDED`
- `estimated_mutated_keys`
- `estimated_write_bytes`
- `estimated_cascade_edges`
- `recommended_mode` (`ASYNC_ALLOWED` when applicable)

Status mapping:

- `RESOURCE_EXHAUSTED`: scope too large for inline tx.
- `ABORTED`: conflict retry exhausted.
- `DEADLINE_EXCEEDED`: tx wall budget exceeded.

---

## C5 Guardrails (Read Snapshot Paths)

### Consistency Modes

- `SNAPSHOT_STRICT`:
  - One pinned read version for the request execution window.
  - No silent read-version rotation.
- `SNAPSHOT_BEST_EFFORT`:
  - May rotate read version across segments/pages.
  - Must report degraded snapshot semantics explicitly.

### Hard Runtime Budgets (Strict Mode)

- `C5_STRICT_MAX_SNAPSHOT_MS = 2000`
- `C5_STRICT_MAX_SCAN_KEYS = 50000`
- `C5_STRICT_MAX_RESULT_BYTES = 8 MiB`
- `C5_STRICT_CURSOR_TTL_SEC = 30` (for continuation snapshots if supported)

### Required Behavior

- If strict budget is exceeded:
  - fail with explicit strict-snapshot limit error, or
  - if client requested fallback, continue in best-effort mode and mark degraded.
- Large traversal/scan operations should default to explicit `SNAPSHOT_BEST_EFFORT` unless client forces strict mode.
- No silent strict-to-best-effort downgrade.

### Client/API Surface (C5)

Add read consistency control fields to query/traversal APIs:

- `snapshot_mode`: `SNAPSHOT_STRICT` | `SNAPSHOT_BEST_EFFORT`
- optional `allow_degrade_to_best_effort` (default `false` for strict correctness)

Return metadata on all paged/streamed responses:

- `consistency_applied`: `STRICT` | `BEST_EFFORT`
- `snapshot_read_version` (strict only)
- `degraded` (bool)
- `degraded_reason` (if true)

Strict-mode error reasons:

- `SNAPSHOT_BUDGET_EXCEEDED`
- `SNAPSHOT_EXPIRED` (cursor/read-version expired)

Status mapping:

- `FAILED_PRECONDITION`: strict snapshot budget exceeded.
- `ABORTED`: snapshot expired.

---

## Operational Defaults

- Default mutation mode: `ASYNC_ALLOWED` for known high-fanout operations (`DeleteNode`, `DropEntityType`); `INLINE_STRICT` for bounded CRUD.
- Default query/traversal mode:
  - `SNAPSHOT_STRICT` for bounded point/small-page queries.
  - `SNAPSHOT_BEST_EFFORT` for deep traversal/large scans unless client opts into strict and accepts failure.

---

## Rollout Plan

1. Introduce API enums/fields and response metadata (backward-compatible defaults).
2. Enforce server-side budgets with explicit status details.
3. Add integration tests:
   - strict mode fails with correct reason under forced budget breach,
   - best-effort mode returns degraded metadata,
   - async mutation conversion path returns job metadata.
4. Document behavior in CLI/API docs and SDKs.
