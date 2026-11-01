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
    def setUp(self):
        self.cleanup = patch.object(ci, 'cleanup_resources', create=True, return_value=[])
        self.cleaned = self.cleanup.start()
        self.addCleanup(self.cleanup.stop)

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

    def test_key_preparation_does_not_depend_on_which_test_runs_first(self):
        with patch.object(ci.Path, 'exists', return_value=False), patch.object(
            ci.Path, 'read_text', return_value='PASSPHRASE: &str = "test-passphrase"'
        ), patch.object(ci.Path, 'unlink'), patch.object(ci.shutil, 'copyfile'), patch.object(ci.subprocess, 'run') as run:
            ci.prepare_test_key()
        command = run.call_args.args[0]
        self.assertEqual(command[0], 'ssh-keygen')
        self.assertIn('ed25519', command)
        self.assertTrue(run.call_args.kwargs['check'])

    def test_trace_inventory_still_runs_together(self):
        with patch.object(ci, 'run_group') as run:
            ci.run_checked(['a::first', 'b::second'], trace=True)
        run.assert_called_once_with(['a::first', 'b::second'], trace=True, timeout=2400)

    def test_a_failed_test_does_not_hide_the_rest_of_the_shard(self):
        with patch.object(ci, 'run_group', side_effect=[ValueError('timed out'), None]) as run:
            with self.assertRaisesRegex(ValueError, 'a::first'):
                ci.run_checked(['a::first', 'b::second'], trace=False)
        self.assertEqual(run.call_count, 2)

    def test_timeout_logs_are_scanned_before_printing(self):
        child = SimpleNamespace(pid=123, returncode=-9)
        calls = []
        def start(command, **kwargs):
            kwargs['stdout'].write('test-passphrase')
            def wait(**options):
                if options:
                    raise ci.subprocess.TimeoutExpired(command, options['timeout'])
                return -9
            child.wait = wait
            return child
        with patch.object(ci.subprocess, 'Popen', side_effect=start), patch.object(ci.os, 'killpg', create=True), patch.object(
            ci.subprocess, 'run'
        ), patch.object(ci.Path, 'read_text', side_effect=[
            'header\ncomment\nabcdefghijklmnop\nqrstuvwxyz012345', 'PASSPHRASE: &str = "test-passphrase"'
        ]), patch('builtins.print', side_effect=lambda *args, **_: calls.append(str(args))):
            with self.assertRaisesRegex(ValueError, 'SECRET WAS FOUND'):
                ci.run_group(['a::hung'], trace=False, timeout=1)
        self.assertFalse(any('test-passphrase' in call for call in calls))

    def test_a_timeout_cleans_its_own_resources_and_still_fails(self):
        envs = []
        child = SimpleNamespace(pid=123, returncode=-9)
        def start(command, **kwargs):
            envs.append(kwargs['env'])
            def wait(**options):
                if options:
                    raise ci.subprocess.TimeoutExpired(command, options['timeout'])
                return -9
            child.wait = wait
            return child
        with patch.object(ci.subprocess, 'Popen', side_effect=start), patch.object(ci.os, 'killpg', create=True), patch.object(
            ci.subprocess, 'run'
        ), patch.object(ci.Path, 'read_text', side_effect=[
            'header\ncomment\nabcdefghijklmnop\nqrstuvwxyz012345', 'PASSPHRASE: &str = "test-passphrase"'
        ]):
            with self.assertRaisesRegex(ValueError, 'Timed out'):
                ci.run_group(['a::hung'], trace=False, timeout=1)
        self.cleaned.assert_called_once_with(envs[0]['VRCAST_TEST_RESOURCE_LABEL'])
        self.assertRegex(envs[0]['VRCAST_TEST_RESOURCE_LABEL'], r'^vrcast\.ci-run=[a-f0-9]{32}$')

    def test_a_passing_process_cannot_hide_a_resource_leak_with_cleanup(self):
        def start(command, **kwargs):
            kwargs['stdout'].write('test result: ok. 1 passed; 0 failed; 0 ignored\n')
            return SimpleNamespace(returncode=0, wait=lambda **_: 0)
        self.cleaned.return_value = ['container abc123']
        with patch.object(ci.subprocess, 'Popen', side_effect=start), patch.object(
            ci.Path, 'read_text', side_effect=[
                'header\ncomment\nabcdefghijklmnop\nqrstuvwxyz012345', 'PASSPHRASE: &str = "test-passphrase"'
            ]
        ):
            with self.assertRaisesRegex(ValueError, 'leaked'):
                ci.run_group(['a::leaky'], trace=False, timeout=1)

    def test_cleanup_never_sweeps_unlabelled_or_other_runs_resources(self):
        self.cleanup.stop()
        label = 'vrcast.ci-run=' + 'a' * 32
        replies = [SimpleNamespace(stdout=s, returncode=0) for s in ['abc123\n', '', '', 'def456\n', '', '']]
        with patch.object(ci.subprocess, 'run', side_effect=replies) as run:
            removed = ci.cleanup_resources(label)
        self.assertEqual(removed, ['container abc123', 'network def456'])
        commands = [call.args[0] for call in run.call_args_list]
        self.assertEqual(commands, [
            ['docker', 'ps', '-aq', '--filter', f'label={label}'],
            ['docker', 'rm', '-f', 'abc123'],
            ['docker', 'ps', '-aq', '--filter', f'label={label}'],
            ['docker', 'network', 'ls', '-q', '--filter', f'label={label}'],
            ['docker', 'network', 'rm', 'def456'],
            ['docker', 'network', 'ls', '-q', '--filter', f'label={label}'],
        ])
        self.assertTrue(all(call.kwargs['timeout'] == 30 for call in run.call_args_list))
        self.assertTrue(all(call.kwargs['check'] for call in run.call_args_list if '--filter' in call.args[0]))

    def test_cleanup_requires_an_exact_ownership_token(self):
        self.cleanup.stop()
        with patch.object(ci.subprocess, 'run') as run:
            for label in ['', 'vrcast', 'vrcast.ci-run=', 'vrcast.ci-run=*']:
                with self.assertRaises(ValueError):
                    ci.cleanup_resources(label)
        run.assert_not_called()

    def test_auto_removed_container_can_disappear_between_listing_and_removal(self):
        self.cleanup.stop()
        label = 'vrcast.ci-run=' + 'b' * 32
        listed = 0
        def run(command, **options):
            nonlocal listed
            if command[1:3] == ['rm', '-f']:
                if options.get('check'):
                    raise ci.subprocess.CalledProcessError(1, command)
                return SimpleNamespace(returncode=1, stdout='')
            if command[1:3] == ['ps', '-aq']:
                listed += 1
                return SimpleNamespace(stdout='abc123\n' if listed == 1 else '')
            return SimpleNamespace(stdout='')
        with patch.object(ci.subprocess, 'run', side_effect=run):
            self.assertEqual(ci.cleanup_resources(label), [])


if __name__ == "__main__":
    unittest.main()
