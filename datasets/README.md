# Example Datasets

This directory contains demo datasets for PelagoDB presentations and functional testing.

## Included
- `datasets/social_graph`
- `datasets/ecommerce`
- `datasets/vfx_pipeline_50k` (50k nodes across 12 namespaces)

Each dataset contains:
- `schemas.json` (list of entity schemas)
- `nodes.jsonl` (node records)
- `edges.jsonl` (edge records)

## Loader
Use Python loader:

```bash
# 1) Generate Python protobuf stubs
./clients/python/scripts/generate_proto.sh

# 2) Load dataset
python datasets/load_dataset.py social_graph \
  --endpoint 127.0.0.1:27615 \
  --database default \
  --namespace demo
```

For multi-namespace datasets, load each namespace partition:

```bash
for i in $(seq -w 1 12); do
  python datasets/load_dataset.py "vfx_pipeline_50k/show_${i}" \
    --endpoint 127.0.0.1:27615 \
    --database default \
    --namespace "vfx.show.${i}"
done
```

## VFX Pipeline 50K Dataset
`vfx_pipeline_50k` is a deterministic, deep production graph with:
- `50,000` nodes
- `154,164` edges
- `12` namespaces (`vfx.show.001` to `vfx.show.012`)
- Entity types: `Studio`, `Artist`, `Department`, `Tool`, `Show`, `Sequence`, `Shot`, `Task`, `Asset`, `Reviewable`, `Review`, `Note`

Main traversal chain shape:
- `Show -> Sequence -> Shot -> Task -> depends_on(Task) -> Reviewable -> Asset`
- Plus review workflow edges: `Reviewable -> Review -> Note`

Metadata and per-namespace counts live in:
- `datasets/vfx_pipeline_50k/manifest.json`

To regenerate the dataset family:

```bash
python3 datasets/generate_vfx_pipeline.py --clean
```

## JSON Formats
`schemas.json`:
```json
[
  {
    "name": "Person",
    "properties": {
      "name": {"type": "string", "required": true}
    },
    "edges": {
      "follows": {"target": "Person", "direction": "outgoing"}
    }
  }
]
```

`nodes.jsonl`:
```json
{"id":"p1","entity_type":"Person","properties":{"name":"Alice"}}
```

`edges.jsonl`:
```json
{"source":"p1","label":"follows","target":"p2","properties":{}}
```
