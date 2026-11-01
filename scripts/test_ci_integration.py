"""The CI partition must cover every test exactly once and never accept an idle run."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location("ci_integration", Path(__file__).with_name("ci-integration.py"))
assert spec is not None and spec.loader is not None
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)


class IntegrationRunnerTests(unittest.TestCase):
    def test_trace_audit_keeps_the_whole_original_inventory_together(self):
        names = [f"{module}::test_{i}" for module in ci.TRACE_SUITES | {"ordinary"} for i in range(5)]
        selected, trace = ci.select_run(names, trace_only=True)
        self.assertTrue(trace)
        self.assertEqual(sorted(selected), sorted(ci.trace_tests(names)))
        normal = [ci.select_run(names, shard=i, shards=8) for i in range(8)]
        self.assertTrue(all(not is_trace for _, is_trace in normal))
        self.assertEqual(sorted(sum([tests for tests, _ in normal], [])), sorted(names))
        for args in [{}, {"shard": 0}, {"trace_only": True, "shard": 0, "shards": 8}]:
            with self.assertRaises(ValueError):
                ci.select_run(names, **args)
        with self.assertRaises(ValueError):
            ci.select_run(["ordinary::test"], trace_only=True)

    def test_the_original_eight_trace_suites_are_partitioned_without_loss(self):
        names = [f"{module}::test_{i}" for module in ci.TRACE_SUITES | {"timing_sensitive"} for i in range(5)]
        selected = sum([ci.trace_tests(ci.partition(names, shard, 8)) for shard in range(8)], [])
        self.assertEqual(sorted(selected), sorted(ci.trace_tests(names)))
        self.assertFalse(any("timing_sensitive" in name for name in selected))

    def test_partition_is_exhaustive_disjoint_and_stable(self):
        names = [f"module_{i}::test_{j}" for i in range(20) for j in range(15)]
        groups = [ci.partition(names, shard, 8) for shard in range(8)]
        self.assertEqual(sorted(sum(groups, [])), sorted(names))
        self.assertEqual(len(set(sum(groups, []))), len(names))
        self.assertEqual(groups, [ci.partition(list(reversed(names)), shard, 8) for shard in range(8)])

    def test_invalid_or_empty_selection_is_an_error(self):
        for names, shard, shards in [([], 0, 8), (["a"], 8, 8), (["a"], -1, 8), (["a"], 0, 0), (["a", "a"], 0, 1)]:
            with self.assertRaises(ValueError):
                ci.partition(names, shard, shards)

    def test_only_test_names_are_taken_from_libtest_listing(self):
        self.assertEqual(ci.test_names("Compiling core\na::b: test\nc::d: test\n2 tests, 0 benchmarks\n"), ["a::b", "c::d"])
        with self.assertRaises(ValueError):
            ci.test_names("0 tests, 0 benchmarks\n")

    def test_success_accounts_for_every_selected_test(self):
        ci.check_result("test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 19 filtered out", 4)
        for log in ["", "test result: ok. 0 passed; 0 failed; 4 ignored", "test result: ok. 2 passed; 0 failed; 1 ignored", "test result: FAILED. 3 passed; 1 failed; 0 ignored"]:
            with self.assertRaises(ValueError):
                ci.check_result(log, 4)

    def test_secret_check_catches_key_material_and_passphrase(self):
        key = "header\ncomment\nabcdefghijklmnop\nqrstuvwxyz012345\nfooter"
        for secret in ["abcdefghijklmnop", "BEGIN OPENSSH PRIVATE KEY", "BEGIN RSA PRIVATE KEY", "test-passphrase-1234"]:
            with self.assertRaises(ValueError):
                ci.check_secrets("prefix " + secret + " suffix", key, "test-passphrase-1234")
        ci.check_secrets("ordinary diagnostic output", key, "test-passphrase-1234")

    def test_secret_check_cannot_pass_without_key_material(self):
        with self.assertRaises(ValueError):
            ci.check_secrets("ordinary diagnostic output", "", "test-passphrase-1234")

    def test_ordinary_tests_each_have_their_own_process_and_result(self):
        calls = []
        def run(command, **kwargs):
            calls.append(command)
            kwargs['stdout'].write('test result: ok. 1 passed; 0 failed; 0 ignored\n')
            return SimpleNamespace(returncode=0, wait=lambda **_: 0)
        with patch.object(ci.subprocess, 'Popen', side_effect=run), patch.object(
            ci.Path, 'read_text', side_effect=['header\ncomment\nabcdefghijklmnop\nqrstuvwxyz012345', 'PASSPHRASE: &str = "test-passphrase"'] * 2
        ):
            ci.run_checked(['a::first', 'b::second'], trace=False)
        self.assertEqual(len(calls), 2)
        for command, name in zip(calls, ['a::first', 'b::second']):
            self.assertEqual(command[-1], name)
            self.assertNotIn('b::second' if name == 'a::first' else 'a::first', command)


if __name__ == "__main__":
    unittest.main()
