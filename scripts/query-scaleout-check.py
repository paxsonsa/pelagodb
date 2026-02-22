#!/usr/bin/env python3
"""Validate stateless query pagination across multiple query endpoints.

This check verifies:
1) identical first-page results across endpoints
2) cursor portability: a cursor minted by endpoint A can be consumed by B/C/...
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import List


def run_capture(cmd: List[str]) -> str:
    completed = subprocess.run(
        cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, text=True
    )
    return completed.stdout.strip()


def maybe_build_cli(bin_path: str, force_build: bool) -> None:
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


def base_cmd(
    pelago_bin: str, server: str, database: str, namespace: str, fmt: str = "json"
) -> List[str]:
    return [
        pelago_bin,
        "--server",
        server,
        "--database",
        database,
        "--namespace",
        namespace,
        "--format",
        fmt,
    ]


def fetch_nodes(
    pelago_bin: str,
    server: str,
    database: str,
    namespace: str,
    entity_type: str,
    cel_filter: str,
    limit: int,
    cursor_hex: str,
) -> List[str]:
    cmd = base_cmd(pelago_bin, server, database, namespace, fmt="json") + [
        "query",
        "find",
        entity_type,
        "--filter",
        cel_filter,
        "--limit",
        str(limit),
        "--cursor-hex",
        cursor_hex,
    ]
    out = run_capture(cmd)
    if not out:
        return []
    payload = json.loads(out)
    if not isinstance(payload, list):
        raise RuntimeError(f"unexpected query JSON payload from {server}: {payload!r}")
    ids: List[str] = []
    for item in payload:
        if not isinstance(item, dict) or "id" not in item:
            raise RuntimeError(f"invalid node payload from {server}: {item!r}")
        ids.append(str(item["id"]))
    return ids


def fetch_next_cursor(
    pelago_bin: str,
    server: str,
    database: str,
    namespace: str,
    entity_type: str,
    cel_filter: str,
    limit: int,
    cursor_hex: str,
) -> str:
    cmd = base_cmd(pelago_bin, server, database, namespace, fmt="table") + [
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
    return run_capture(cmd)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate cross-endpoint query pagination behavior"
    )
    parser.add_argument(
        "--servers",
        required=True,
        help="Comma-separated query server URLs (first server is baseline)",
    )
    parser.add_argument("--database", default="default")
    parser.add_argument("--namespace", default="default")
    parser.add_argument("--entity-type", required=True)
    parser.add_argument("--filter", default="")
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("--pelago-bin", default="target/release/pelago")
    parser.add_argument("--build-cli", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    maybe_build_cli(args.pelago_bin, args.build_cli)

    servers = [s.strip() for s in args.servers.split(",") if s.strip()]
    if len(servers) < 2:
        print("error: provide at least two endpoints in --servers", file=sys.stderr)
        return 2

    baseline = servers[0]
    others = servers[1:]

    print(f"Baseline endpoint: {baseline}")
    page1_nodes = fetch_nodes(
        args.pelago_bin,
        baseline,
        args.database,
        args.namespace,
        args.entity_type,
        args.filter,
        args.limit,
        "",
    )
    page1_cursor = fetch_next_cursor(
        args.pelago_bin,
        baseline,
        args.database,
        args.namespace,
        args.entity_type,
        args.filter,
        args.limit,
        "",
    )
    print(f"Page1 rows: {len(page1_nodes)}")
    print(f"Page1 cursor: {'<empty>' if not page1_cursor else page1_cursor}")

    failed = False
    for server in others:
        other_nodes = fetch_nodes(
            args.pelago_bin,
            server,
            args.database,
            args.namespace,
            args.entity_type,
            args.filter,
            args.limit,
            "",
        )
        other_cursor = fetch_next_cursor(
            args.pelago_bin,
            server,
            args.database,
            args.namespace,
            args.entity_type,
            args.filter,
            args.limit,
            "",
        )
        same_rows = other_nodes == page1_nodes
        same_cursor = other_cursor == page1_cursor
        print(
            f"{server}: first-page rows {'OK' if same_rows else 'MISMATCH'}, "
            f"cursor {'OK' if same_cursor else 'MISMATCH'}"
        )
        failed = failed or not same_rows or not same_cursor

    if page1_cursor:
        baseline_page2_nodes = fetch_nodes(
            args.pelago_bin,
            baseline,
            args.database,
            args.namespace,
            args.entity_type,
            args.filter,
            args.limit,
            page1_cursor,
        )
        print(f"Page2 rows (baseline cursor): {len(baseline_page2_nodes)}")
        for server in others:
            other_page2_nodes = fetch_nodes(
                args.pelago_bin,
                server,
                args.database,
                args.namespace,
                args.entity_type,
                args.filter,
                args.limit,
                page1_cursor,
            )
            same_page2 = other_page2_nodes == baseline_page2_nodes
            print(
                f"{server}: baseline-cursor page2 rows {'OK' if same_page2 else 'MISMATCH'}"
            )
            failed = failed or not same_page2
    else:
        print("Skipping page2 portability check (no continuation cursor on first page).")

    if failed:
        print("scale-out check FAILED")
        return 1

    print("scale-out check PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
