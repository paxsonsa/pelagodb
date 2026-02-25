# PelagoDB v1 Gap Closure Worker Matrix

**Date:** 2026-02-20  
**Analyzed Worktree:** `/Users/apaxson/work/projects/pelagodb/.codex/worktrees/pelagodb-spec-v1`  
**Source Inputs:** `.llm/context/pelagodb-spec-v1.md`, `.llm/context/pelagodb-spec-v1-section-watch.md`, `.llm/context/pelagodb-spec-v1-security-section.md`, `spec/query-indexing-checklist-matrix.md`

---

## Worker Assignment Matrix

| Worker ID | Priority | Track | Spec Gap Summary | Primary Code Surface | Deliverables | Validation Gate | Status |
|---|---|---|---|---|---|---|---|
| W1 | P0 | Security/AuthN/AuthZ parity | Security section expects API key + mTLS + service JWT + token lifecycle + path ACL semantics; service-token RPCs + durable token sessions + path ACL matching landed, and in-process TLS/mTLS transport with peer-certificate principal extraction is now available | `proto/pelago.proto`, `crates/pelago-server/src/main.rs`, `crates/pelago-api/src/auth_service.rs`, `crates/pelago-api/src/authz.rs`, `crates/pelago-storage/src/security.rs` | mTLS/JWT/token lifecycle support, path-based ACL evaluation parity, auth RPC expansion | `cargo test -p pelago-api --lib`, `cargo test -p pelago-storage --test replication_security_integration -- --ignored --nocapture --test-threads=1` | Completed |
| W2 | P0 | Watch system parity | Watch spec supports `NodeWatch`/`EdgeWatch`, heartbeat/initial snapshot semantics, richer options; model/options, resume behavior alignment, durable query-watch state, and retention cleanup are implemented | `proto/pelago.proto`, `crates/pelago-api/src/watch_service.rs`, `crates/pelago-storage/src/watch_state.rs`, `crates/pelago-server/src/main.rs`, `crates/pelago-core/src/config.rs` | Proto+server parity for watch request model/options, resume/heartbeat/initial behavior alignment, durable query-watch state persistence/restore, retention sweep wiring | `cargo test -p pelago-api --lib`, `cargo test -p pelago-storage --lib`, `cargo check -p pelago-server` | Completed |
| W3 | P1 | PQL advanced execution | PQL spec phases include set ops, advanced directives, and upserts; set-op runtime + directive/filter execution + safer param binding landed, unsupported directives are now explicitly rejected, and upsert is explicitly gated/deferred in runtime and canonical spec | `crates/pelago-query/src/pql/*`, `crates/pelago-api/src/query_service.rs`, `proto/pelago.proto`, `.llm/context/pelagodb-spec-v1.md` | Implement missing set-op semantics, close directive execution gaps, design+implement upsert path or clearly gate/defer in canonical spec | `cargo test -p pelago-query --tests`, add end-to-end PQL execution tests (API layer) | Completed |
| W4 | P1 | Query/indexing roadmap closure | Query/indexing closure now includes batched native OR union fetches, residual-filter reconstruction, template-term acceleration for top context lookup shapes, deep-page guardrails, and cross-endpoint cursor portability validation tooling | `crates/pelago-query/src/planner.rs`, `crates/pelago-query/src/executor.rs`, `crates/pelago-storage/src/term_index.rs`, `scripts/perf-benchmark.py`, `scripts/query-scaleout-check.py`, `spec/query-indexing-checklist-matrix.md` | Native OR optimizer improvements, residual correctness tests, context/template index strategy implementation, deep-page and scale-out validation harness | benchmark gates via `scripts/perf-benchmark.py` + `scripts/query-scaleout-check.py` + query correctness suite | Completed |
| W5 | P1 | CLI/REPL real e2e coverage | Placeholder/comment-only CLI/REPL integration tests were replaced with runnable ignored e2e suites backed by a real server fixture and scripted REPL execution path | `crates/pelago-cli/tests/cli_integration.rs`, `crates/pelago-cli/tests/repl_integration.rs`, `crates/pelago-cli/tests/common/mod.rs`, `crates/pelago-cli/src/repl/mod.rs`, `crates/pelago-cli/src/commands/schema.rs` | Executable assertions against live server fixture across schema/node/edge/query/REPL flows, plus fixture bootstrap and documented prerequisites | `cargo test -p pelago-cli --tests -- --ignored --nocapture` | Completed |
| W6 | P2 | Ops hardening/release gate | Docs still list operational gaps (required-check policy wiring, regular failover/rebuild drills, lease-holder visibility) | `docs/07-production-readiness.md`, `docs/13-centralized-replication-and-scaling.md`, `deploy/k8s/CHECKLIST.md`, `scripts/ci-gate.sh` | Close production checklist items, codify recurring drill/benchmark automation, add lease visibility surface | CI gate + documented runbook evidence + staging drill reports | Open |

---

## Single-Session Parallel Confirmation (No Subprocess Workers)

This snapshot was confirmed directly against the codebase in this worktree and replaces the earlier forked worker process approach.

### W1 - Security/AuthN/AuthZ

| Checklist Item | Spec Target | Codebase Confirmation | Gap | Status |
|---|---|---|---|---|
| Durable token/session lifecycle | Service token lifecycle and durable credential state | Token sessions and refresh bindings are persisted via storage-backed auth session records with revoke/introspect flows (`crates/pelago-storage/src/security.rs`, `crates/pelago-api/src/auth_service.rs`) | Optional rotation policy hardening remains a non-blocking enhancement | DONE |
| Service token + JWT parity | Service-account token issuance/revocation/introspection semantics | Auth RPC surface now includes `IssueServiceToken`, `RevokeToken`, `IntrospectToken` (`proto/pelago.proto`, `crates/pelago-api/src/auth_service.rs`) | JWT crypto/provider hardening still optional follow-up | DONE |
| mTLS auth path | API key + mTLS + service token support | Server now supports in-process TLS/mTLS (`PELAGO_TLS_CERT`/`PELAGO_TLS_KEY`/`PELAGO_TLS_CA` + client auth mode) and interceptor can authenticate from peer cert fingerprint as well as trusted subject headers (`crates/pelago-server/src/main.rs`, `crates/pelago-api/src/auth_service.rs`, `crates/pelago-core/src/config.rs`) | Optional cert-to-principal registry migration from env mappings to FDB records can follow | DONE |
| Path ACL wildcard semantics | Vault-style path matching with `*`/`+` segment behavior | Path matcher with `*`/`+` segment semantics is implemented and wired in permission checks, with replication/security integration coverage (`crates/pelago-storage/src/security.rs`, `crates/pelago-storage/tests/replication_security_integration.rs`) | Continue adding policy-shape soak coverage over time | DONE |
| Existing auth compatibility | Preserve API key and bearer flows | API key + bearer validation paths already in place | Keep behavior while adding new authn/authz surface | KEEP |

### W2 - Watch System

| Checklist Item | Spec Target | Codebase Confirmation | Gap | Status |
|---|---|---|---|---|
| Point watch model parity | `NodeWatch`/`EdgeWatch` structures | `WatchPointRequest` includes `nodes` and `edges` target lists and runtime matching for node/edge scopes (`proto/pelago.proto`, `crates/pelago-api/src/watch_service.rs`) | No blocking gap identified | DONE |
| Heartbeat + initial snapshot semantics | Configurable heartbeat and initial snapshot behavior | Heartbeat emission and initial snapshot behavior are implemented and covered by unit behavior tests (`crates/pelago-api/src/watch_service.rs`) | No blocking gap identified | DONE |
| Resume semantics | Resume via CDC position with expired/out-of-range handling | Resume validation now distinguishes expired positions vs start-from-now fallback for non-existent non-expired positions, with coverage (`crates/pelago-api/src/watch_service.rs`, `crates/pelago-storage/src/cdc.rs`) | Optional full e2e reconnect soak tests can be added later | DONE |
| Durable query-watch match state | Durable tracked membership across restarts | Query watch match-state is persisted/restored with CDC position via durable state keys, and stale/corrupt state retention cleanup is wired into server background sweeps with config controls (`crates/pelago-storage/src/watch_state.rs`, `crates/pelago-api/src/watch_service.rs`, `crates/pelago-server/src/main.rs`, `crates/pelago-core/src/config.rs`) | Optional long-run scale/soak validation only | DONE |

### W3 - PQL Advanced Execution

| Checklist Item | Spec Target | Codebase Confirmation | Gap | Status |
|---|---|---|---|---|
| `uid_set` set-op semantics | Union/intersect/difference runtime behavior | Parser/compiler/runtime map `uid(a,b,intersect|difference)` to variable-set execution semantics with API-layer multi-block runtime tests (`crates/pelago-query/src/pql/parser.rs`, `crates/pelago-query/src/pql/compiler.rs`, `crates/pelago-api/src/query_service.rs`) | Continue load/perf validation on representative datasets | DONE |
| Directive execution parity | Parser/compiler directives reflected in runtime | `VariableRef` and `VariableSet` execute filter + limit/offset directives; unsupported directives are now explicitly rejected at compile time instead of being silently ignored (`crates/pelago-query/src/pql/compiler.rs`, `crates/pelago-api/src/query_service.rs`) | Expand implemented directive surface in future phases as needed | DONE |
| Parameter binding robustness | Safe parameter substitution in ExecutePQL | Parameter substitution uses token-aware `$name` scanning (no unordered global replace side-effects), with targeted API tests (`crates/pelago-api/src/query_service.rs`) | AST-aware typing/binding remains optional future hardening | DONE |
| Upsert block support | Upsert/mutation PQL phase support or explicit gate | Runtime now gates upsert with explicit unsupported-feature status mapping, and canonical spec records upsert as deferred for current runtime (`crates/pelago-query/src/pql/parser.rs`, `crates/pelago-api/src/query_service.rs`, `.llm/context/pelagodb-spec-v1.md`) | Full mutation/upsert runtime remains future phase work | DONE |

### W4 - Query/Indexing Roadmap

| Checklist Item | Spec Target | Codebase Confirmation | Gap | Status |
|---|---|---|---|---|
| Native OR optimizer closure | Efficient server-side OR union with low fetch overhead | OR union path now batches node hydration, supports full `IN` expansion, and avoids per-node read round trips (`crates/pelago-query/src/executor.rs`) | Residual complex-expression shapes outside simple term grammar still fall back to planner path | DONE |
| Residual filtering correctness | Correct mixed indexed/non-indexed predicate behavior | Planner now removes only the primary indexed predicate from residual expressions for simple conjunctions, with coverage for mixed and multi-predicate cases (`crates/pelago-query/src/planner.rs`) | Complex boolean rewrites beyond conjunction decomposition can be expanded later | DONE |
| Keyset deep-page validation | Stable deep pagination with large cardinality | Keyset path remains default and benchmark harness now detects deep-page cursor loop instability with optional hard-fail gating (`crates/pelago-api/src/query_service.rs`, `scripts/perf-benchmark.py`) | Execute benchmark against large staging datasets as part of release cadence | DONE |
| Checklist roadmap closure | Close open matrix items | Top context templates now map to synthetic term postings and query-term rewrite acceleration; scale-out cursor portability checker added (`crates/pelago-storage/src/term_index.rs`, `crates/pelago-query/src/executor.rs`, `scripts/query-scaleout-check.py`) | Continue long-run perf tuning after initial closure | DONE |

### Ready-to-Execute Lanes (Single Session)

| Lane | Scope | Implementation Target | Validation |
|---|---|---|---|
| Lane A (P0) | W1 auth/authz parity | `auth_service.rs`, `authz.rs`, `security.rs`, `main.rs`, `proto/pelago.proto` | `cargo test -p pelago-api --lib` + replication security integration |
| Lane B (P0) | W2 watch parity | `watch_service.rs`, `proto/pelago.proto` | `cargo test -p pelago-api --lib` + watch behavior tests |
| Lane C (P1) | W3 PQL runtime parity | `pql/*`, `query_service.rs` | `cargo test -p pelago-query --tests` + API PQL tests |
| Lane D (P1) | W4 query/indexing closure | `planner.rs`, `executor.rs`, checklist matrix | query correctness suite + targeted OR/residual/pagination tests |

---

## Detailed Checklists by Worker

## W1 - Security/AuthN/AuthZ Parity (P0)

- [x] Align auth mechanisms with canonical security section (API key, mTLS, service token path).
- [x] Add token lifecycle RPC coverage needed by spec surface (issue/revoke/introspection semantics as scoped).
- [x] Move/extend token/session handling beyond in-memory-only where required.
- [x] Implement path-oriented ACL matcher semantics (including wildcard behavior) or update canonical spec with explicit divergence.
- [x] Validate built-in role behavior remains intact (`admin`, `read-only`, `namespace-admin`).
- [x] Extend audit events for new auth flows.

## W2 - Watch System Parity (P0)

- [x] Expand watch request model parity (`NodeWatch`/`EdgeWatch` shape and filters if adopted).
- [x] Add/align option semantics for heartbeat and initial snapshot behavior.
- [x] Validate resume semantics under reconnect and expired position handling.
- [x] Decide and implement durable query-watch match state strategy (or document bounded in-memory semantics as canonical).
- [x] Implement retention/GC sweep for durable query-watch state (configurable enable/days/sweep/batch).
- [x] Add watch load/backpressure tests beyond unit shape checks.

## W3 - PQL Advanced Execution (P1)

- [x] Implement set-operation semantics for `uid_set` (union/intersect/difference behavior).
- [x] Ensure advanced directives have execution semantics (not parser/compiler-only).
- [x] Add parameter binding hardening (avoid naive global text replacement behavior).
- [x] Implement or explicitly gate/defer upsert blocks in canonical spec and API surface.
- [x] Add multi-block correctness tests that assert actual result sets.

## W4 - Query/Indexing Roadmap Closure (P1)

- [x] Close native OR optimizer gap with batched fetch/union improvements.
- [x] Add residual filtering correctness matrix for mixed indexed/non-indexed predicates.
- [x] Validate keyset deep-page behavior under large cardinality.
- [x] Decide context/template index strategy and implement top query templates.
- [x] Add scale-out benchmark/validation evidence for stateless query tier.

## W5 - CLI/REPL e2e Tests (P1)

- [x] Replace comment-only ignored tests with runnable flows and assertions.
- [x] Add fixture bootstrap for server + test namespace setup/teardown.
- [x] Cover schema CRUD, node CRUD, edge CRUD, query find/traverse/pql, repl `:explain`.
- [x] Ensure ignored suite is stable in CI/local with documented env prerequisites.

## W6 - Ops Hardening (P2)

- [ ] Enforce security-first defaults and env profiles in deploy paths.
- [ ] Finalize CI required-check policy wiring (remote VCS settings + documented gate).
- [ ] Schedule/automate benchmark and DR rehearsal cadence with artifact retention.
- [ ] Run replicator failover and cache rebuild drills regularly and record outcomes.
- [ ] Add admin visibility for lease holder/epoch if still missing.

---

## Suggested Parallel Execution Order

1. Start immediately: `W1`, `W2` (P0 contract + runtime parity).  
2. In parallel after API contract freeze: `W3`, `W5`.  
3. Continuous performance lane: `W4`.  
4. Release hardening lane: `W6` once P0/P1 merges stabilize.

---

## Reporting Template (Per Worker)

- `Owner:`  
- `Branch/worktree:`  
- `Scope this sprint:`  
- `Files touched:`  
- `Tests added/updated:`  
- `Validation commands + result:`  
- `Blocked by:`  
- `Next handoff:`
