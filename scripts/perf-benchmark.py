#!/usr/bin/env python3
"""Lightweight latency benchmark harness for PelagoDB.

This script is meant for repeatable, presentation-grade performance checks.
It executes representative read/query flows and reports p50/p95/p99 latencies.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import statistics
import subprocess
import sys
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional
from urllib.parse import urlparse


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


@dataclass(frozen=True)
class ScenarioGate:
    scenario: str
    max_p95_ms: float
    max_p99_ms: float
    min_cache_hit_ratio: float
    max_projector_lag_events: float
    max_fallback_rate: float


SCENARIO_GATES: Dict[str, ScenarioGate] = {
    "S1": ScenarioGate(
        scenario="S1",
        max_p95_ms=40.0,
        max_p99_ms=100.0,
        min_cache_hit_ratio=0.90,
        max_projector_lag_events=2_000.0,
        max_fallback_rate=0.10,
    ),
    "S2": ScenarioGate(
        scenario="S2",
        max_p95_ms=50.0,
        max_p99_ms=120.0,
        min_cache_hit_ratio=0.92,
        max_projector_lag_events=2_500.0,
        max_fallback_rate=0.08,
    ),
    "S3": ScenarioGate(
        scenario="S3",
        max_p95_ms=60.0,
        max_p99_ms=140.0,
        min_cache_hit_ratio=0.90,
        max_projector_lag_events=2_500.0,
        max_fallback_rate=0.12,
    ),
    "S4": ScenarioGate(
        scenario="S4",
        max_p95_ms=70.0,
        max_p99_ms=160.0,
        min_cache_hit_ratio=0.85,
        max_projector_lag_events=5_000.0,
        max_fallback_rate=0.15,
    ),
    "S5": ScenarioGate(
        scenario="S5",
        max_p95_ms=90.0,
        max_p99_ms=200.0,
        min_cache_hit_ratio=0.80,
        max_projector_lag_events=8_000.0,
        max_fallback_rate=0.20,
    ),
    "S6": ScenarioGate(
        scenario="S6",
        max_p95_ms=110.0,
        max_p99_ms=250.0,
        min_cache_hit_ratio=0.75,
        max_projector_lag_events=12_000.0,
        max_fallback_rate=0.25,
    ),
}


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


def run_timed_command(cmd: List[str]) -> float:
    start_ns = time.perf_counter_ns()
    subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    elapsed_ns = time.perf_counter_ns() - start_ns
    return elapsed_ns / 1_000_000.0


def run_timed_callable(fn: Callable[[], None]) -> float:
    start_ns = time.perf_counter_ns()
    fn()
    elapsed_ns = time.perf_counter_ns() - start_ns
    return elapsed_ns / 1_000_000.0


def run_capture(cmd: List[str]) -> str:
    completed = subprocess.run(
        cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, text=True
    )
    return completed.stdout


def run_case_command(name: str, cmd: List[str], warmup: int, runs: int) -> BenchResult:
    for _ in range(warmup):
        run_timed_command(cmd)

    samples: List[float] = []
    for _ in range(runs):
        samples.append(run_timed_command(cmd))
    return BenchResult(name=name, samples_ms=samples)


def run_case_callable(
    name: str, fn: Callable[[], None], warmup: int, runs: int
) -> BenchResult:
    for _ in range(warmup):
        run_timed_callable(fn)

    samples: List[float] = []
    for _ in range(runs):
        samples.append(run_timed_callable(fn))
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


def load_observability(path: str) -> Dict[str, float]:
    if not path:
        return {}
    payload = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise ValueError(f"observability payload must be an object: {path}")

    metrics: Dict[str, float] = {}
    for key in ("cache_hit_ratio", "projector_lag_events", "fallback_rate"):
        raw = payload.get(key)
        if raw is None:
            continue
        try:
            metrics[key] = float(raw)
        except (TypeError, ValueError) as exc:
            raise ValueError(f"observability field '{key}' must be numeric") from exc
    return metrics


def evaluate_scenario_gates(
    scenario: str,
    results: List[BenchResult],
    observability: Dict[str, float],
) -> tuple[bool, List[str], Dict[str, float]]:
    if scenario not in SCENARIO_GATES:
        raise ValueError(f"unknown scenario gate '{scenario}'")

    gate = SCENARIO_GATES[scenario]
    failures: List[str] = []

    peak_p95 = max((r.p95_ms for r in results), default=0.0)
    peak_p99 = max((r.p99_ms for r in results), default=0.0)
    if peak_p95 > gate.max_p95_ms:
        failures.append(
            f"{scenario} latency gate failed: peak p95 {peak_p95:.3f}ms > {gate.max_p95_ms:.3f}ms"
        )
    if peak_p99 > gate.max_p99_ms:
        failures.append(
            f"{scenario} latency gate failed: peak p99 {peak_p99:.3f}ms > {gate.max_p99_ms:.3f}ms"
        )

    cache_hit_ratio = observability.get("cache_hit_ratio")
    if cache_hit_ratio is None:
        failures.append(f"{scenario} observability gate failed: missing cache_hit_ratio")
    elif cache_hit_ratio < gate.min_cache_hit_ratio:
        failures.append(
            f"{scenario} cache gate failed: hit ratio {cache_hit_ratio:.4f} < {gate.min_cache_hit_ratio:.4f}"
        )

    projector_lag = observability.get("projector_lag_events")
    if projector_lag is None:
        failures.append(f"{scenario} observability gate failed: missing projector_lag_events")
    elif projector_lag > gate.max_projector_lag_events:
        failures.append(
            f"{scenario} projector gate failed: lag {projector_lag:.1f} > {gate.max_projector_lag_events:.1f}"
        )

    fallback_rate = observability.get("fallback_rate")
    if fallback_rate is None:
        failures.append(f"{scenario} observability gate failed: missing fallback_rate")
    elif fallback_rate > gate.max_fallback_rate:
        failures.append(
            f"{scenario} fallback gate failed: rate {fallback_rate:.4f} > {gate.max_fallback_rate:.4f}"
        )

    summary = {
        "scenario": gate.scenario,
        "peak_p95_ms": peak_p95,
        "peak_p99_ms": peak_p99,
        "cache_hit_ratio": cache_hit_ratio if cache_hit_ratio is not None else float("nan"),
        "projector_lag_events": projector_lag if projector_lag is not None else float("nan"),
        "fallback_rate": fallback_rate if fallback_rate is not None else float("nan"),
        "max_p95_ms": gate.max_p95_ms,
        "max_p99_ms": gate.max_p99_ms,
        "min_cache_hit_ratio": gate.min_cache_hit_ratio,
        "max_projector_lag_events": gate.max_projector_lag_events,
        "max_fallback_rate": gate.max_fallback_rate,
    }

    return (len(failures) == 0, failures, summary)


def print_scenario_gate_summary(summary: Dict[str, float], passed: bool) -> None:
    print("\nScenario Gate Summary")
    print(
        "scenario={scenario} status={status} peak_p95={peak_p95:.3f}ms/{max_p95:.3f}ms "
        "peak_p99={peak_p99:.3f}ms/{max_p99:.3f}ms "
        "cache_hit_ratio={cache_hit:.4f}/{min_hit:.4f} "
        "projector_lag_events={lag:.1f}/{max_lag:.1f} "
        "fallback_rate={fallback:.4f}/{max_fallback:.4f}".format(
            scenario=summary["scenario"],
            status="PASS" if passed else "FAIL",
            peak_p95=summary["peak_p95_ms"],
            max_p95=summary["max_p95_ms"],
            peak_p99=summary["peak_p99_ms"],
            max_p99=summary["max_p99_ms"],
            cache_hit=summary["cache_hit_ratio"],
            min_hit=summary["min_cache_hit_ratio"],
            lag=summary["projector_lag_events"],
            max_lag=summary["max_projector_lag_events"],
            fallback=summary["fallback_rate"],
            max_fallback=summary["max_fallback_rate"],
        )
    )


def parse_args(argv: Optional[List[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run PelagoDB latency benchmark checks")
    parser.add_argument("--pelago-bin", default="target/release/pelago")
    parser.add_argument("--build-cli", action="store_true", help="Build pelago CLI if binary is missing")
    parser.add_argument(
        "--transport",
        choices=["cli", "grpc"],
        default="cli",
        help="Benchmark transport: CLI subprocesses or persistent gRPC channel",
    )

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
    parser.add_argument(
        "--scenario",
        choices=sorted(SCENARIO_GATES.keys()),
        default="",
        help="Optional S1-S6 scenario gate profile for latency/cache/projector thresholds",
    )
    parser.add_argument(
        "--observability-json",
        default="",
        help=(
            "JSON file with cache/projector metrics for scenario gates. "
            "Expected fields: cache_hit_ratio, projector_lag_events, fallback_rate."
        ),
    )
    parser.add_argument(
        "--enforce-scenario-gates",
        action="store_true",
        help="Exit non-zero if selected S1-S6 scenario gate thresholds are missed",
    )
    parser.add_argument("--output-json", default="", help="Optional file path to write JSON summary")
    return parser.parse_args(argv)


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


def grpc_endpoint_from_server(server: str) -> str:
    if "://" not in server:
        return server

    parsed = urlparse(server)
    if parsed.scheme not in ("http", "https"):
        raise ValueError(f"unsupported server scheme '{parsed.scheme}'")
    if parsed.scheme == "https":
        raise ValueError("grpc transport currently supports only insecure http endpoints")
    if not parsed.hostname:
        raise ValueError(f"invalid server address '{server}'")
    return f"{parsed.hostname}:{parsed.port or 80}"


def load_grpc_dependencies() -> tuple[Any, Any, Any]:
    repo_root = Path(__file__).resolve().parents[1]
    clients_python = repo_root / "clients" / "python"
    sys_path = str(clients_python)
    if sys_path not in sys.path:
        sys.path.insert(0, sys_path)

    try:
        import grpc  # type: ignore[import-not-found]
    except ImportError as exc:
        raise RuntimeError(
            "grpc transport requires python package 'grpcio'. "
            "Install project dependencies first (see clients/python/pyproject.toml)."
        ) from exc

    try:
        from pelagodb.generated import pelago_pb2  # type: ignore[import-not-found]
        from pelagodb.generated import pelago_pb2_grpc  # type: ignore[import-not-found]
    except ImportError as exc:
        raise RuntimeError(
            "failed to import pelagodb generated Python stubs from clients/python."
        ) from exc

    return grpc, pelago_pb2, pelago_pb2_grpc


class GrpcBenchmarkClient:
    def __init__(self, args: argparse.Namespace):
        grpc, pelago_pb2, pelago_pb2_grpc = load_grpc_dependencies()
        endpoint = grpc_endpoint_from_server(args.server)
        self._args = args
        self._pb2 = pelago_pb2
        self._channel = grpc.insecure_channel(endpoint)
        self._node_client = pelago_pb2_grpc.NodeServiceStub(self._channel)
        self._query_client = pelago_pb2_grpc.QueryServiceStub(self._channel)

    def close(self) -> None:
        self._channel.close()

    def _context(self) -> Any:
        return self._pb2.RequestContext(
            database=self._args.database,
            namespace=self._args.namespace,
            site_id="",
            request_id=str(uuid.uuid4()),
        )

    def node_get(self, entity_type: str, node_id: str) -> None:
        req = self._pb2.GetNodeRequest(
            context=self._context(),
            entity_type=entity_type,
            node_id=node_id,
            consistency=self._pb2.READ_CONSISTENCY_STRONG,
            fields=[],
        )
        self._node_client.GetNode(req)

    def find_nodes(
        self,
        entity_type: str,
        cel_filter: str,
        limit: int,
        cursor: bytes = b"",
    ) -> bytes:
        req = self._pb2.FindNodesRequest(
            context=self._context(),
            entity_type=entity_type,
            cel_expression=cel_filter,
            consistency=self._pb2.READ_CONSISTENCY_STRONG,
            fields=[],
            limit=limit,
            cursor=cursor,
            snapshot_mode=self._pb2.SNAPSHOT_MODE_STRICT,
            allow_degrade_to_best_effort=False,
        )
        next_cursor = b""
        stream = self._query_client.FindNodes(req)
        for item in stream:
            if item.next_cursor:
                next_cursor = item.next_cursor
        return next_cursor

    def traverse(self, entity_type: str, node_id: str, edge_label: str, limit: int) -> None:
        step = self._pb2.TraversalStep(
            edge_type=edge_label,
            direction=self._pb2.EDGE_DIRECTION_OUTGOING,
            edge_filter="",
            node_filter="",
            fields=[],
            per_node_limit=0,
            edge_fields=[],
        )
        req = self._pb2.TraverseRequest(
            context=self._context(),
            start=self._pb2.NodeRef(
                entity_type=entity_type,
                node_id=node_id,
                database="",
                namespace="",
            ),
            steps=[step],
            max_depth=2,
            timeout_ms=5000,
            max_results=limit,
            consistency=self._pb2.READ_CONSISTENCY_STRONG,
            cascade=False,
            cursor=b"",
            snapshot_mode=self._pb2.SNAPSHOT_MODE_BEST_EFFORT,
            allow_degrade_to_best_effort=True,
        )
        stream = self._query_client.Traverse(req)
        for _ in stream:
            pass


def compute_deep_cursor_bytes(
    client: GrpcBenchmarkClient,
    entity_type: str,
    cel_filter: str,
    page_size: int,
    pages: int,
) -> bytes:
    cursor = b""
    seen: set[str] = set()
    for _ in range(max(pages, 0)):
        next_cursor = client.find_nodes(entity_type, cel_filter, page_size, cursor)
        if not next_cursor:
            return b""
        next_hex = next_cursor.hex()
        if next_hex in seen:
            return b""
        seen.add(next_hex)
        cursor = next_cursor
    return cursor


def build_cli_cases(
    args: argparse.Namespace, base: List[str]
) -> tuple[List[tuple[str, List[str]]], Dict[str, float]]:
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
                    raise RuntimeError(f"context_deep_page gate failed: {msg}")
                print(f"Skipping context_deep_page: {msg}.")

    return cases, targets


def build_grpc_cases(
    args: argparse.Namespace, client: GrpcBenchmarkClient
) -> tuple[List[tuple[str, Callable[[], None]]], Dict[str, float]]:
    cases: List[tuple[str, Callable[[], None]]] = []
    targets: Dict[str, float] = {}

    if args.profile == "default":
        cases = [
            (
                "node_get",
                lambda entity_type=args.entity_type, node_id=args.seed_node_id: client.node_get(
                    entity_type, node_id
                ),
            ),
            (
                "query_find",
                lambda entity_type=args.entity_type, cel_filter=args.filter, limit=args.limit: client.find_nodes(
                    entity_type, cel_filter, limit
                ),
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
                    lambda entity_type=args.entity_type, node_id=args.seed_node_id, edge_label=args.edge_label, limit=args.limit: client.traverse(
                        entity_type, node_id, edge_label, limit
                    ),
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
                lambda entity_type=args.entity_type, node_id=args.seed_node_id: client.node_get(
                    entity_type, node_id
                ),
            ),
            (
                "context_show_shot",
                lambda entity_type=args.entity_type, cel_filter=show_shot_filter, limit=page_size: client.find_nodes(
                    entity_type, cel_filter, limit
                ),
            ),
            (
                "context_show_shot_task_label",
                lambda entity_type=args.entity_type, cel_filter=template_filter, limit=page_size: client.find_nodes(
                    entity_type, cel_filter, limit
                ),
            ),
            (
                "context_or_shots",
                lambda entity_type=args.entity_type, cel_filter=or_filter, limit=page_size: client.find_nodes(
                    entity_type, cel_filter, limit
                ),
            ),
        ]
        targets = {
            "context_point_lookup": args.target_get_ms,
            "context_show_shot": args.target_context_show_shot_ms,
            "context_show_shot_task_label": args.target_context_template_ms,
            "context_or_shots": args.target_context_or_ms,
        }

        if args.context_deep_pages > 0:
            deep_cursor = compute_deep_cursor_bytes(
                client,
                args.entity_type,
                show_scheme_filter,
                page_size,
                args.context_deep_pages,
            )
            if deep_cursor:
                cases.append(
                    (
                        "context_deep_page",
                        lambda entity_type=args.entity_type, cel_filter=show_scheme_filter, limit=page_size, cursor=deep_cursor: client.find_nodes(
                            entity_type,
                            cel_filter,
                            limit,
                            cursor,
                        ),
                    )
                )
                targets["context_deep_page"] = args.target_context_deep_page_ms
            else:
                msg = (
                    "unable to walk requested deep pages "
                    "(insufficient result depth or unstable keyset cursor progression)"
                )
                if args.require_deep_page:
                    raise RuntimeError(f"context_deep_page gate failed: {msg}")
                print(f"Skipping context_deep_page: {msg}.")

    return cases, targets


def build_output_payload(
    args: argparse.Namespace,
    results: List[BenchResult],
    targets: Dict[str, float],
    observability: Dict[str, float],
    scenario_gate_passed: Optional[bool],
    scenario_gate_summary: Optional[Dict[str, float]],
    scenario_gate_failures: List[str],
    exit_code: int,
) -> Dict[str, Any]:
    return {
        "server": args.server,
        "database": args.database,
        "namespace": args.namespace,
        "profile": args.profile,
        "transport": args.transport,
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
        "scenario": args.scenario,
        "enforce_scenario_gates": args.enforce_scenario_gates,
        "observability": observability,
        "scenario_gate": (
            {
                "passed": scenario_gate_passed,
                "summary": scenario_gate_summary,
                "failures": scenario_gate_failures,
            }
            if args.scenario
            else None
        ),
        "exit_code": exit_code,
    }


def run_benchmarks(args: argparse.Namespace) -> tuple[List[BenchResult], Dict[str, float]]:
    if args.transport == "cli":
        maybe_build_cli(args.pelago_bin, args.build_cli)
        base = base_cmd(args)
        cli_cases, targets = build_cli_cases(args, base)
        results: List[BenchResult] = []
        for name, cmd in cli_cases:
            print(f"Running {name} ({args.runs} samples, warmup {args.warmup})...")
            results.append(run_case_command(name, cmd, args.warmup, args.runs))
        return results, targets

    grpc_client = GrpcBenchmarkClient(args)
    try:
        grpc_cases, targets = build_grpc_cases(args, grpc_client)
        results: List[BenchResult] = []
        for name, case_fn in grpc_cases:
            print(f"Running {name} ({args.runs} samples, warmup {args.warmup})...")
            results.append(run_case_callable(name, case_fn, args.warmup, args.runs))
        return results, targets
    finally:
        grpc_client.close()


def main(argv: Optional[List[str]] = None) -> int:
    args = parse_args(argv)
    if args.enforce_scenario_gates and not args.scenario:
        print("scenario gate enforcement requested but no --scenario was provided")
        return 2

    try:
        results, targets = run_benchmarks(args)
    except RuntimeError as exc:
        print(str(exc))
        return 2
    except Exception as exc:  # noqa: BLE001
        print(f"benchmark execution failed: {exc}")
        return 2

    exit_code = print_summary(results, targets, args.enforce_targets)
    observability: Dict[str, float] = {}
    scenario_gate_summary: Optional[Dict[str, float]] = None
    scenario_gate_passed: Optional[bool] = None
    scenario_gate_failures: List[str] = []

    if args.scenario:
        try:
            observability = load_observability(args.observability_json)
        except Exception as exc:  # noqa: BLE001
            print(f"scenario gate input failed: {exc}")
            return 2

        scenario_gate_passed, scenario_gate_failures, scenario_gate_summary = (
            evaluate_scenario_gates(args.scenario, results, observability)
        )
        print_scenario_gate_summary(scenario_gate_summary, scenario_gate_passed)
        for failure in scenario_gate_failures:
            print(f"scenario-gate-fail: {failure}")

        if args.enforce_scenario_gates and not scenario_gate_passed:
            exit_code = 2

    if args.output_json:
        payload = build_output_payload(
            args=args,
            results=results,
            targets=targets,
            observability=observability,
            scenario_gate_passed=scenario_gate_passed,
            scenario_gate_summary=scenario_gate_summary,
            scenario_gate_failures=scenario_gate_failures,
            exit_code=exit_code,
        )
        out = Path(args.output_json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(payload, indent=2), encoding="utf-8")
        print(f"Wrote benchmark JSON: {out}")

    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
