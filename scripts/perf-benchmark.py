#!/usr/bin/env python3
"""Lightweight CLI-driven latency benchmark harness for PelagoDB.

This script is meant for repeatable, presentation-grade performance checks.
It executes representative read/query flows via the `pelago` CLI and reports
p50/p95/p99 latencies.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import statistics
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List


@dataclass
class BenchResult:
    name: str
    samples_ms: List[float]

    @property
    def count(self) -> int:
        return len(self.samples_ms)

    @property
    def mean_ms(self) -> float:
        return statistics.fmean(self.samples_ms)

    @property
    def p50_ms(self) -> float:
        return percentile(self.samples_ms, 50.0)

    @property
    def p95_ms(self) -> float:
        return percentile(self.samples_ms, 95.0)

    @property
    def p99_ms(self) -> float:
        return percentile(self.samples_ms, 99.0)


def percentile(values: List[float], p: float) -> float:
    if not values:
        return 0.0
    sorted_vals = sorted(values)
    if len(sorted_vals) == 1:
        return sorted_vals[0]
    idx = (len(sorted_vals) - 1) * (p / 100.0)
    lo = math.floor(idx)
    hi = math.ceil(idx)
    if lo == hi:
        return sorted_vals[lo]
    frac = idx - lo
    return sorted_vals[lo] + (sorted_vals[hi] - sorted_vals[lo]) * frac


def run_timed(cmd: List[str]) -> float:
    start_ns = time.perf_counter_ns()
    subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    elapsed_ns = time.perf_counter_ns() - start_ns
    return elapsed_ns / 1_000_000.0


def run_capture(cmd: List[str]) -> str:
    completed = subprocess.run(
        cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, text=True
    )
    return completed.stdout


def run_case(name: str, cmd: List[str], warmup: int, runs: int) -> BenchResult:
    for _ in range(warmup):
        run_timed(cmd)

    samples: List[float] = []
    for _ in range(runs):
        samples.append(run_timed(cmd))
    return BenchResult(name=name, samples_ms=samples)


def maybe_build_cli(bin_path: str, force_build: bool) -> None:
    if shutil.which(bin_path):
        return

    path_obj = Path(bin_path)
    if path_obj.exists():
        return

    if not force_build:
        raise FileNotFoundError(
            f"pelago binary not found at '{bin_path}'. Build it first or pass --build-cli."
        )

    subprocess.run(["cargo", "build", "-p", "pelago-cli", "--release"], check=True)
    if not path_obj.exists():
        raise FileNotFoundError(f"Expected CLI binary at '{bin_path}' after build")


def base_cmd(args: argparse.Namespace) -> List[str]:
    return [
        args.pelago_bin,
        "--server",
        args.server,
        "--database",
        args.database,
        "--namespace",
        args.namespace,
    ]


def print_summary(results: List[BenchResult], targets: Dict[str, float] | None, enforce: bool) -> int:
    print("\nBenchmark Summary (ms)")
    print("name,count,mean,p50,p95,p99")

    failed = False
    for result in results:
        print(
            f"{result.name},{result.count},{result.mean_ms:.3f},{result.p50_ms:.3f},{result.p95_ms:.3f},{result.p99_ms:.3f}"
        )

        if targets and result.name in targets and result.p99_ms > targets[result.name]:
            failed = True
            print(
                f"target-fail: {result.name} p99={result.p99_ms:.3f}ms > target={targets[result.name]:.3f}ms"
            )

    if failed and enforce:
        return 2
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run PelagoDB CLI latency benchmark checks")
    parser.add_argument("--pelago-bin", default="target/release/pelago")
    parser.add_argument("--build-cli", action="store_true", help="Build pelago CLI if binary is missing")

    parser.add_argument("--server", default=os.getenv("PELAGO_SERVER", "http://127.0.0.1:27615"))
    parser.add_argument("--database", default=os.getenv("PELAGO_DATABASE", "default"))
    parser.add_argument("--namespace", default=os.getenv("PELAGO_NAMESPACE", "default"))
    parser.add_argument(
        "--profile",
        choices=["default", "context"],
        default="default",
        help="Benchmark profile: generic defaults or context-centric query shapes",
    )

    parser.add_argument("--entity-type", default="Person")
    parser.add_argument("--seed-node-id", default="1_0")
    parser.add_argument("--edge-label", default="follows")
    parser.add_argument("--filter", default="age >= 30")
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("--context-show", default="show_001")
    parser.add_argument("--context-scheme", default="main")
    parser.add_argument("--context-shot", default="shot_0001")
    parser.add_argument("--context-shot-alt", default="shot_0002")
    parser.add_argument("--context-sequence", default="seq_001")
    parser.add_argument("--context-task", default="fx")
    parser.add_argument("--context-label", default="default")
    parser.add_argument("--context-page-size", type=int, default=500)
    parser.add_argument(
        "--context-deep-pages",
        type=int,
        default=20,
        help="Number of pages to walk before deep-page benchmark (context profile)",
    )
    parser.add_argument(
        "--require-deep-page",
        action="store_true",
        help="Fail when deep-page cursor walk cannot reach requested depth",
    )
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--runs", type=int, default=30)
    parser.add_argument(
        "--skip-traverse",
        action="store_true",
        help="Skip traversal benchmark case (useful if edge schema is not populated)",
    )

    parser.add_argument(
        "--target-get-ms",
        type=float,
        default=1.0,
        help="P99 target for node get (phase target: <1ms)",
    )
    parser.add_argument(
        "--target-find-ms",
        type=float,
        default=10.0,
        help="P99 target for indexed find (phase target: <10ms)",
    )
    parser.add_argument(
        "--target-traverse-ms",
        type=float,
        default=100.0,
        help="P99 target for traversal (phase target: <100ms)",
    )
    parser.add_argument(
        "--target-context-show-shot-ms",
        type=float,
        default=50.0,
        help="P99 target for Context show+shot lookups",
    )
    parser.add_argument(
        "--target-context-template-ms",
        type=float,
        default=50.0,
        help="P99 target for Context show+shot+task+label lookups",
    )
    parser.add_argument(
        "--target-context-or-ms",
        type=float,
        default=80.0,
        help="P99 target for Context OR query lookups",
    )
    parser.add_argument(
        "--target-context-deep-page-ms",
        type=float,
        default=80.0,
        help="P99 target for Context deep keyset page lookups",
    )
    parser.add_argument(
        "--enforce-targets",
        action="store_true",
        help="Exit non-zero if any P99 target is missed",
    )
    parser.add_argument("--output-json", default="", help="Optional file path to write JSON summary")
    return parser.parse_args()


def context_filter_show_shot(args: argparse.Namespace) -> str:
    return (
        f"show == '{args.context_show}'"
        f" && scheme == '{args.context_scheme}'"
        f" && shot == '{args.context_shot}'"
        f" && sequence == '{args.context_sequence}'"
    )


def context_filter_template(args: argparse.Namespace) -> str:
    return (
        f"{context_filter_show_shot(args)}"
        f" && task == '{args.context_task}'"
        f" && label == '{args.context_label}'"
    )


def context_filter_show_scheme(args: argparse.Namespace) -> str:
    return f"show == '{args.context_show}' && scheme == '{args.context_scheme}'"


def context_filter_or_shots(args: argparse.Namespace) -> str:
    left = (
        f"show == '{args.context_show}'"
        f" && scheme == '{args.context_scheme}'"
        f" && shot == '{args.context_shot}'"
        f" && sequence == '{args.context_sequence}'"
    )
    right = (
        f"show == '{args.context_show}'"
        f" && scheme == '{args.context_scheme}'"
        f" && shot == '{args.context_shot_alt}'"
        f" && sequence == '{args.context_sequence}'"
    )
    return f"{left} || {right}"


def next_cursor_hex_for_find(
    base: List[str], entity_type: str, cel_filter: str, limit: int, cursor_hex: str
) -> str:
    cmd = (
        base
        + [
            "query",
            "find",
            entity_type,
            "--filter",
            cel_filter,
            "--limit",
            str(limit),
            "--cursor-hex",
            cursor_hex,
            "--next-cursor-only",
        ]
    )
    return run_capture(cmd).strip()


def compute_deep_cursor_hex(
    base: List[str], entity_type: str, cel_filter: str, page_size: int, pages: int
) -> str:
    cursor_hex = ""
    seen: set[str] = set()
    for _ in range(max(pages, 0)):
        next_hex = next_cursor_hex_for_find(base, entity_type, cel_filter, page_size, cursor_hex)
        if not next_hex:
            return ""
        if next_hex in seen:
            # Cursor loop indicates unstable keyset progression.
            return ""
        seen.add(next_hex)
        cursor_hex = next_hex
    return cursor_hex


def main() -> int:
    args = parse_args()
    maybe_build_cli(args.pelago_bin, args.build_cli)

    base = base_cmd(args)
    cases: List[tuple[str, List[str]]] = []
    targets: Dict[str, float] = {}

    if args.profile == "default":
        cases = [
            (
                "node_get",
                base
                + [
                    "node",
                    "get",
                    args.entity_type,
                    args.seed_node_id,
                    "--format",
                    "json",
                ],
            ),
            (
                "query_find",
                base
                + [
                    "query",
                    "find",
                    args.entity_type,
                    "--filter",
                    args.filter,
                    "--limit",
                    str(args.limit),
                    "--format",
                    "json",
                ],
            ),
        ]
        targets = {
            "node_get": args.target_get_ms,
            "query_find": args.target_find_ms,
        }

        if not args.skip_traverse:
            cases.append(
                (
                    "query_traverse",
                    base
                    + [
                        "query",
                        "traverse",
                        f"{args.entity_type}:{args.seed_node_id}",
                        args.edge_label,
                        "--max-depth",
                        "2",
                        "--max-results",
                        str(args.limit),
                        "--format",
                        "json",
                    ],
                )
            )
            targets["query_traverse"] = args.target_traverse_ms
    else:
        show_shot_filter = context_filter_show_shot(args)
        template_filter = context_filter_template(args)
        or_filter = context_filter_or_shots(args)
        show_scheme_filter = context_filter_show_scheme(args)
        page_size = max(args.context_page_size, 1)

        cases = [
            (
                "context_point_lookup",
                base
                + [
                    "node",
                    "get",
                    args.entity_type,
                    args.seed_node_id,
                    "--format",
                    "json",
                ],
            ),
            (
                "context_show_shot",
                base
                + [
                    "query",
                    "find",
                    args.entity_type,
                    "--filter",
                    show_shot_filter,
                    "--limit",
                    str(page_size),
                    "--format",
                    "json",
                ],
            ),
            (
                "context_show_shot_task_label",
                base
                + [
                    "query",
                    "find",
                    args.entity_type,
                    "--filter",
                    template_filter,
                    "--limit",
                    str(page_size),
                    "--format",
                    "json",
                ],
            ),
            (
                "context_or_shots",
                base
                + [
                    "query",
                    "find",
                    args.entity_type,
                    "--filter",
                    or_filter,
                    "--limit",
                    str(page_size),
                    "--format",
                    "json",
                ],
            ),
        ]
        targets = {
            "context_point_lookup": args.target_get_ms,
            "context_show_shot": args.target_context_show_shot_ms,
            "context_show_shot_task_label": args.target_context_template_ms,
            "context_or_shots": args.target_context_or_ms,
        }

        if args.context_deep_pages > 0:
            deep_cursor_hex = compute_deep_cursor_hex(
                base, args.entity_type, show_scheme_filter, page_size, args.context_deep_pages
            )
            if deep_cursor_hex:
                cases.append(
                    (
                        "context_deep_page",
                        base
                        + [
                            "query",
                            "find",
                            args.entity_type,
                            "--filter",
                            show_scheme_filter,
                            "--limit",
                            str(page_size),
                            "--cursor-hex",
                            deep_cursor_hex,
                            "--format",
                            "json",
                        ],
                    )
                )
                targets["context_deep_page"] = args.target_context_deep_page_ms
            else:
                msg = (
                    "unable to walk requested deep pages "
                    "(insufficient result depth or unstable keyset cursor progression)"
                )
                if args.require_deep_page:
                    print(f"context_deep_page gate failed: {msg}")
                    return 2
                print(f"Skipping context_deep_page: {msg}.")

    results: List[BenchResult] = []
    for name, cmd in cases:
        print(f"Running {name} ({args.runs} samples, warmup {args.warmup})...")
        results.append(run_case(name, cmd, args.warmup, args.runs))

    exit_code = print_summary(results, targets, args.enforce_targets)

    if args.output_json:
        payload = {
            "server": args.server,
            "database": args.database,
            "namespace": args.namespace,
            "profile": args.profile,
            "runs": args.runs,
            "warmup": args.warmup,
            "results": [
                {
                    "name": r.name,
                    "count": r.count,
                    "mean_ms": r.mean_ms,
                    "p50_ms": r.p50_ms,
                    "p95_ms": r.p95_ms,
                    "p99_ms": r.p99_ms,
                }
                for r in results
            ],
            "targets_ms": targets,
            "enforced": args.enforce_targets,
            "exit_code": exit_code,
        }
        out = Path(args.output_json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(payload, indent=2), encoding="utf-8")
        print(f"Wrote benchmark JSON: {out}")

    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
