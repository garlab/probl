import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import urllib.error
from unittest.mock import patch

import grammar_release
import release


class GrammarReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.command = release.run
        self.git('init', '-q', '-b', 'main')
        self.directory = self.root / grammar_release.DIRECTORY
        self.directory.mkdir(parents=True)
        (self.root / 'Cargo.toml').write_text('[workspace.package]\nversion = "9.9.9"\n')
        self.manifest = self.directory / 'Cargo.toml'
        self.manifest.write_text('[package]\nname = "tree-sitter-probl"\nversion = "1.2.3"\n')
        self.metadata = self.directory / 'tree-sitter.json'
        self.metadata.write_text(json.dumps({'metadata': {'version': '1.2.3'}}))
        self.commit()
        self.tag = 'tree-sitter-probl-v1.2.3'

    def git(self, *args):
        return self.command('git', *args, cwd=self.root, stderr=subprocess.PIPE)

    def commit(self):
        self.git('add', '.')
        self.git('-c', 'user.name=Test', '-c', 'user.email=test@example.com',
                 '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture')
        self.sha = self.git('rev-parse', 'HEAD')
        self.git('update-ref', 'refs/remotes/origin/main', self.sha)

    def check(self, responses=None, environment=None):
        env = {'GITHUB_REPOSITORY': release.REPO,
               'GITHUB_REF': f'refs/tags/{self.tag}', 'GITHUB_SHA': self.sha}
        env.update(environment or {})
        responses = responses or {
            f'git/ref/tags/{self.tag}': {'object': {'type': 'commit', 'sha': self.sha}},
        }
        with patch.dict(os.environ, env), \
             patch.object(release, 'run', side_effect=lambda *args: self.command(
                 *args, cwd=self.root, stderr=subprocess.PIPE)), \
             patch.object(release, 'api', side_effect=lambda path: responses[path]):
            return grammar_release.check(self.tag)

    def test_version_is_independent_and_uses_committed_files(self):
        self.manifest.write_text('uncommitted and invalid')
        self.metadata.write_text('{}')
        self.assertEqual(self.check(), '1.2.3')

    def test_both_metadata_versions_and_package_name_must_match(self):
        cases = [('Cargo version', self.manifest, '1.2.3', '1.2.4', 'Cargo package'),
                 ('package name', self.manifest, 'tree-sitter-probl', 'other', 'Cargo package'),
                 ('Tree-sitter version', self.metadata, '1.2.3', '1.2.4', 'metadata.version')]
        for name, path, old, new, message in cases:
            with self.subTest(case=name):
                original = path.read_text()
                path.write_text(original.replace(old, new))
                self.commit()
                # No remote lookup should occur before metadata is accepted.
                with self.assertRaisesRegex(ValueError, message):
                    self.check(responses={'unexpected API call': None})
                path.write_text(original)
                self.commit()

    def test_workflow_identity_must_match(self):
        for change, message in [
            ({'GITHUB_REPOSITORY': 'other/probl'}, 'garlab/probl'),
            ({'GITHUB_REF': 'refs/heads/main'}, 'version tag'),
            ({'GITHUB_SHA': '0' * 40}, 'workflow commit'),
        ]:
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, message):
                self.check(environment=change, responses={'unexpected API call': None})

    def test_release_commit_must_be_on_remote_main(self):
        main = self.sha
        (self.root / 'change').write_text('release not pushed to main yet')
        self.commit()
        self.git('update-ref', 'refs/remotes/origin/main', main)
        with self.assertRaisesRegex(ValueError, 'not on origin/main'):
            self.check(responses={'unexpected API call': None})

    def test_annotated_tags_resolve_to_the_tested_commit(self):
        responses = {
            f'git/ref/tags/{self.tag}': {'object': {'type': 'tag', 'sha': 'annotation'}},
            'git/tags/annotation': {'object': {'type': 'commit', 'sha': self.sha}},
        }
        self.assertEqual(self.check(responses), '1.2.3')
        responses['git/tags/annotation']['object']['sha'] = '0' * 40
        with self.assertRaisesRegex(ValueError, 'Remote tag'):
            self.check(responses)

    def test_non_commit_remote_refs_are_rejected(self):
        for kind in ['blob', 'tree']:
            with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, 'Remote tag'):
                self.check({f'git/ref/tags/{self.tag}': {'object': {'type': kind, 'sha': self.sha}}})


class GrammarRegistryTests(unittest.TestCase):
    def test_only_stable_grammar_tags_are_accepted(self):
        self.assertEqual(grammar_release.version('tree-sitter-probl-v0.2.10'), '0.2.10')
        for tag in ['v0.1.0', 'main', 'tree-sitter-probl-v01.2.3',
                    'tree-sitter-probl-v1.2.3-rc.1', 'tree-sitter-probl-v1.2.3\n',
                    'tree-sitter-probl-v1.2.3;echo bad', '../tree-sitter-probl-v1.2.3']:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                grammar_release.version(tag)

    def test_existing_version_can_be_skipped_on_retry(self):
        data = {'version': {'crate': 'tree-sitter-probl', 'num': '1.2.3', 'yanked': False}}
        with patch('urllib.request.urlopen', return_value=io.BytesIO(json.dumps(data).encode())):
            self.assertTrue(grammar_release.published('1.2.3'))

    def test_only_a_404_means_not_published(self):
        for code in [404, 403, 429, 500]:
            error = urllib.error.HTTPError('https://crates.io', code, 'test error', {}, None)
            with self.subTest(code=code), patch('urllib.request.urlopen', side_effect=error):
                if code == 404:
                    self.assertFalse(grammar_release.published('1.2.3'))
                else:
                    with self.assertRaises(urllib.error.HTTPError):
                        grammar_release.published('1.2.3')
        with patch('urllib.request.urlopen', side_effect=urllib.error.URLError('offline')):
            with self.assertRaises(urllib.error.URLError):
                grammar_release.published('1.2.3')

    def test_registry_mismatches_and_yanked_versions_fail_closed(self):
        for fields in [{'crate': 'other'}, {'num': '1.2.4'}, {'yanked': True}]:
            version = {'crate': 'tree-sitter-probl', 'num': '1.2.3', 'yanked': False, **fields}
            data = io.BytesIO(json.dumps({'version': version}).encode())
            with self.subTest(fields=fields), patch('urllib.request.urlopen', return_value=data):
                with self.assertRaises(ValueError):
                    grammar_release.published('1.2.3')


if __name__ == '__main__':
    unittest.main()
