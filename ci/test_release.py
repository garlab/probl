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


def browser_fixture(folder, commit='a' * 40):
    name = 'probl-1.2.3-browser.json'
    (folder / name).write_text(json.dumps({'schema': 1, 'tag': 'v1.2.3', 'commit': commit}))
    (folder / f'{name}.sha256').write_text(f'{release.digest(folder / name)}  {name}\n')


class ReleaseTests(unittest.TestCase):
    def test_find_release_includes_paginated_drafts(self):
        draft = {'id': 7, 'tag_name': 'v1.2.3', 'draft': True}
        responses = {
            f'releases/tags/v1.2.3': None,
            f'releases?per_page=100&page=1': [
                {'tag_name': 'v0.0.1', 'draft': False}] * 100,
            f'releases?per_page=100&page=2': [draft],
        }
        with patch.object(release, 'api', side_effect=lambda path, **kw: responses[path]):
            self.assertEqual(release.find_release('v1.2.3'), draft)

    def test_find_release_distinguishes_missing_and_published(self):
        with patch.object(release, 'api', side_effect=[None, []]):
            self.assertIsNone(release.find_release('v1.2.3'))
        published = {'id': 7, 'draft': False}
        with patch.object(release, 'api', return_value=published) as api:
            self.assertEqual(release.find_release('v1.2.3'), published)
            self.assertEqual(api.call_count, 1)

    def test_find_release_rejects_ambiguous_drafts(self):
        drafts = [{'id': n, 'tag_name': 'v1.2.3', 'draft': True} for n in [7, 8]]
        with patch.object(release, 'api', side_effect=[None, drafts]):
            with self.assertRaisesRegex(ValueError, 'Multiple releases'):
                release.find_release('v1.2.3')

    def test_publish_creates_or_resumes_draft_and_preserves_assets(self):
        for already_exists in [False, True]:
            with self.subTest(already_exists=already_exists), tempfile.TemporaryDirectory() as temporary:
                folder = Path(temporary)
                archive = folder / 'probl-1.2.3-source.tar.gz'
                archive.write_bytes(b'source')
                (folder / 'release.json').write_text(json.dumps({
                    'commit': 'a' * 40, 'tag': 'v1.2.3', 'archive': archive.name,
                    'sha256': release.digest(archive)}))
                (folder / 'SHA256SUMS').write_text('fixture')
                browser_fixture(folder)
                upload_url = 'https://uploads.github.com/repos/garlab/probl/releases/7/assets'
                draft = {'id': 7, 'tag_name': 'v1.2.3', 'draft': True, 'prerelease': False,
                         'upload_url': upload_url + '{?name,label}', 'assets': []}
                earlier = dict(draft, assets=[{'name': archive.name}])
                uploads = []
                published = False
                def api(path, missing=False, method='GET', body=None, url=None, content_type=None):
                    nonlocal published
                    if (method, path) == ('GET', 'releases/tags/v1.2.3'):
                        return None  # never a draft
                    if (method, path) == ('GET', 'releases?per_page=100&page=1'):
                        # A draft from an earlier run is listed; the one this
                        # run creates isn't, as can happen right after.
                        return [earlier] if already_exists else []
                    if (method, path) == ('POST', 'releases'):
                        self.assertFalse(already_exists)
                        self.assertEqual((body['tag_name'], body['target_commitish'], body['draft']),
                                         ('v1.2.3', 'a' * 40, True))
                        return draft
                    if method == 'POST' and url and url.startswith(upload_url + '?name='):
                        uploads.append(url)
                        return {}
                    if (method, path) == ('PATCH', 'releases/7'):
                        self.assertEqual(body, {'draft': False, 'make_latest': 'true'})
                        published = True
                        return {}
                    if (method, path) == ('GET', 'releases/7'):
                        return {'immutable': published}
                    raise AssertionError((method, path, url))
                def command(*args):
                    self.assertEqual(args[2], 'download')
                    (Path(args[args.index('--dir') + 1]) / archive.name).write_bytes(archive.read_bytes())
                with patch.dict(os.environ, {'GITHUB_SHA': 'a' * 40}), \
                     patch.object(release, 'check'), patch.object(release, 'api', side_effect=api), \
                     patch.object(release, 'run', side_effect=command):
                    release.publish('v1.2.3', folder)
                names = [url.split('?name=')[1] for url in uploads]
                expected = ['release.json', 'SHA256SUMS', 'probl-1.2.3-browser.json', 'probl-1.2.3-browser.json.sha256']
                self.assertEqual(names, expected if already_exists else [archive.name] + expected)
                self.assertTrue(published)

    def test_publish_refuses_to_replace_a_published_asset(self):
        with tempfile.TemporaryDirectory() as temporary:
            folder = Path(temporary)
            archive = folder / 'probl-1.2.3-source.tar.gz'
            archive.write_bytes(b'source')
            (folder / 'release.json').write_text(json.dumps({
                'commit': 'a' * 40, 'tag': 'v1.2.3', 'archive': archive.name,
                'sha256': release.digest(archive)}))
            (folder / 'SHA256SUMS').write_text('fixture')
            browser_fixture(folder)
            published = {'id': 7, 'tag_name': 'v1.2.3', 'draft': False, 'prerelease': False,
                         'upload_url': 'unused{?name,label}', 'assets': [{'name': archive.name}]}
            def command(*args):
                (Path(args[args.index('--dir') + 1]) / archive.name).write_bytes(b'other bytes')
            with patch.dict(os.environ, {'GITHUB_SHA': 'a' * 40}), \
                 patch.object(release, 'check'), patch.object(release, 'api', return_value=published), \
                 patch.object(release, 'run', side_effect=command):
                with self.assertRaisesRegex(ValueError, 'never overwrite'):
                    release.publish('v1.2.3', folder)

    def test_publish_rejects_another_browser_bundle_before_any_release_change(self):
        for case, error in [('commit', 'different release'), ('checksum', 'Browser bundle checksum')]:
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                folder = Path(temporary)
                archive = folder / 'probl-1.2.3-source.tar.gz'
                archive.write_bytes(b'source')
                (folder / 'release.json').write_text(json.dumps({
                    'commit': 'a' * 40, 'tag': 'v1.2.3', 'archive': archive.name,
                    'sha256': release.digest(archive)}))
                browser_fixture(folder, commit='b' * 40 if case == 'commit' else 'a' * 40)
                if case == 'checksum':
                    (folder / 'probl-1.2.3-browser.json.sha256').write_text('0' * 64 + '  probl-1.2.3-browser.json\n')
                with patch.dict(os.environ, {'GITHUB_SHA': 'a' * 40}), \
                     patch.object(release, 'check'), patch.object(release, 'api') as api:
                    with self.assertRaisesRegex(ValueError, error):
                        release.publish('v1.2.3', folder)
                    api.assert_not_called()

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
