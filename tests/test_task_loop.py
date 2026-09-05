"""Isolated process/HTTP tests: never connect to the deployed task server."""
import json
import os
import signal
import time
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SCRIPT = Path(__file__).resolve().parents[1] / 'bin/task-loop'


class LoopTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.repo = self.root / 'projects/org/repo'
        self.repo.mkdir(parents=True)
        for args in [('init', '-b', 'main'), ('-c', 'user.name=Test', '-c', 'user.email=t@x', 'commit', '--allow-empty', '-m', 'initial')]:
            subprocess.run(['git', '-C', str(self.repo), *args], check=True, capture_output=True)
        self.calls = []
        self.fail_report = False
        self.refuse_report = False
        self.claim_id = 'claim-one'
        self.lose_claim = False
        self.task = {'id': 'task-one', 'title': 'Small task', 'body': 'Do the requested work', 'product_id': 'org/repo', 'branch': None, 'milestones': []}
        case = self
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                case.calls.append((self.path, body))
                status, result = 200, {}
                if self.path == '/worker/claim':
                    status = 200 if case.task else 204
                    result = {'claim_id': case.claim_id, 'lease_expires_at': '2099-01-01T00:00:00Z', 'task': case.task}
                elif self.path == '/worker/report':
                    status = 503 if case.fail_report else 409 if case.refuse_report else 200
                    result = {**case.task, 'report_id': 17}
                elif self.path == '/worker/heartbeat':
                    status = 409 if case.lose_claim else 200
                    result = {'claim_id': case.claim_id, 'lease_expires_at': '2099-01-01T00:00:00Z', 'task': case.task}
                self.send_response(status)
                self.end_headers()
                if status != 204:
                    self.wfile.write(json.dumps(result).encode())
            def log_message(self, *args):
                pass
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.agent = self.root / 'agent'
        self.report_markdown = '# 完了\n\n自由な報告。  \n検証結果と判断を一度だけ記録。\n'
        self.set_agent("import json,sys\nfrom pathlib import Path\nPath(sys.argv[sys.argv.index('-o')+1]).write_text(json.dumps({'outcome':'done','report_markdown':" + repr(self.report_markdown) + ",'commit_sha':None,'milestones':[{'name':'implemented','at':'2026-09-06T00:00:00Z','commit_sha':None}],'checks':[{'name':'test','exit_code':0}]}))\n")

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
        self.tmp.cleanup()

    def set_agent(self, code):
        self.agent.write_text('#!/usr/bin/env python3\n' + code)
        self.agent.chmod(0o755)

    def loop_command(self, *extra):
        return [str(SCRIPT), '--once', '--url', f'http://127.0.0.1:{self.server.server_port}', '--projects-root', str(self.root/'projects'), '--state-dir', str(self.root/'state'), '--agent-command', str(self.agent), '--heartbeat-seconds', '.05', *extra]

    def run_loop(self, *extra):
        return subprocess.run(self.loop_command(*extra), capture_output=True, text=True, timeout=8)

    def reports(self):
        return [body for path, body in self.calls if path == '/worker/report']

    def test_success_and_durable_logs(self):
        result = self.run_loop()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.reports()[0]['outcome'], 'done')
        self.assertTrue(list((self.root/'state').glob('claims/*/prompt.txt')))
        report = self.reports()[0]
        self.assertEqual(report['report_markdown'], self.report_markdown)
        self.assertNotIn('summary', report)
        self.assertNotIn('verification', report)
        self.assertNotIn('evidence', report['milestones'][0])
        self.assertEqual(report['run']['agent_exit'], 0)
        self.assertFalse(any(p == '/worker/runs' for p,b in self.calls))

    def test_no_work_does_not_launch(self):
        self.task = None
        self.set_agent('raise RuntimeError("must not start")\n')
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports(), [])

    def test_refused_report_rescues_full_original_body(self):
        self.refuse_report = True
        self.report_markdown = '原文の長い報告\n' * 2000
        self.set_agent("import json,sys\nfrom pathlib import Path\nPath(sys.argv[sys.argv.index('-o')+1]).write_text(json.dumps({'outcome':'done','report_markdown':" + repr(self.report_markdown) + ",'commit_sha':None,'milestones':[],'checks':[]}))\n")
        self.assertEqual(self.run_loop().returncode, 0)
        runs = [body for path, body in self.calls if path == '/worker/runs']
        self.assertEqual(len(runs), 1)
        self.assertEqual(runs[0]['body'], self.report_markdown)

    def test_report_receipt_recovery_does_not_send_second_run(self):
        self.assertEqual(self.run_loop().returncode, 0)
        folder = next((self.root/'state/claims').iterdir())
        (folder/'run-sent.json').unlink()
        self.calls.clear()
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.calls, [])
        self.assertTrue((folder/'run-sent.json').exists())

    def test_legacy_pending_journal_converts_without_losing_evidence(self):
        self.fail_report = True
        self.assertNotEqual(self.run_loop().returncode, 0)
        folder = next((self.root/'state/claims').iterdir())
        legacy = {'outcome': 'done', 'summary': 'Old summary', 'verification': 'Old checks', 'commit_sha': None,
                  'milestones': [{'name': 'implemented', 'at': '2026-09-06T00:00:00Z', 'commit_sha': None, 'evidence': 'Old evidence'}], 'checks': []}
        original = json.dumps({'claim_id': self.claim_id, **legacy})
        (folder/'report.json').write_text(original)
        (folder/'outcome.json').write_text(json.dumps(legacy))
        self.fail_report = False
        self.calls.clear()
        self.assertEqual(self.run_loop().returncode, 0)
        report = self.reports()[0]
        for text in ('Old summary', 'Old checks', 'Old evidence'):
            self.assertIn(text, report['report_markdown'])
        self.assertNotIn('evidence', report['milestones'][0])
        self.assertEqual((folder/'report.json').read_text(), original)
        self.assertFalse(any(path == '/worker/runs' for path, body in self.calls))

    def test_legacy_reported_journal_only_finishes_old_run(self):
        self.fail_report = True
        self.assertNotEqual(self.run_loop().returncode, 0)
        folder = next((self.root/'state/claims').iterdir())
        (folder/'reported.json').write_text(json.dumps({'task': {'id': self.task['id']}}))
        self.calls.clear()
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual([path for path, body in self.calls], ['/worker/runs'])

    def test_legacy_interrupted_outcome_can_resume(self):
        self.fail_report = True
        self.assertNotEqual(self.run_loop().returncode, 0)
        folder = next((self.root/'state/claims').iterdir())
        (folder/'outcome.json').write_text(json.dumps({'outcome': 'done', 'summary': 'Old summary', 'verification': 'Old checks', 'commit_sha': None, 'milestones': [], 'checks': []}))
        for name in ('run.json', 'report.json'):
            (folder/name).unlink()
        self.calls.clear()
        self.fail_report = False
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports()[0]['outcome'], 'done')
        self.assertIn('Old checks', self.reports()[0]['report_markdown'])

    def test_legacy_interrupted_raw_result_can_resume(self):
        self.fail_report = True
        self.assertNotEqual(self.run_loop().returncode, 0)
        folder = next((self.root/'state/claims').iterdir())
        (folder/'result.json').write_text(json.dumps({'outcome': 'done', 'summary': 'Old raw summary', 'verification': 'Old raw checks', 'commit_sha': None, 'milestones': [], 'checks': []}))
        for name in ('outcome.json', 'run.json', 'report.json'):
            (folder/name).unlink()
        self.calls.clear()
        self.fail_report = False
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports()[0]['outcome'], 'done')
        self.assertIn('Old raw checks', self.reports()[0]['report_markdown'])

    def test_interrupted_malformed_result_is_blocked(self):
        self.fail_report = True
        self.assertNotEqual(self.run_loop().returncode, 0)
        folder = next((self.root/'state/claims').iterdir())
        (folder/'result.json').write_text('{}')
        for name in ('outcome.json', 'run.json', 'report.json'):
            (folder/name).unlink()
        self.calls.clear()
        self.fail_report = False
        result = self.run_loop()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.reports()[0]['outcome'], 'blocked')

    def test_malformed_is_blocked(self):
        self.set_agent('pass\n')
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports()[0]['outcome'], 'blocked')

    def test_nonzero_is_blocked(self):
        self.set_agent('raise SystemExit(7)\n')
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports()[0]['outcome'], 'blocked')

    def test_timeout_terminates_child(self):
        self.set_agent('import time\ntime.sleep(30)\n')
        self.assertEqual(self.run_loop('--timeout-seconds', '.15').returncode, 0)
        self.assertEqual(self.reports()[0]['outcome'], 'blocked')
        self.assertIn('timeout', self.reports()[0]['report_markdown'])

    def test_pending_report_retried_without_rerunning(self):
        self.fail_report = True
        self.assertNotEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports()[0]['outcome'], 'done')
        self.calls.clear()
        self.fail_report = False
        self.set_agent('raise RuntimeError("must not rerun")\n')
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports()[0]['outcome'], 'done')
        self.assertFalse(any(p == '/worker/claim' for p,b in self.calls))

    def test_interrupted_journal_preserves_saved_outcome(self):
        self.fail_report = True
        self.assertNotEqual(self.run_loop().returncode, 0)
        folder = next((self.root/'state/claims').iterdir())
        (folder/'run.json').unlink()
        (folder/'report.json').unlink()
        self.fail_report = False
        self.calls.clear()
        self.set_agent('raise RuntimeError("must not rerun")\n')
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports()[0]['outcome'], 'done')
        self.assertFalse(any(p == '/worker/claim' for p,b in self.calls))

    def test_prompt_is_small_but_full_task_is_saved(self):
        self.task.update(legacy={'raw_row': 'old-secret-marker'}, claim_id='stale-claim-marker', verification='x' * 20000)
        self.assertEqual(self.run_loop().returncode, 0)
        folder = next((self.root/'state/claims').iterdir())
        prompt = (folder/'prompt.txt').read_text()
        self.assertNotIn('old-secret-marker', prompt)
        self.assertNotIn('stale-claim-marker', prompt)
        self.assertNotIn('legacy', prompt)
        context = json.loads(prompt.split('\n', 1)[1])
        self.assertEqual(len(context['task']['verification'].encode()), 8192)
        self.assertEqual(json.loads(Path(context['full_task_path']).read_text()), self.task)

    def test_fallback_does_not_repeat_existing_milestones(self):
        self.task['milestones'] = [{'name': 'implemented', 'at': '2026-09-05T00:00:00Z', 'commit_sha': 'old', 'evidence': 'already recorded'}]
        self.set_agent('raise SystemExit(7)\n')
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports()[0]['milestones'], [])

    def test_interrupted_raw_result_is_recovered(self):
        self.fail_report = True
        self.assertNotEqual(self.run_loop().returncode, 0)
        folder = next((self.root/'state/claims').iterdir())
        for name in ('outcome.json', 'run.json', 'report.json'):
            (folder/name).unlink()
        self.fail_report = False
        self.calls.clear()
        self.set_agent('raise RuntimeError("must not rerun")\n')
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports()[0]['outcome'], 'done')
        self.assertEqual(self.reports()[0]['report_markdown'], self.report_markdown)
        self.assertFalse(any(p == '/worker/claim' for p,b in self.calls))

    def test_sigkill_after_raw_done_result_reports_blocked(self):
        # The adapter has written JSON but has not exited successfully yet.
        self.agent.write_text(self.agent.read_text() + 'import time\ntime.sleep(60)\n')
        process = subprocess.Popen(self.loop_command(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            deadline = time.monotonic() + 4
            results = []
            while not results and time.monotonic() < deadline:
                results = list((self.root/'state/claims').glob('*/result.json'))
                time.sleep(.01)
            self.assertTrue(results)
            folder = results[0].parent
            self.assertFalse((folder/'agent-exit.json').exists())
            process.kill()
            process.wait(timeout=3)
            self.calls.clear()
            self.assertEqual(self.run_loop().returncode, 0)
            self.assertEqual(self.reports()[0]['outcome'], 'blocked')
            self.assertIn('unconfirmed', self.reports()[0]['report_markdown'])
            self.assertEqual(json.loads(results[0].read_text())['outcome'], 'done')
            self.assertFalse(any(p == '/worker/claim' for p,b in self.calls))
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()

    def test_sigkill_parent_stops_agent_and_grandchild_before_resume(self):
        pidfile = self.root/'child-pids.json'
        self.set_agent("import json,os,subprocess,sys,time\nfrom pathlib import Path\nchild=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'])\nPath(" + repr(str(pidfile)) + ").write_text(json.dumps([os.getpid(),child.pid]))\ntime.sleep(60)\n")
        process = subprocess.Popen(self.loop_command(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        pids = []
        def running(pid):
            try:
                return Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[0] != 'Z'
            except FileNotFoundError:
                return False
        try:
            deadline = time.monotonic() + 4
            while not pidfile.exists() and time.monotonic() < deadline:
                time.sleep(.01)
            self.assertTrue(pidfile.exists())
            pids = json.loads(pidfile.read_text())
            process.kill()
            process.wait(timeout=3)
            deadline = time.monotonic() + 3
            while any(running(pid) for pid in pids) and time.monotonic() < deadline:
                time.sleep(.01)
            self.assertFalse(any(running(pid) for pid in pids), 'parent death must terminate the complete agent group')
            self.calls.clear()
            self.assertEqual(self.run_loop().returncode, 0)
            self.assertEqual(self.reports()[0]['outcome'], 'blocked')
            self.assertFalse(any(p == '/worker/claim' for p,b in self.calls))
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            for pid in pids:
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def test_owned_dirty_workspace_can_resume(self):
        self.set_agent("from pathlib import Path\nPath('unfinished.txt').write_text('keep me')\nraise SystemExit(7)\n")
        self.assertEqual(self.run_loop().returncode, 0)
        record = json.loads(next((self.root/'state/workspaces').glob('*.json')).read_text())
        self.assertEqual((Path(record['path'])/'unfinished.txt').read_text(), 'keep me')
        self.claim_id = 'claim-two'
        self.calls.clear()
        self.set_agent("import json,sys\nfrom pathlib import Path\nassert Path('unfinished.txt').read_text() == 'keep me'\nPath(sys.argv[sys.argv.index('-o')+1]).write_text(json.dumps({'outcome':'done','report_markdown':'Resumed; kept previous work','commit_sha':None,'milestones':[],'checks':[]}))\n")
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports()[0]['outcome'], 'done')

    def test_unowned_dirty_branch_is_preserved(self):
        self.task['branch'] = 'main'
        (self.repo/'someone.txt').write_text('untouched')
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports()[0]['outcome'], 'blocked')
        self.assertEqual((self.repo/'someone.txt').read_text(), 'untouched')

    def test_missing_named_branch_is_created(self):
        self.task['branch'] = 'task/new-work'
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports()[0]['outcome'], 'done')
        record = json.loads(next((self.root/'state/workspaces').glob('*.json')).read_text())
        branch = subprocess.check_output(['git', '-C', record['path'], 'branch', '--show-current'], text=True).strip()
        self.assertEqual(branch, 'task/new-work')

    def test_lease_loss_preserves_logs_and_stops(self):
        self.lose_claim = True
        self.set_agent('import time\ntime.sleep(30)\n')
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual(self.reports(), [])
        self.assertTrue(list((self.root/'state').glob('claims/*/lease-lost.json')))
        runs = [body for path, body in self.calls if path == '/worker/runs']
        self.assertEqual(len(runs), 1)
        self.assertIn('409', runs[0]['body'])

    def test_interrupted_lease_loss_only_rescues_run(self):
        self.lose_claim = True
        self.set_agent('import time\ntime.sleep(30)\n')
        self.assertEqual(self.run_loop().returncode, 0)
        folder = next((self.root/'state/claims').iterdir())
        for name in ('run.json', 'run-sent.json'):
            (folder/name).unlink()
        self.calls.clear()
        self.assertEqual(self.run_loop().returncode, 0)
        self.assertEqual([path for path, body in self.calls], ['/worker/runs'])

    def test_lease_loss_rescues_raw_body_without_claiming_success(self):
        self.lose_claim = True
        self.agent.write_text(self.agent.read_text() + 'import time\ntime.sleep(30)\n')
        self.assertEqual(self.run_loop('--heartbeat-seconds', '.2').returncode, 0)
        self.assertEqual(self.reports(), [])
        run = next(body for path, body in self.calls if path == '/worker/runs')
        self.assertEqual(run['outcome'], 'blocked')
        self.assertEqual(run['body'], self.report_markdown)
        self.assertIn('409', run['note'])


if __name__ == '__main__':
    unittest.main()
