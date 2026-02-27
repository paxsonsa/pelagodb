# Datasets and Loading

PelagoDB ships with bundled datasets and a loader for demos, benchmarks, and tutorials.

## Bundled Datasets

| Dataset | Description | Use Case |
|---|---|---|
| `social_graph` | People, relationships, and interests | Tutorials, quick demos |
| `ecommerce` | Customers, orders, products | Modeling patterns, traversal examples |
| `vfx_pipeline_50k` | VFX production hierarchy (Show → Sequence → Shot → Task) | Benchmarks, presentation demos |

## Loading Datasets

### Basic Load

```bash
python datasets/load_dataset.py social_graph --namespace demo
```

### Load with Custom Endpoint

```bash
python datasets/load_dataset.py vfx_pipeline_50k/show_001 \
  --endpoint 127.0.0.1:27615 \
  --database default \
  --namespace vfx.show.001
```

### Prerequisites

The dataset loader uses the Python SDK:

```bash
cd clients/python
python -m venv .venv
source .venv/bin/activate
pip install -e .
./scripts/generate_proto.sh
```

## Dataset Generator

For custom datasets or scaling tests, use the generator directly. See `datasets/README.md` for generator options and schema templates.

## Using Datasets in Tutorials

The [Build a Social Graph](../tutorials/build-a-social-graph.md) tutorial uses the `social_graph` dataset schema as its model.

The [Presentation Runbook](../operations/presentation-runbook.md) uses `vfx_pipeline_50k/show_001` for demo sequences.

## Using Datasets for Benchmarks

```bash
# Load benchmark dataset
python datasets/load_dataset.py vfx_pipeline_50k/show_001 \
  --endpoint 127.0.0.1:27615 \
  --namespace benchmark.vfx

# Run benchmark
scripts/perf-benchmark.py \
  --namespace benchmark.vfx \
  --entity-type Task \
  --seed-node-id 1_0 \
  --output-json .tmp/bench/latest.json
```

## Related

- [Quickstart](../getting-started/quickstart.md) — first data load
- [Build a Social Graph](../tutorials/build-a-social-graph.md) — tutorial using dataset model
- [Daily Operations](../operations/daily-operations.md) — benchmark procedures
- [Presentation Runbook](../operations/presentation-runbook.md) — demo dataset loading
