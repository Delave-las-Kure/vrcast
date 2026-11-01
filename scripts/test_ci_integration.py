"""The CI partition must cover every test exactly once and never accept an idle run."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("ci_integration", Path(__file__).with_name("ci-integration.py"))
assert spec is not None and spec.loader is not None
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)


class IntegrationRunnerTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
