"""Validate the independent grammar release and detect already-published retries."""
import argparse
import json
import tomllib
import urllib.error
import urllib.request

import release

CRATE = 'tree-sitter-probl'
DIRECTORY = f'editors/{CRATE}'
PREFIX = f'{CRATE}-v'


def version(tag):
    if not tag.startswith(PREFIX):
        raise ValueError(f'Use a stable grammar tag such as {PREFIX}0.1.1')
    number = tag.removeprefix(PREFIX)
    release.version(f'v{number}')
    return number


def check(tag):
    number = version(tag)
    commit = release.check_identity(tag)
    cargo = tomllib.loads(release.run('git', 'show', f'HEAD:{DIRECTORY}/Cargo.toml'))
    if cargo['package']['name'] != CRATE or cargo['package']['version'] != number:
        raise ValueError('Tag does not match the grammar Cargo package')
    metadata = json.loads(release.run('git', 'show', f'HEAD:{DIRECTORY}/tree-sitter.json'))
    if metadata['metadata']['version'] != number:
        raise ValueError('Tag does not match tree-sitter.json metadata.version')
    release.check_remote_tag(tag, commit)
    return number


def published(number):
    version(f'{PREFIX}{number}')
    request = urllib.request.Request(
        f'https://crates.io/api/v1/crates/{CRATE}/{number}',
        headers={'User-Agent': 'probl release (https://github.com/garlab/probl)',
                 'Accept': 'application/json'},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            existing = json.load(response)['version']
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return False
        raise
    if existing['crate'] != CRATE or existing['num'] != number:
        raise ValueError('Registry response identifies a different package version')
    if existing['yanked']:
        raise ValueError(f'{CRATE} {number} is already published and yanked')
    return True


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['check', 'published'])
    parser.add_argument('value', help='release tag for check, version for published')
    args = parser.parse_args()
    if args.command == 'check':
        print(check(args.value))
    else:
        print(str(published(args.value)).lower())
