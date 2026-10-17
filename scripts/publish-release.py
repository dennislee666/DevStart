#!/usr/bin/env python3
"""Publish one verified, stable release per application version, then prune its intermediate builds."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time
import tomllib


def gh(*arguments):
    result = subprocess.run(['gh', *arguments], text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip())
    return result.stdout


def packages(directory, version):
    names = [f'DevStart-{version}-macOS-universal.dmg', f'DevStart-{version}-macOS-universal.zip']
    manifest = directory / 'SHA256SUMS.txt'
    if manifest.is_symlink():
        raise ValueError('Checksum manifest must be a regular file')
    records = {}
    for line in manifest.read_text().splitlines():
        digest, name = line.split(None, 1)
        name = name.strip()
        if name not in names or name in records or not re.fullmatch('[0-9a-f]{64}', digest):
            raise ValueError('Invalid release checksum manifest')
        records[name] = digest
    if set(records) != set(names):
        raise ValueError('DMG and ZIP checksums are both required')
    for name, digest in records.items():
        path = directory / name
        if path.is_symlink() or not path.is_file():
            raise ValueError(f'Release package must be a regular file: {name}')
        with path.open('rb') as stream:
            actual_digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        if actual_digest != digest:
            raise ValueError(f'Release package checksum mismatch: {name}')
    records[manifest.name] = hashlib.sha256(manifest.read_bytes()).hexdigest()
    return records


def superseded(tag, version):
    # The separator is essential: 1.0.0 must never select 1.0.01 or another stable version.
    return tag.startswith(f'v{version}-')


def releases(repo):
    pages = json.loads(gh('api', '--paginate', '--slurp', f'repos/{repo}/releases?per_page=100'))
    return [release for page in pages for release in page]


def publish(repo, sha, directory, version):
    if not re.fullmatch('[0-9a-f]{40}|[0-9a-f]{64}', sha):
        raise ValueError('A complete source commit SHA is required')
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo):
        raise ValueError('Repository must be owner/name')
    expected = packages(directory, version)
    metadata = json.loads(gh('api', f'repos/{repo}'))
    if json.loads(gh('api', f'repos/{repo}/git/commits/{sha}'))['sha'] != sha:
        raise ValueError('Source commit does not exist in the publication repository')
    if metadata['private']:
        raise ValueError('Public installers must be published from the public repository')
    tag = f'v{version}'
    current = next((r for r in releases(repo) if r['tag_name'] == tag), None)
    if current and current.get('immutable'):
        raise ValueError('Stable release is immutable; it cannot be replaced')
    title = 'DevStart ' + version.removesuffix('.0')
    notes = Path('docs/releases') / (version.removesuffix('.0') + '.md')
    if not notes.is_file():
        notes = Path('docs/open-source/releases') / (version.removesuffix('.0') + '.md')
    if not notes.is_file():
        raise ValueError('Release notes are missing')
    # Hide the stable release while its assets change. A failed update remains a draft;
    # previously published intermediate builds are not removed until the final release is verified.
    if current:
        gh('release', 'edit', tag, '--repo', repo, '--draft=true')
    else:
        gh('release', 'create', tag, '--repo', repo, '--target', sha, '--title', title, '--notes-file', str(notes), '--draft')
    gh('release', 'upload', tag, *[str(directory / name) for name in expected], '--repo', repo, '--clobber')
    current = next(r for r in releases(repo) if r['tag_name'] == tag)
    release = None
    for _ in range(10):
        release = json.loads(gh('api', f'repos/{repo}/releases/{current["id"]}'))
        actual = {a['name']: a for a in release['assets']}
        if all(name in actual and actual[name].get('digest') == f'sha256:{digest}' and actual[name]['size'] == (directory / name).stat().st_size for name, digest in expected.items()):
            break
        time.sleep(1)
    else:
        raise ValueError('Uploaded release digests do not match verified local packages')
    refs = json.loads(gh('api', f'repos/{repo}/git/matching-refs/tags/{tag}'))
    if any(ref['ref'] == f'refs/tags/{tag}' for ref in refs):
        gh('api', '--method', 'PATCH', f'repos/{repo}/git/refs/tags/{tag}', '-f', f'sha={sha}', '-F', 'force=true')
    else:
        gh('api', '--method', 'POST', f'repos/{repo}/git/refs', '-f', f'ref=refs/tags/{tag}', '-f', f'sha={sha}')
    for asset in release['assets']:
        if asset['name'] not in expected:
            gh('api', '--method', 'DELETE', f'repos/{repo}/releases/assets/{asset["id"]}')
    gh('release', 'edit', tag, '--repo', repo, '--target', sha, '--title', title, '--notes-file', str(notes), '--draft=false', '--prerelease=false', '--latest')
    actual_sha = json.loads(gh('api', f'repos/{repo}/git/ref/tags/{tag}'))['object']['sha']
    if actual_sha != sha:
        raise ValueError('Published tag does not match the build source commit')
    deleted = []
    for old in releases(repo):
        if superseded(old['tag_name'], version):
            gh('release', 'delete', old['tag_name'], '--repo', repo, '--cleanup-tag', '--yes')
            deleted.append(old['tag_name'])
    # Remove orphaned intermediate tags, but never touch branch history or other versions.
    for ref in json.loads(gh('api', f'repos/{repo}/git/matching-refs/tags/{tag}-')):
        if ref['ref'].startswith(f'refs/tags/{tag}-'):
            gh('api', '--method', 'DELETE', f'repos/{repo}/git/refs/{ref["ref"][5:]}')
    print(json.dumps({'version': version, 'tag': tag, 'sha': sha, 'url': f'https://github.com/{repo}/releases/tag/{tag}', 'removed_intermediate_releases': deleted}, ensure_ascii=False))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--sha', default=os.getenv('GITHUB_SHA'))
    parser.add_argument('--repo', default=os.getenv('GH_REPO'))
    parser.add_argument('--check-only', action='store_true')
    arguments = parser.parse_args()
    version = tomllib.loads(Path('Cargo.toml').read_text())['package']['version']
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise ValueError('A stable three-component application version is required')
    if arguments.check_only:
        print(json.dumps({'version': version, 'verified_packages': packages(arguments.directory, version)}))
    else:
        if not arguments.sha or not arguments.repo:
            parser.error('--sha and --repo are required to publish')
        publish(arguments.repo, arguments.sha, arguments.directory, version)
