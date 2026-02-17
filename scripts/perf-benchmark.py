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

    parser.add_argument("--entity-type", default="Person")
    parser.add_argument("--seed-node-id", default="1_0")
    parser.add_argument("--edge-label", default="follows")
    parser.add_argument("--filter", default="age >= 30")
    parser.add_argument("--limit", type=int, default=100)
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
        "--enforce-targets",
        action="store_true",
        help="Exit non-zero if any P99 target is missed",
    )
    parser.add_argument("--output-json", default="", help="Optional file path to write JSON summary")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    maybe_build_cli(args.pelago_bin, args.build_cli)

    base = base_cmd(args)

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

    results: List[BenchResult] = []
    for name, cmd in cases:
        print(f"Running {name} ({args.runs} samples, warmup {args.warmup})...")
        results.append(run_case(name, cmd, args.warmup, args.runs))

    targets = {
        "node_get": args.target_get_ms,
        "query_find": args.target_find_ms,
        "query_traverse": args.target_traverse_ms,
    }

    exit_code = print_summary(results, targets, args.enforce_targets)

    if args.output_json:
        payload = {
            "server": args.server,
            "database": args.database,
            "namespace": args.namespace,
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
