# Interactive Onboarding Course (60 Minutes)

This course is a complete, presentation-ready onboarding path for PelagoDB that runs from deployment through schema design, data creation, querying, and SDK integrations (Python + Elixir).

Course artifacts:
- Interactive runner: `scripts/onboarding-course.sh`
- Python exercise: `clients/python/examples/onboarding_course.py`
- Elixir exercise: `clients/elixir/examples/onboarding_course.exs`

## Target Audience
- Application engineers integrating PelagoDB
- Platform engineers operating PelagoDB instances
- Technical stakeholders evaluating PelagoDB in a live demo setting

## Learning Outcomes
By the end of the session, participants can:
1. Bring up/connect to a PelagoDB deployment.
2. Register schema-first entity models.
3. Create connected graph data with nodes and edges.
4. Query with CEL, traversal, and PQL.
5. Execute equivalent flows from Python and Elixir SDK wrappers.
6. Read operational signals from replication and audit admin surfaces.

## 60-Minute Module Matrix
| Module | Time | Capability | Delivery Surface | Completion Signal |
|---|---:|---|---|---|
| M1 | 10 min | Deployment and connectivity | CLI admin/schema commands | `schema list` and `admin sites` succeed |
| M2 | 8 min | Schema-first modeling | CLI schema register/get | `Person` + `Project` schemas visible |
| M3 | 10 min | Data creation and relationships | CLI node/edge commands | Nodes created and edges listed |
| M4 | 10 min | Querying modes | CLI query find/traverse/pql | All 3 query styles return rows |
| M5 | 8 min | Python integration | Python SDK example | Script creates and queries nodes |
| M6 | 8 min | Elixir integration | Elixir SDK example | Script creates and queries nodes |
| M7 | 6 min | Operations visibility | CLI admin replication/audit | Status and audit rows displayed |

## Detailed Matrix Checklist
| Module | Instructor Checklist | Learner Checklist | Validation |
|---|---|---|---|
| M1 | Confirm server endpoint, database, namespace; show deployment options | Connect CLI to endpoint | `pelago schema list`, `pelago admin sites` |
| M2 | Explain required properties and index choices | Register `Person` and `Project` schema | `pelago schema get Person`, `pelago schema get Project` |
| M3 | Explain node IDs and relationship labels | Create 2 people + 1 project + 2 edges | `pelago edge list Person <id> --dir out` |
| M4 | Compare CEL vs traversal vs PQL use-cases | Run one command per query style | `pelago query find`, `traverse`, `pql` |
| M5 | Show Python client construction and auth-ready pattern | Run Python onboarding script | Output includes created/updated node + query rows |
| M6 | Show Elixir client construction and stream handling | Run Elixir onboarding script | Output includes created/updated node + query rows |
| M7 | Tie admin commands to day-2 operations | Inspect replication and audit surfaces | `pelago admin replication-status`, `pelago admin audit` |

## Environment Setup (Before Session)

### 1) Server and CLI
Use existing local server or start one (single-site baseline):

```bash
export PELAGO_FDB_CLUSTER=/usr/local/etc/foundationdb/fdb.cluster
export PELAGO_SITE_ID=1
export PELAGO_SITE_NAME=local
export PELAGO_LISTEN_ADDR=127.0.0.1:27615
export PELAGO_AUTH_REQUIRED=false
cargo run -p pelago-server --bin pelago-server
```

In a second terminal, verify connectivity:

```bash
cargo run -p pelago-cli -- --server http://127.0.0.1:27615 --database default --namespace default schema list
```

### 2) Python SDK Prereqs
```bash
cd clients/python
python -m venv .venv
source .venv/bin/activate
pip install -e .
pip install -r requirements-dev.txt
./scripts/generate_proto.sh
```

### 3) Elixir SDK Prereqs
```bash
cd clients/elixir
mix deps.get
./scripts/generate_proto.sh
```

## Fast Interactive Delivery

Set course scope (recommended isolated namespace):

```bash
export PELAGO_SERVER=http://127.0.0.1:27615
export PELAGO_COURSE_DATABASE=default
export PELAGO_COURSE_NAMESPACE=onboarding.demo
```

Run the guided flow:

```bash
./scripts/onboarding-course.sh plan
./scripts/onboarding-course.sh preflight
./scripts/onboarding-course.sh run --mode learn
./scripts/onboarding-course.sh verify
```

For presentation mode without pauses:

```bash
./scripts/onboarding-course.sh run --mode demo
```

Reset the namespace between rehearsals:

```bash
./scripts/onboarding-course.sh reset
```

## Module-by-Module Delivery Script

### M1 Deploy + Connect (10 min)
Command:
```bash
./scripts/onboarding-course.sh preflight
```

Talking points:
- Deployment options: local single-site, local multi-site (`docker-compose.multisite.yml`), Kubernetes (`deploy/k8s`).
- Context routing by server/database/namespace.

### M2 Schema Design (8 min)
Commands:
```bash
pelago schema register --inline '{"name":"Person","properties":{"name":{"type":"string","required":true},"role":{"type":"string","index":"equality"},"level":{"type":"int","index":"range"}},"meta":{"allow_undeclared_edges":true,"extras_policy":"allow"}}'
pelago schema register --inline '{"name":"Project","properties":{"name":{"type":"string","required":true},"status":{"type":"string","index":"equality"},"priority":{"type":"int","index":"range"}},"meta":{"allow_undeclared_edges":true,"extras_policy":"allow"}}'
```

Talking points:
- Why schema-first improves consistency.
- Why `level` and `priority` are range-indexed.

### M3 Create Data + Edges (10 min)
Commands:
```bash
pelago node create Person name=Alice role=Lead level=4
pelago node create Person name=Bob role=Engineer level=2
pelago node create Project name=Onboarding status=active priority=1
pelago edge create Person:<alice_id> mentors Person:<bob_id>
pelago edge create Project:<project_id> assigned_to Person:<alice_id>
```

Talking points:
- ID and label conventions.
- Building domain graph semantics through edge labels.

### M4 Querying (10 min)
Commands:
```bash
pelago query find Person --filter "level >= 3" --limit 20
pelago query traverse Project:<project_id> assigned_to --max-depth 2 --max-results 20
pelago query pql --query "Person @filter(level >= 2) { uid name role level }"
```

Talking points:
- CEL for precise filters.
- Traverse for graph navigation.
- PQL for graph-native query expression.

### M5 Python SDK (8 min)
Command:
```bash
cd clients/python
PYTHONPATH=. PELAGO_ENDPOINT=127.0.0.1:27615 PELAGO_DATABASE=default PELAGO_NAMESPACE=onboarding.demo python3 examples/onboarding_course.py
```

Talking points:
- `PelagoClient` lifecycle and request context.
- Parity with CLI operations.

### M6 Elixir SDK (8 min)
Command:
```bash
cd clients/elixir
PELAGO_ENDPOINT=127.0.0.1:27615 PELAGO_DATABASE=default PELAGO_NAMESPACE=onboarding.demo mix run examples/onboarding_course.exs
```

Talking points:
- Wrapper ergonomics and stream consumption.
- Integrating query streams into Elixir workflows.

### M7 Ops Wrap-Up (6 min)
Commands:
```bash
pelago admin replication-status
pelago admin audit --limit 10
```

Talking points:
- Replication visibility as reliability signal.
- Audit logs as governance/security baseline.

## 1-Hour Presentation Narrative (Suggested)
1. Frame problem: distributed graph workloads need strict contracts and operational controls.
2. Deploy/connect quickly and establish confidence in environment health.
3. Model a compact but realistic domain (`Person` + `Project`).
4. Create relationships and query them with increasing expressiveness.
5. Re-run same workflows through Python and Elixir to show integration readiness.
6. Close on day-2 operations: replication and audit observability.

## Rehearsal Checklist
- [ ] Server process is up and reachable.
- [ ] Course namespace set and isolated.
- [ ] Python stubs generated and virtualenv ready.
- [ ] Elixir generated modules present and deps fetched.
- [ ] `./scripts/onboarding-course.sh run --mode demo` succeeds end-to-end.
- [ ] `./scripts/onboarding-course.sh verify` passes.

## Fallback Path (If Live SDK Demo Fails)
1. Continue with CLI modules M1-M4 and M7.
2. Show code in `clients/python/examples/onboarding_course.py` and `clients/elixir/examples/onboarding_course.exs`.
3. Run `verify` to demonstrate data/query outcomes still achieved.
