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
from typing import Dict, List, Optional


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
    elif args.enforce_scenario_gates:
        print("scenario gate enforcement requested but no --scenario was provided")
        return 2

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
        out = Path(args.output_json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(payload, indent=2), encoding="utf-8")
        print(f"Wrote benchmark JSON: {out}")

    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
