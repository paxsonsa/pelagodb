#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Dict, Iterable, Tuple

ROOT = Path(__file__).resolve().parents[1]
PYTHON_SDK_PATH = ROOT / "clients" / "python"
if str(PYTHON_SDK_PATH) not in sys.path:
    sys.path.insert(0, str(PYTHON_SDK_PATH))

try:
    from pelagodb import PelagoClient
except Exception as exc:  # pragma: no cover
    print(
        "Failed to import Python SDK. Generate stubs first: ./clients/python/scripts/generate_proto.sh",
        file=sys.stderr,
    )
    raise


def read_json(path: Path):
    with path.open("r", encoding="utf-8") as f:
        return json.load(f)


def iter_jsonl(path: Path) -> Iterable[dict]:
    with path.open("r", encoding="utf-8") as f:
        for i, line in enumerate(f, start=1):
            line = line.strip()
            if not line:
                continue
            try:
                yield json.loads(line)
            except json.JSONDecodeError as exc:
                raise ValueError(f"Invalid JSON in {path}:{i}: {exc}") from exc


def load_dataset(
    dataset_dir: Path,
    *,
    endpoint: str,
    database: str,
    namespace: str,
    api_key: str | None,
    bearer_token: str | None,
    skip_edges: bool,
) -> None:
    schemas_path = dataset_dir / "schemas.json"
    nodes_path = dataset_dir / "nodes.jsonl"
    edges_path = dataset_dir / "edges.jsonl"

    if not schemas_path.exists():
        raise FileNotFoundError(f"Missing {schemas_path}")
    if not nodes_path.exists():
        raise FileNotFoundError(f"Missing {nodes_path}")
    if not edges_path.exists():
        raise FileNotFoundError(f"Missing {edges_path}")

    client = PelagoClient(
        endpoint,
        database=database,
        namespace=namespace,
        api_key=api_key,
        bearer_token=bearer_token,
    )

    try:
        schemas = read_json(schemas_path)
        print(f"Registering {len(schemas)} schemas from {schemas_path}...")
        for schema in schemas:
            client.register_schema_dict(schema)

        alias_to_node: Dict[str, Tuple[str, str]] = {}
        node_count = 0
        print(f"Loading nodes from {nodes_path}...")
        for node_record in iter_jsonl(nodes_path):
            alias = node_record["id"]
            entity_type = node_record["entity_type"]
            props = node_record.get("properties", {})
            created = client.create_node(entity_type, props)
            alias_to_node[alias] = (entity_type, created.id)
            node_count += 1

        edge_count = 0
        if skip_edges:
            print(f"Skipping edges from {edges_path} (--skip-edges)")
        else:
            print(f"Loading edges from {edges_path}...")
            for edge_record in iter_jsonl(edges_path):
                source_alias = edge_record["source"]
                target_alias = edge_record["target"]
                label = edge_record["label"]
                props = edge_record.get("properties", {})

                source_type, source_id = alias_to_node[source_alias]
                target_type, target_id = alias_to_node[target_alias]
                client.create_edge(source_type, source_id, label, target_type, target_id, props)
                edge_count += 1

        print("Done")
        print(f"Dataset: {dataset_dir.name}")
        print(f"Database/Namespace: {database}/{namespace}")
        print(f"Nodes loaded: {node_count}")
        print(f"Edges loaded: {edge_count}")
    finally:
        client.close()


def main() -> None:
    parser = argparse.ArgumentParser(description="Load PelagoDB example datasets")
    parser.add_argument(
        "dataset",
        help=(
            "Dataset path under datasets/ "
            "(e.g. social_graph, ecommerce, vfx_pipeline_50k/show_001)"
        ),
    )
    parser.add_argument("--endpoint", default="127.0.0.1:27615")
    parser.add_argument("--database", default="default")
    parser.add_argument("--namespace", default="default")
    parser.add_argument("--api-key", default=None)
    parser.add_argument("--bearer-token", default=None)
    parser.add_argument(
        "--skip-edges",
        action="store_true",
        help="Load schemas and nodes only (skip edge creation)",
    )
    args = parser.parse_args()

    dataset_dir = ROOT / "datasets" / args.dataset
    if not dataset_dir.exists():
        raise FileNotFoundError(f"Dataset not found: {dataset_dir}")

    load_dataset(
        dataset_dir,
        endpoint=args.endpoint,
        database=args.database,
        namespace=args.namespace,
        api_key=args.api_key,
        bearer_token=args.bearer_token,
        skip_edges=args.skip_edges,
    )


if __name__ == "__main__":
    main()
