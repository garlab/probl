import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import release


class ReleaseTests(unittest.TestCase):
    def test_rejects_unsafe_and_prerelease_tags(self):
        for tag in ['main', 'v1.2.3-rc.1', 'v01.2.3', '../v1.2.3', 'v1.2.3\n', 'v1.2.3;echo bad']:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.version(tag)
        self.assertEqual(release.version('v1.2.3'), (1, 2, 3))

    def test_archive_is_repeatable_and_ignores_dirty_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            subprocess.run(['git', 'init', '-q', str(root)], check=True)
            (root / 'model.probl').write_text('report 2d6\n')
            subprocess.run(['git', '-C', str(root), 'add', '.'], check=True)
            subprocess.run(['git', '-C', str(root), '-c', 'user.name=Test', '-c', 'user.email=test@example.com',
                            '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture'], check=True)
            original = Path.cwd()
            try:
                os.chdir(root)
                release.archive('v1.2.3', root / 'first')
                (root / 'model.probl').write_text('uncommitted edit')
                (root / 'secret').write_text('untracked')
                release.archive('v1.2.3', root / 'second')
            finally:
                os.chdir(original)
            for name in ['probl-1.2.3-source.tar.gz', 'release.json', 'SHA256SUMS']:
                self.assertEqual((root/'first'/name).read_bytes(), (root/'second'/name).read_bytes())
            data = (root/'first/probl-1.2.3-source.tar.gz').read_bytes()
            with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as archive:
                self.assertEqual(archive.extractfile('probl-1.2.3/model.probl').read(), b'report 2d6\n')
                self.assertNotIn('probl-1.2.3/secret', archive.getnames())
            manifest = json.loads((root/'first/release.json').read_text())
            self.assertEqual(manifest['sha256'], hashlib.sha256(data).hexdigest())

    def test_checks_workflow_identity_before_api_or_publication(self):
        with patch.dict(os.environ, {'GITHUB_REPOSITORY': 'attacker/probl'}):
            with self.assertRaisesRegex(ValueError, 'garlab/probl'):
                release.check('v1.2.3')
        with patch.dict(os.environ, {'GITHUB_REPOSITORY': release.REPO, 'GITHUB_REF': 'refs/heads/main'}):
            with self.assertRaisesRegex(ValueError, 'version tag'):
                release.check('v1.2.3')


if __name__ == '__main__':
    unittest.main()
