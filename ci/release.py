"""Release assets are built from committed Git objects, never the working tree."""
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import tomllib
import urllib.error
import urllib.parse
import urllib.request

REPO = "garlab/probl"


def run(*args, **kwargs):
    return subprocess.check_output(args, **kwargs).decode().strip()


def api(path, missing=False, method='GET', body=None, url=None, content_type='application/json'):
    headers = {"Authorization": f"Bearer {os.environ['GH_TOKEN']}",
               "Accept": "application/vnd.github+json"}
    data = None
    if body is not None:
        data = body if isinstance(body, bytes) else json.dumps(body).encode()
        headers["Content-Type"] = content_type
    request = urllib.request.Request(
        url or f"https://api.github.com/repos/{REPO}/{path}",
        data=data, headers=headers, method=method,
    )
    try:
        with urllib.request.urlopen(request) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        if missing and error.code == 404:
            return None
        raise


def find_release(tag):
    # The by-tag endpoint only returns published releases. The authenticated
    # list endpoint also includes drafts, including those from a failed run.
    published = api(f'releases/tags/{tag}', missing=True)
    if published is not None:
        return published
    matches = []
    page = 1
    while True:
        releases = api(f'releases?per_page=100&page={page}')
        matches.extend(item for item in releases if item['tag_name'] == tag)
        if len(releases) < 100:
            break
        page += 1
    if len(matches) > 1:
        raise ValueError(f'Multiple releases found for {tag}; resolve duplicate drafts before retrying')
    return matches[0] if matches else None


def version(tag):
    if not re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", tag):
        raise ValueError("Use a stable version tag such as v0.2.2")
    return tuple(map(int, tag[1:].split('.')))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def upload(release, path):
    # The upload URL is a URI template: .../releases/7/assets{?name,label}.
    url = release['upload_url'].split('{', 1)[0] + '?' + urllib.parse.urlencode({'name': path.name})
    kind = {'.gz': 'application/gzip', '.json': 'application/json'}.get(path.suffix, 'application/octet-stream')
    api(None, method='POST', url=url, body=path.read_bytes(), content_type=kind)


def check(tag):
    version(tag)
    if os.environ['GITHUB_REPOSITORY'] != REPO:
        raise ValueError("Releases may only run in garlab/probl")
    if os.environ['GITHUB_REF'] != f"refs/tags/{tag}":
        raise ValueError("Run this workflow on a version tag, not a branch")
    commit = run('git', 'rev-parse', 'HEAD')
    if commit != os.environ['GITHUB_SHA']:
        raise ValueError("Checkout does not match the workflow commit")
    run('git', 'merge-base', '--is-ancestor', 'HEAD', 'origin/main')
    cargo = tomllib.loads(run('git', 'show', 'HEAD:Cargo.toml'))
    if tag != 'v' + cargo['workspace']['package']['version']:
        raise ValueError("Tag does not match the Cargo workspace version")
    obj = api(f'git/ref/tags/{tag}')['object']
    for _ in range(5):
        if obj['type'] != 'tag':
            break
        obj = api(f'git/tags/{obj["sha"]}')['object']
    if obj['type'] != 'commit' or obj['sha'] != commit:
        raise ValueError('Remote tag no longer identifies the tested commit')
    latest = api('releases/latest', missing=True)
    if latest and version(latest['tag_name']) > version(tag):
        raise ValueError("Refusing to publish an older version over a newer release")
    return commit


def archive(tag, directory):
    version(tag)
    directory.mkdir(parents=True, exist_ok=True)
    commit = run('git', 'rev-parse', 'HEAD')
    name = f"probl-{tag[1:]}-source.tar.gz"
    # gzip mtime and filename are omitted: retries produce identical bytes.
    content = subprocess.check_output(['git', 'archive', '--format=tar',
                                      f'--prefix=probl-{tag[1:]}/', commit])
    (directory / name).write_bytes(gzip.compress(content, mtime=0))
    manifest = {'schema': 1, 'tag': tag, 'commit': commit,
                'archive': name, 'sha256': digest(directory / name)}
    (directory / 'release.json').write_text(json.dumps(manifest, sort_keys=True) + '\n')
    (directory / 'SHA256SUMS').write_text(''.join(
        f'{digest(directory / item)}  {item}\n' for item in [name, 'release.json']))


def publish(tag, directory):
    check(tag)
    manifest = json.loads((directory / 'release.json').read_text())
    if manifest['commit'] != os.environ['GITHUB_SHA'] or manifest['tag'] != tag:
        raise ValueError("Assets belong to a different release")
    if digest(directory / manifest['archive']) != manifest['sha256']:
        raise ValueError("Source archive checksum mismatch")
    browser = f'probl-{tag[1:]}-browser.json'
    data = json.loads((directory / browser).read_text())
    if data.get('schema') != 1 or data.get('tag') != tag or data.get('commit') != manifest['commit']:
        raise ValueError('Browser bundle belongs to a different release')
    expected = f'{digest(directory / browser)}  {browser}\n'
    if (directory / f'{browser}.sha256').read_text() != expected:
        raise ValueError('Browser bundle checksum mismatch')
    existing = find_release(tag)
    if not existing:
        # A draft is never found by its tag, and may not be listed yet right
        # after it's created: keep the release the API returns, and change it
        # by its id. check() has verified that the tag names this commit.
        existing = api('releases', method='POST', body={
            'tag_name': tag, 'target_commitish': manifest['commit'], 'name': tag,
            'draft': True, 'generate_release_notes': True})
    if existing['prerelease']:
        raise ValueError("Refusing to replace an existing prerelease")
    known = {asset['name'] for asset in existing['assets']}
    for asset in [manifest['archive'], 'release.json', 'SHA256SUMS', browser, f'{browser}.sha256']:
        if asset in known:
            with tempfile.TemporaryDirectory() as temporary:
                run('gh', 'release', 'download', tag, '--repo', REPO,
                    '--pattern', asset, '--dir', temporary)
                if digest(Path(temporary) / asset) != digest(directory / asset):
                    raise ValueError(f"Existing {asset} differs; never overwrite release assets")
        elif existing['draft']:
            upload(existing, directory / asset)
        else:
            raise ValueError(f"Published release is missing {asset}; use a new version")
    if existing['draft']:
        api(f"releases/{existing['id']}", method='PATCH', body={'draft': False, 'make_latest': 'true'})
    if not api(f"releases/{existing['id']}").get('immutable'):
        raise ValueError("Enable immutable releases before releasing; this release is not immutable")


if __name__ == '__main__':
    command, tag = sys.argv[1:3]
    if command == 'check':
        check(tag)
    elif command == 'archive':
        archive(tag, Path(sys.argv[3]))
    elif command == 'publish':
        publish(tag, Path(sys.argv[3]))
    else:
        raise ValueError(f"Unknown command: {command}")
