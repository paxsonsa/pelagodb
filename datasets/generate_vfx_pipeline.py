#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import TextIO

ROOT = Path(__file__).resolve().parent

SHOW_COUNT = 12
SEQUENCES_PER_SHOW = 20
SHOTS_PER_SEQUENCE = 18
TASKS_PER_SHOT = 4
REVIEWABLES_PER_SHOT = 1
REVIEWS_PER_REVIEWABLE = 1
NOTES_PER_REVIEW = 2
ASSETS_PER_SHOW = 560

STUDIOS_PER_SHOW = 1
DEPARTMENTS_PER_STUDIO = 8
TOOLS_PER_SHOW = 24
ARTISTS_PER_SHOW_BASE = 300
FIRST_SHOW_ARTIST_BONUS = 152


@dataclass
class NamespaceSummary:
    namespace: str
    dataset: str
    nodes: int
    edges: int


DEPARTMENTS = [
    "Layout",
    "Modeling",
    "Surfacing",
    "Rigging",
    "Animation",
    "Lighting",
    "FX",
    "Compositing",
]

DISCIPLINES = [
    "layout",
    "modeling",
    "surfacing",
    "rigging",
    "animation",
    "lighting",
    "fx",
    "compositing",
]

SENIORITIES = ["junior", "mid", "senior", "lead", "supervisor"]
TIMEZONES = ["UTC-8", "UTC-5", "UTC+0", "UTC+1", "UTC+5:30", "UTC+9"]
TOOL_FAMILIES = [
    "Maya",
    "Houdini",
    "Nuke",
    "Katana",
    "Substance",
    "RenderMan",
    "Arnold",
    "Blender",
    "Mari",
    "Unreal",
    "ZBrush",
    "Clarisse",
]
TASK_STAGES = ["layout", "animation", "fx", "comp"]
TASK_STATUS = ["queued", "in_progress", "review", "approved", "blocked"]
ASSET_KINDS = ["character", "environment", "prop", "matte", "groom", "sim_cache", "camera", "rig"]
REVIEWABLE_KINDS = ["playblast", "comp", "turntable", "wedge"]
REVIEW_STATUS = ["pass", "needs_work", "blocked"]
NOTE_SEVERITY = ["nit", "minor", "major", "critical"]


def schema_definitions() -> list[dict]:
    return [
        {
            "name": "Studio",
            "properties": {
                "studio_code": {"type": "string", "required": True, "index": "unique"},
                "name": {"type": "string", "required": True},
                "region": {"type": "string", "index": "equality"},
                "active_shows": {"type": "int", "index": "range"},
            },
            "edges": {
                "has_department": {"target": "Department", "direction": "outgoing"},
                "owns_show": {"target": "Show", "direction": "outgoing"},
            },
        },
        {
            "name": "Department",
            "properties": {
                "department_code": {"type": "string", "required": True, "index": "unique"},
                "name": {"type": "string", "required": True},
                "discipline": {"type": "string", "index": "equality"},
            },
            "edges": {"staffed_by": {"target": "Artist", "direction": "outgoing"}},
        },
        {
            "name": "Artist",
            "properties": {
                "badge_id": {"type": "string", "required": True, "index": "unique"},
                "name": {"type": "string", "required": True},
                "discipline": {"type": "string", "index": "equality"},
                "seniority": {"type": "string", "index": "equality"},
                "timezone": {"type": "string", "index": "equality"},
            },
            "edges": {"uses_tool": {"target": "Tool", "direction": "outgoing"}},
        },
        {
            "name": "Tool",
            "properties": {
                "tool_id": {"type": "string", "required": True, "index": "unique"},
                "name": {"type": "string", "required": True},
                "vendor": {"type": "string", "index": "equality"},
                "major_version": {"type": "int", "index": "range"},
            },
        },
        {
            "name": "Show",
            "properties": {
                "show_code": {"type": "string", "required": True, "index": "unique"},
                "title": {"type": "string", "required": True},
                "phase": {"type": "string", "index": "equality"},
                "priority": {"type": "int", "index": "range"},
            },
            "edges": {
                "has_sequence": {"target": "Sequence", "direction": "outgoing"},
                "has_asset": {"target": "Asset", "direction": "outgoing"},
            },
        },
        {
            "name": "Sequence",
            "properties": {
                "sequence_code": {"type": "string", "required": True, "index": "unique"},
                "name": {"type": "string", "required": True},
                "order": {"type": "int", "index": "range"},
            },
            "edges": {"has_shot": {"target": "Shot", "direction": "outgoing"}},
        },
        {
            "name": "Shot",
            "properties": {
                "shot_code": {"type": "string", "required": True, "index": "unique"},
                "name": {"type": "string", "required": True},
                "order": {"type": "int", "index": "range"},
                "cut_duration_frames": {"type": "int", "index": "range"},
            },
            "edges": {
                "has_task": {"target": "Task", "direction": "outgoing"},
                "has_reviewable": {"target": "Reviewable", "direction": "outgoing"},
                "next_shot": {"target": "Shot", "direction": "outgoing"},
            },
        },
        {
            "name": "Task",
            "properties": {
                "task_code": {"type": "string", "required": True, "index": "unique"},
                "name": {"type": "string", "required": True},
                "stage": {"type": "string", "index": "equality"},
                "status": {"type": "string", "index": "equality"},
                "priority": {"type": "int", "index": "range"},
            },
            "edges": {
                "depends_on": {"target": "Task", "direction": "outgoing"},
                "assigned_to": {"target": "Artist", "direction": "outgoing"},
                "uses_tool": {"target": "Tool", "direction": "outgoing"},
                "produces_reviewable": {"target": "Reviewable", "direction": "outgoing"},
            },
        },
        {
            "name": "Asset",
            "properties": {
                "asset_code": {"type": "string", "required": True, "index": "unique"},
                "name": {"type": "string", "required": True},
                "kind": {"type": "string", "index": "equality"},
                "version": {"type": "int", "index": "range"},
            },
            "edges": {"created_by": {"target": "Artist", "direction": "outgoing"}},
        },
        {
            "name": "Reviewable",
            "properties": {
                "reviewable_code": {"type": "string", "required": True, "index": "unique"},
                "kind": {"type": "string", "index": "equality"},
                "frame_count": {"type": "int", "index": "range"},
            },
            "edges": {
                "uses_asset": {"target": "Asset", "direction": "outgoing"},
                "has_review": {"target": "Review", "direction": "outgoing"},
            },
        },
        {
            "name": "Review",
            "properties": {
                "review_code": {"type": "string", "required": True, "index": "unique"},
                "status": {"type": "string", "index": "equality"},
                "score": {"type": "int", "index": "range"},
            },
            "edges": {
                "has_note": {"target": "Note", "direction": "outgoing"},
                "targets_task": {"target": "Task", "direction": "outgoing"},
            },
        },
        {
            "name": "Note",
            "properties": {
                "note_code": {"type": "string", "required": True, "index": "unique"},
                "severity": {"type": "string", "index": "equality"},
                "line": {"type": "int", "index": "range"},
                "message": {"type": "string", "required": True},
            },
            "edges": {"authored_by": {"target": "Artist", "direction": "outgoing"}},
        },
    ]


def write_jsonl_line(handle: TextIO, payload: dict) -> None:
    handle.write(json.dumps(payload, separators=(",", ":")))
    handle.write("\n")


def write_schema_file(path: Path) -> None:
    path.write_text(json.dumps(schema_definitions(), indent=2), encoding="utf-8")


def generate_namespace(dataset_dir: Path, show_idx: int, artist_count: int) -> NamespaceSummary:
    folder = dataset_dir / f"show_{show_idx:03d}"
    folder.mkdir(parents=True, exist_ok=True)
    write_schema_file(folder / "schemas.json")

    ns = f"vfx.show.{show_idx:03d}"
    prefix = f"s{show_idx:03d}"

    node_count = 0
    edge_count = 0

    nodes_path = folder / "nodes.jsonl"
    edges_path = folder / "edges.jsonl"

    with nodes_path.open("w", encoding="utf-8") as nodes_out, edges_path.open("w", encoding="utf-8") as edges_out:
        studio_ids: list[str] = []
        department_ids: list[str] = []
        artist_ids: list[str] = []
        tool_ids: list[str] = []
        asset_ids: list[str] = []

        for studio_num in range(1, STUDIOS_PER_SHOW + 1):
            studio_id = f"{prefix}_studio_{studio_num:02d}"
            studio_ids.append(studio_id)
            write_jsonl_line(
                nodes_out,
                {
                    "id": studio_id,
                    "entity_type": "Studio",
                    "properties": {
                        "studio_code": f"{prefix.upper()}-ST{studio_num:02d}",
                        "name": f"Studio {show_idx:03d}-{studio_num:02d}",
                        "region": ["us-west", "us-east", "uk", "ca"][show_idx % 4],
                        "active_shows": 1,
                    },
                },
            )
            node_count += 1

        for dept_idx in range(DEPARTMENTS_PER_STUDIO):
            dept_id = f"{prefix}_dept_{dept_idx + 1:02d}"
            department_ids.append(dept_id)
            discipline = DISCIPLINES[dept_idx % len(DISCIPLINES)]
            write_jsonl_line(
                nodes_out,
                {
                    "id": dept_id,
                    "entity_type": "Department",
                    "properties": {
                        "department_code": f"{prefix.upper()}-D{dept_idx + 1:02d}",
                        "name": DEPARTMENTS[dept_idx % len(DEPARTMENTS)],
                        "discipline": discipline,
                    },
                },
            )
            node_count += 1

            write_jsonl_line(
                edges_out,
                {
                    "source": studio_ids[0],
                    "label": "has_department",
                    "target": dept_id,
                    "properties": {},
                },
            )
            edge_count += 1

        for tool_idx in range(TOOLS_PER_SHOW):
            tool_id = f"{prefix}_tool_{tool_idx + 1:03d}"
            tool_ids.append(tool_id)
            family = TOOL_FAMILIES[tool_idx % len(TOOL_FAMILIES)]
            write_jsonl_line(
                nodes_out,
                {
                    "id": tool_id,
                    "entity_type": "Tool",
                    "properties": {
                        "tool_id": f"{prefix.upper()}-T{tool_idx + 1:03d}",
                        "name": f"{family} {tool_idx % 5 + 1}",
                        "vendor": family,
                        "major_version": 2022 + (tool_idx % 4),
                    },
                },
            )
            node_count += 1

        for artist_idx in range(artist_count):
            artist_id = f"{prefix}_artist_{artist_idx + 1:04d}"
            artist_ids.append(artist_id)
            discipline = DISCIPLINES[artist_idx % len(DISCIPLINES)]
            dept_id = department_ids[artist_idx % len(department_ids)]
            write_jsonl_line(
                nodes_out,
                {
                    "id": artist_id,
                    "entity_type": "Artist",
                    "properties": {
                        "badge_id": f"{prefix.upper()}-A{artist_idx + 1:04d}",
                        "name": f"Artist {show_idx:03d}-{artist_idx + 1:04d}",
                        "discipline": discipline,
                        "seniority": SENIORITIES[artist_idx % len(SENIORITIES)],
                        "timezone": TIMEZONES[artist_idx % len(TIMEZONES)],
                    },
                },
            )
            node_count += 1

            write_jsonl_line(
                edges_out,
                {
                    "source": dept_id,
                    "label": "staffed_by",
                    "target": artist_id,
                    "properties": {},
                },
            )
            edge_count += 1

            tool_a = tool_ids[artist_idx % len(tool_ids)]
            tool_b = tool_ids[(artist_idx * 7 + 3) % len(tool_ids)]
            write_jsonl_line(
                edges_out,
                {"source": artist_id, "label": "uses_tool", "target": tool_a, "properties": {}},
            )
            write_jsonl_line(
                edges_out,
                {"source": artist_id, "label": "uses_tool", "target": tool_b, "properties": {}},
            )
            edge_count += 2

        show_id = f"{prefix}_show_01"
        write_jsonl_line(
            nodes_out,
            {
                "id": show_id,
                "entity_type": "Show",
                "properties": {
                    "show_code": f"{prefix.upper()}-SHOW",
                    "title": f"VFX Show {show_idx:03d}",
                    "phase": ["prep", "shoot", "post", "delivery"][show_idx % 4],
                    "priority": 1 + (show_idx % 5),
                },
            },
        )
        node_count += 1

        write_jsonl_line(
            edges_out,
            {
                "source": studio_ids[0],
                "label": "owns_show",
                "target": show_id,
                "properties": {},
            },
        )
        edge_count += 1

        for asset_idx in range(ASSETS_PER_SHOW):
            asset_id = f"{prefix}_asset_{asset_idx + 1:04d}"
            asset_ids.append(asset_id)
            write_jsonl_line(
                nodes_out,
                {
                    "id": asset_id,
                    "entity_type": "Asset",
                    "properties": {
                        "asset_code": f"{prefix.upper()}-AS{asset_idx + 1:04d}",
                        "name": f"Asset {show_idx:03d}-{asset_idx + 1:04d}",
                        "kind": ASSET_KINDS[asset_idx % len(ASSET_KINDS)],
                        "version": 1 + (asset_idx % 11),
                    },
                },
            )
            node_count += 1

            write_jsonl_line(
                edges_out,
                {"source": show_id, "label": "has_asset", "target": asset_id, "properties": {}},
            )
            edge_count += 1

            creator_id = artist_ids[(asset_idx * 11 + show_idx) % len(artist_ids)]
            write_jsonl_line(
                edges_out,
                {"source": asset_id, "label": "created_by", "target": creator_id, "properties": {}},
            )
            edge_count += 1

        for seq_idx in range(SEQUENCES_PER_SHOW):
            sequence_id = f"{prefix}_seq_{seq_idx + 1:03d}"
            write_jsonl_line(
                nodes_out,
                {
                    "id": sequence_id,
                    "entity_type": "Sequence",
                    "properties": {
                        "sequence_code": f"{prefix.upper()}-SQ{seq_idx + 1:03d}",
                        "name": f"Sequence {seq_idx + 1:03d}",
                        "order": seq_idx + 1,
                    },
                },
            )
            node_count += 1

            write_jsonl_line(
                edges_out,
                {
                    "source": show_id,
                    "label": "has_sequence",
                    "target": sequence_id,
                    "properties": {},
                },
            )
            edge_count += 1

            prev_shot_id: str | None = None
            prev_shot_last_task_id: str | None = None

            for shot_idx in range(SHOTS_PER_SEQUENCE):
                shot_num = shot_idx + 1
                shot_id = f"{prefix}_shot_{seq_idx + 1:03d}_{shot_num:03d}"
                write_jsonl_line(
                    nodes_out,
                    {
                        "id": shot_id,
                        "entity_type": "Shot",
                        "properties": {
                            "shot_code": f"{prefix.upper()}-SQ{seq_idx + 1:03d}-SH{shot_num:03d}",
                            "name": f"SQ{seq_idx + 1:03d}_SH{shot_num:03d}",
                            "order": shot_num,
                            "cut_duration_frames": 70 + ((seq_idx + shot_idx + show_idx) % 120),
                        },
                    },
                )
                node_count += 1

                write_jsonl_line(
                    edges_out,
                    {"source": sequence_id, "label": "has_shot", "target": shot_id, "properties": {}},
                )
                edge_count += 1

                if prev_shot_id is not None:
                    write_jsonl_line(
                        edges_out,
                        {"source": prev_shot_id, "label": "next_shot", "target": shot_id, "properties": {}},
                    )
                    edge_count += 1
                prev_shot_id = shot_id

                task_ids: list[str] = []
                for task_idx in range(TASKS_PER_SHOT):
                    task_num = task_idx + 1
                    task_id = f"{prefix}_task_{seq_idx + 1:03d}_{shot_num:03d}_{task_num:02d}"
                    stage = TASK_STAGES[task_idx % len(TASK_STAGES)]
                    write_jsonl_line(
                        nodes_out,
                        {
                            "id": task_id,
                            "entity_type": "Task",
                            "properties": {
                                "task_code": f"{prefix.upper()}-TSK-{seq_idx + 1:03d}-{shot_num:03d}-{task_num:02d}",
                                "name": f"{stage.title()} Task",
                                "stage": stage,
                                "status": TASK_STATUS[(seq_idx + shot_idx + task_idx) % len(TASK_STATUS)],
                                "priority": 1 + ((show_idx + task_idx + shot_idx) % 5),
                            },
                        },
                    )
                    node_count += 1

                    write_jsonl_line(
                        edges_out,
                        {"source": shot_id, "label": "has_task", "target": task_id, "properties": {}},
                    )
                    edge_count += 1

                    assignee = artist_ids[(seq_idx * 101 + shot_idx * 31 + task_idx) % len(artist_ids)]
                    task_tool = tool_ids[(seq_idx * 13 + shot_idx * 7 + task_idx) % len(tool_ids)]
                    write_jsonl_line(
                        edges_out,
                        {"source": task_id, "label": "assigned_to", "target": assignee, "properties": {}},
                    )
                    write_jsonl_line(
                        edges_out,
                        {"source": task_id, "label": "uses_tool", "target": task_tool, "properties": {}},
                    )
                    edge_count += 2

                    if task_idx > 0:
                        write_jsonl_line(
                            edges_out,
                            {
                                "source": task_id,
                                "label": "depends_on",
                                "target": task_ids[-1],
                                "properties": {"kind": "intra_shot"},
                            },
                        )
                        edge_count += 1
                    elif prev_shot_last_task_id is not None:
                        write_jsonl_line(
                            edges_out,
                            {
                                "source": task_id,
                                "label": "depends_on",
                                "target": prev_shot_last_task_id,
                                "properties": {"kind": "carry_over"},
                            },
                        )
                        edge_count += 1

                    task_ids.append(task_id)

                prev_shot_last_task_id = task_ids[-1]

                for reviewable_idx in range(REVIEWABLES_PER_SHOT):
                    reviewable_num = reviewable_idx + 1
                    reviewable_id = f"{prefix}_revb_{seq_idx + 1:03d}_{shot_num:03d}_{reviewable_num:02d}"
                    write_jsonl_line(
                        nodes_out,
                        {
                            "id": reviewable_id,
                            "entity_type": "Reviewable",
                            "properties": {
                                "reviewable_code": f"{prefix.upper()}-RVB-{seq_idx + 1:03d}-{shot_num:03d}-{reviewable_num:02d}",
                                "kind": REVIEWABLE_KINDS[(seq_idx + shot_idx + reviewable_idx) % len(REVIEWABLE_KINDS)],
                                "frame_count": 24 + ((seq_idx + shot_idx) % 240),
                            },
                        },
                    )
                    node_count += 1

                    write_jsonl_line(
                        edges_out,
                        {"source": shot_id, "label": "has_reviewable", "target": reviewable_id, "properties": {}},
                    )
                    write_jsonl_line(
                        edges_out,
                        {
                            "source": task_ids[-1],
                            "label": "produces_reviewable",
                            "target": reviewable_id,
                            "properties": {},
                        },
                    )
                    edge_count += 2

                    for asset_hop in range(4):
                        asset_id = asset_ids[
                            (seq_idx * 173 + shot_idx * 29 + reviewable_idx * 11 + asset_hop * 7)
                            % len(asset_ids)
                        ]
                        write_jsonl_line(
                            edges_out,
                            {
                                "source": reviewable_id,
                                "label": "uses_asset",
                                "target": asset_id,
                                "properties": {},
                            },
                        )
                        edge_count += 1

                    for review_idx in range(REVIEWS_PER_REVIEWABLE):
                        review_num = review_idx + 1
                        review_id = f"{prefix}_review_{seq_idx + 1:03d}_{shot_num:03d}_{review_num:02d}"
                        write_jsonl_line(
                            nodes_out,
                            {
                                "id": review_id,
                                "entity_type": "Review",
                                "properties": {
                                    "review_code": f"{prefix.upper()}-RV-{seq_idx + 1:03d}-{shot_num:03d}-{review_num:02d}",
                                    "status": REVIEW_STATUS[(show_idx + seq_idx + shot_idx + review_idx) % len(REVIEW_STATUS)],
                                    "score": 60 + ((seq_idx * 9 + shot_idx * 5 + review_idx * 7) % 41),
                                },
                            },
                        )
                        node_count += 1

                        write_jsonl_line(
                            edges_out,
                            {"source": reviewable_id, "label": "has_review", "target": review_id, "properties": {}},
                        )
                        write_jsonl_line(
                            edges_out,
                            {"source": review_id, "label": "targets_task", "target": task_ids[-1], "properties": {}},
                        )
                        edge_count += 2

                        for note_idx in range(NOTES_PER_REVIEW):
                            note_num = note_idx + 1
                            note_id = f"{prefix}_note_{seq_idx + 1:03d}_{shot_num:03d}_{review_num:02d}_{note_num:02d}"
                            write_jsonl_line(
                                nodes_out,
                                {
                                    "id": note_id,
                                    "entity_type": "Note",
                                    "properties": {
                                        "note_code": f"{prefix.upper()}-NT-{seq_idx + 1:03d}-{shot_num:03d}-{review_num:02d}-{note_num:02d}",
                                        "severity": NOTE_SEVERITY[
                                            (seq_idx + shot_idx + note_idx + show_idx) % len(NOTE_SEVERITY)
                                        ],
                                        "line": 1 + ((seq_idx * 13 + shot_idx * 3 + note_idx) % 200),
                                        "message": (
                                            f"Iteration feedback for SQ{seq_idx + 1:03d} "
                                            f"SH{shot_num:03d} note {note_num:02d}"
                                        ),
                                    },
                                },
                            )
                            node_count += 1

                            writer = artist_ids[
                                (seq_idx * 97 + shot_idx * 17 + review_idx * 11 + note_idx * 3 + show_idx)
                                % len(artist_ids)
                            ]
                            write_jsonl_line(
                                edges_out,
                                {"source": review_id, "label": "has_note", "target": note_id, "properties": {}},
                            )
                            write_jsonl_line(
                                edges_out,
                                {"source": note_id, "label": "authored_by", "target": writer, "properties": {}},
                            )
                            edge_count += 2

    return NamespaceSummary(
        namespace=ns,
        dataset=f"vfx_pipeline_50k/show_{show_idx:03d}",
        nodes=node_count,
        edges=edge_count,
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Generate deterministic multi-namespace VFX pipeline dataset")
    parser.add_argument(
        "--output-dir",
        default="vfx_pipeline_50k",
        help="Directory name under datasets/ for generated dataset family",
    )
    parser.add_argument(
        "--clean",
        action="store_true",
        help="Remove existing output directory before generating",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    dataset_dir = ROOT / args.output_dir

    if args.clean and dataset_dir.exists():
        shutil.rmtree(dataset_dir)

    dataset_dir.mkdir(parents=True, exist_ok=True)

    summaries: list[NamespaceSummary] = []
    total_nodes = 0
    total_edges = 0

    for show_idx in range(1, SHOW_COUNT + 1):
        artist_count = ARTISTS_PER_SHOW_BASE + (FIRST_SHOW_ARTIST_BONUS if show_idx == 1 else 0)
        summary = generate_namespace(dataset_dir, show_idx, artist_count)
        summaries.append(summary)
        total_nodes += summary.nodes
        total_edges += summary.edges

    manifest = {
        "name": "vfx_pipeline_50k",
        "description": "Deterministic VFX production graph split across 12 show namespaces.",
        "entity_types": [
            "Studio",
            "Artist",
            "Department",
            "Tool",
            "Show",
            "Sequence",
            "Shot",
            "Task",
            "Asset",
            "Reviewable",
            "Review",
            "Note",
        ],
        "counts": {
            "shows": SHOW_COUNT,
            "sequences_per_show": SEQUENCES_PER_SHOW,
            "shots_per_sequence": SHOTS_PER_SEQUENCE,
            "tasks_per_shot": TASKS_PER_SHOT,
            "assets_per_show": ASSETS_PER_SHOW,
            "artists_per_show_base": ARTISTS_PER_SHOW_BASE,
            "first_show_artist_bonus": FIRST_SHOW_ARTIST_BONUS,
        },
        "load": [
            {
                "dataset": summary.dataset,
                "namespace": summary.namespace,
                "nodes": summary.nodes,
                "edges": summary.edges,
            }
            for summary in summaries
        ],
        "totals": {"nodes": total_nodes, "edges": total_edges},
    }
    (dataset_dir / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    (dataset_dir / "README.md").write_text(
        (
            "# VFX Pipeline 50K Dataset\n\n"
            "Generated by `datasets/generate_vfx_pipeline.py`.\n\n"
            "## Summary\n"
            f"- namespaces: {SHOW_COUNT}\n"
            f"- total nodes: {total_nodes}\n"
            f"- total edges: {total_edges}\n"
            "- entity types: Studio, Artist, Department, Tool, Show, Sequence, Shot, Task, Asset, Reviewable, Review, Note\n\n"
            "## Load Example\n"
            "```bash\n"
            "python datasets/load_dataset.py vfx_pipeline_50k/show_001 \\\n"
            "  --endpoint 127.0.0.1:27615 \\\n"
            "  --database default \\\n"
            "  --namespace vfx.show.001\n"
            "```\n"
        ),
        encoding="utf-8",
    )

    print(f"Generated dataset family at {dataset_dir}")
    print(f"Namespaces: {len(summaries)}")
    print(f"Total nodes: {total_nodes}")
    print(f"Total edges: {total_edges}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
