import importlib.util
import pathlib
import sys
import unittest


MODULE_PATH = pathlib.Path(__file__).with_name("perf-benchmark.py")
SPEC = importlib.util.spec_from_file_location("perf_benchmark", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("failed to load scripts/perf-benchmark.py module spec")
perf = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = perf
SPEC.loader.exec_module(perf)


class PerfBenchmarkArgsTests(unittest.TestCase):
    def test_parse_args_transport_defaults_to_cli(self) -> None:
        args = perf.parse_args([])
        self.assertEqual(args.transport, "cli")

    def test_parse_args_transport_grpc(self) -> None:
        args = perf.parse_args(["--transport", "grpc"])
        self.assertEqual(args.transport, "grpc")

    def test_main_requires_scenario_when_enforcing_scenario_gates(self) -> None:
        rc = perf.main(["--enforce-scenario-gates"])
        self.assertEqual(rc, 2)


class PerfBenchmarkGateTests(unittest.TestCase):
    def test_print_summary_enforces_targets(self) -> None:
        results = [perf.BenchResult(name="node_get", samples_ms=[1.0, 2.0, 3.0])]
        targets = {"node_get": 2.5}
        self.assertEqual(perf.print_summary(results, targets, enforce=False), 0)
        self.assertEqual(perf.print_summary(results, targets, enforce=True), 2)

    def test_scenario_gate_fails_when_observability_missing(self) -> None:
        results = [perf.BenchResult(name="node_get", samples_ms=[1.0, 2.0, 3.0])]
        passed, failures, _summary = perf.evaluate_scenario_gates("S1", results, {})
        self.assertFalse(passed)
        self.assertTrue(any("missing cache_hit_ratio" in f for f in failures))
        self.assertTrue(any("missing projector_lag_events" in f for f in failures))
        self.assertTrue(any("missing fallback_rate" in f for f in failures))

    def test_build_output_payload_includes_transport(self) -> None:
        args = perf.parse_args(["--transport", "grpc"])
        payload = perf.build_output_payload(
            args=args,
            results=[perf.BenchResult(name="node_get", samples_ms=[1.0])],
            targets={"node_get": 4.0},
            observability={},
            scenario_gate_passed=None,
            scenario_gate_summary=None,
            scenario_gate_failures=[],
            exit_code=0,
        )
        self.assertEqual(payload["transport"], "grpc")


class PerfBenchmarkGrpcCasesTests(unittest.TestCase):
    class _FakeGrpcClient:
        def __init__(self) -> None:
            self.calls = []
            self._cursor_idx = 0
            self._cursors = [b"a", b"a"]

        def node_get(self, entity_type: str, node_id: str) -> None:
            self.calls.append(("node_get", entity_type, node_id))

        def find_nodes(self, entity_type: str, cel_filter: str, limit: int, cursor: bytes = b"") -> bytes:
            self.calls.append(("find_nodes", entity_type, cel_filter, limit, cursor))
            return b""

        def traverse(self, entity_type: str, node_id: str, edge_label: str, limit: int) -> None:
            self.calls.append(("traverse", entity_type, node_id, edge_label, limit))

        def next_cursor_for_loop(self, *_args, **_kwargs) -> bytes:
            cursor = self._cursors[self._cursor_idx]
            self._cursor_idx = min(self._cursor_idx + 1, len(self._cursors) - 1)
            return cursor

    def test_build_grpc_cases_default_profile(self) -> None:
        args = perf.parse_args(["--transport", "grpc", "--skip-traverse"])
        fake = self._FakeGrpcClient()
        cases, targets = perf.build_grpc_cases(args, fake)
        self.assertEqual([name for name, _ in cases], ["node_get", "query_find"])
        self.assertEqual(targets["node_get"], args.target_get_ms)
        self.assertEqual(targets["query_find"], args.target_find_ms)
        for _name, fn in cases:
            fn()
        self.assertEqual(len(fake.calls), 2)

    def test_compute_deep_cursor_bytes_breaks_on_loop(self) -> None:
        fake = self._FakeGrpcClient()
        fake.find_nodes = fake.next_cursor_for_loop  # type: ignore[assignment]
        cursor = perf.compute_deep_cursor_bytes(
            fake,
            entity_type="Context",
            cel_filter="show == 'show_001'",
            page_size=10,
            pages=2,
        )
        self.assertEqual(cursor, b"")


if __name__ == "__main__":
    unittest.main()
