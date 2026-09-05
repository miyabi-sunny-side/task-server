import importlib.machinery
import importlib.util
import io
from contextlib import closing
import json
import os
from unittest.mock import patch
from pathlib import Path
import sqlite3
import tarfile
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'bin/task-data'


def load():
    loader = importlib.machinery.SourceFileLoader('task_data', str(SCRIPT))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


class DataTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.data = load()

    def test_migrate_preserves_every_column_and_finished_history(self):
        source = self.root / 'legacy.db'
        with closing(sqlite3.connect(source)) as db, db:
            db.executescript('''CREATE TABLE tasks (id TEXT, title TEXT, body TEXT, status TEXT, kind TEXT,
                updated_at TEXT, commit_sha TEXT, review_attempt INTEGER, release_tag TEXT);
                CREATE TABLE products (id TEXT, repository TEXT, releases INTEGER, updated_at TEXT);
                CREATE TABLE runs (id INTEGER, note TEXT, read_at TEXT, checks TEXT);
                CREATE TABLE claim_receipts (idempotency_key TEXT, claim_id TEXT);''')
            row = ('日本語:task', 'Title', '本文\n---\n', 'released', 'normal',
                   '2026-09-05T00:00:00Z', 'abc', 7, 'v1.2.3')
            db.execute('INSERT INTO tasks VALUES (?,?,?,?,?,?,?,?,?)', row)
            db.execute('INSERT INTO tasks VALUES (?,?,?,?,?,?,?,?,?)', ('review:x','R','', 'released','review',row[5],'abc',2,None))
            db.execute('INSERT INTO products VALUES (?,?,?,?)', ('org/repo','url',1,row[5]))
            db.execute('INSERT INTO runs VALUES (?,?,?,?)', (42,'keep unread',None,'[]'))
            db.execute('INSERT INTO claim_receipts VALUES (?,?)', ('attempt','lease'))
        destination = self.root / 'ledger'
        counts = self.data.migrate(source, destination)
        self.assertEqual(counts['tasks'], 2)
        record = self.data.read_generated(destination/'tasks'/self.data.filename(row[0]))
        self.assertEqual(record['body'], row[2])
        self.assertEqual(record['status'], 'done')
        self.assertEqual(record['legacy']['review_attempt'], 7)
        self.assertEqual(record['legacy']['body'], row[2])
        self.assertTrue(any(m['name']=='released' for m in record['milestones']))
        archived = self.data.read_generated(destination/'tasks'/self.data.filename('review:x'))
        self.assertTrue(archived['archived'])
        run = self.data.read_generated(destination/'runs'/'42.md')
        self.assertIsNone(run['read_at'])
        self.assertEqual(run['checks'], [])
        with self.assertRaises(FileExistsError):
            self.data.migrate(source, destination)
        with closing(sqlite3.connect(source)) as db, db:
            self.assertEqual(db.execute('SELECT * FROM tasks WHERE id=?',(row[0],)).fetchone(), row)

    def test_failed_import_does_not_publish_partial_ledger(self):
        source=self.root/'bad.db'
        with closing(sqlite3.connect(source)) as db, db:
            db.execute('CREATE TABLE tasks (id TEXT,status TEXT)')
            db.execute("INSERT INTO tasks VALUES ('same','draft'),('same','ready')")
        destination=self.root/'ledger'
        with self.assertRaises(ValueError):
            self.data.migrate(source,destination)
        self.assertFalse(destination.exists())

    def test_snapshot_restore_checks_bytes_and_refuses_corruption(self):
        records={name:[] for name in self.data.COLLECTIONS}
        records['tasks']=[dict(id='../日本語',title='x',body='文\n',status='draft')]
        records['runs']=[dict(id=81,note='n',read_at=None)]
        archive=self.root/'snapshot.tar.gz'
        self.data.snapshot(records,archive)
        destination=self.root/'restored'
        self.data.restore(archive,destination)
        task=self.data.read_generated(destination/'tasks'/self.data.filename('../日本語'))
        self.assertEqual(task,records['tasks'][0])
        with self.assertRaises(FileExistsError): self.data.restore(archive,destination)
        bad=self.root/'bad.tar.gz'
        with tarfile.open(archive,'r:gz') as src, tarfile.open(bad,'w:gz') as dst:
            for item in src:
                contents=src.extractfile(item).read()
                if item.name.endswith('.md'): contents+=b'corrupt'
                item.size=len(contents)
                dst.addfile(item,io.BytesIO(contents))
        with self.assertRaises(ValueError): self.data.restore(bad,self.root/'bad-restore')
        self.assertFalse((self.root/'bad-restore').exists())

    def test_r2_upload_uses_configured_destination_and_environment(self):
        archive = self.root / 'generation.tar.gz'
        archive.write_bytes(b'local backup')
        fake = self.root / 'aws'
        capture = self.root / 'aws.json'
        fake.write_text("#!/usr/bin/env python3\nimport json,os,sys\nfrom pathlib import Path\nPath(os.environ['CAPTURE']).write_text(json.dumps(dict(args=sys.argv[1:],key=os.environ['AWS_ACCESS_KEY_ID'],secret=os.environ['AWS_SECRET_ACCESS_KEY'],region=os.environ['AWS_DEFAULT_REGION'])))\n")
        fake.chmod(0o755)
        env = dict(PATH=str(self.root)+os.pathsep+os.environ['PATH'], CAPTURE=str(capture),
                   R2_ENDPOINT='https://example.invalid',R2_BUCKET='private',R2_PREFIX='ledger',
                   R2_ACCESS_KEY_ID='stub-key',R2_SECRET_ACCESS_KEY='stub-secret')
        with patch.dict(os.environ, env):
            self.assertEqual(self.data.upload(archive), 'ledger/generation.tar.gz')
        actual=json.loads(capture.read_text())
        self.assertEqual(actual['args'], ['--endpoint-url','https://example.invalid','s3','cp',
                                         str(archive),'s3://private/ledger/generation.tar.gz','--only-show-errors'])
        self.assertEqual(actual['key'],'stub-key')
        self.assertEqual(actual['region'],'auto')
        self.assertEqual(archive.read_bytes(),b'local backup')

    def test_restore_refuses_escape_paths(self):
        archive=self.root/'escape.tar.gz'
        with tarfile.open(archive,'w:gz') as dst:
            item=tarfile.TarInfo('../escape'); item.size=1
            dst.addfile(item,io.BytesIO(b'x'))
        with self.assertRaises(ValueError): self.data.restore(archive,self.root/'restored')
        self.assertFalse((self.root/'escape').exists())


if __name__ == '__main__': unittest.main()
