#!/usr/bin/env python3
"""Developer ID sign, notarize and staple app/DMG; fail closed and never log credentials."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import secrets
import subprocess
import tempfile
import time


def configuration(env):
    required = ['MACOS_CERTIFICATE_P12_BASE64', 'MACOS_CERTIFICATE_PASSWORD', 'MACOS_SIGNING_IDENTITY', 'APPLE_TEAM_ID']
    missing = [name for name in required if not env.get(name)]
    api_fields = ['APPLE_API_KEY_P8_BASE64', 'APPLE_API_KEY_ID']
    if any(env.get(name) for name in api_fields + ['APPLE_API_ISSUER_ID']):
        missing += [name for name in api_fields if not env.get(name)]
        method = 'api_key'
    else:
        missing += [name for name in ['APPLE_ID', 'APPLE_APP_SPECIFIC_PASSWORD'] if not env.get(name)]
        method = 'apple_id'
    if missing:
        raise ValueError('Missing signing/notarization secrets: ' + ', '.join(missing))
    if not re.fullmatch('[A-Z0-9]{10}', env['APPLE_TEAM_ID']):
        raise ValueError('APPLE_TEAM_ID must be the 10-character developer team identifier')
    identity = env['MACOS_SIGNING_IDENTITY']
    if not identity.startswith('Developer ID Application:') and not re.fullmatch('[0-9A-Fa-f]{40}', identity):
        raise ValueError('A Developer ID Application identity is required; Apple Distribution is not valid here')
    return method


def private_file(path, data):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, 'wb') as output:
        output.write(data)


def decode(value):
    return base64.b64decode(''.join(value.split()), validate=True)


def run(arguments, *, allow_failure=False):
    result = subprocess.run([str(arg) for arg in arguments], capture_output=True, text=True)
    if result.returncode and not allow_failure:
        # Never format the complete command: keychain/password arguments contain secrets.
        diagnostic = (result.stderr or result.stdout)[-6000:]
        for name in ['MACOS_CERTIFICATE_PASSWORD', 'APPLE_APP_SPECIFIC_PASSWORD']:
            if os.environ.get(name):
                diagnostic = diagnostic.replace(os.environ[name], '***')
        raise RuntimeError(f'{arguments[0]} failed ({result.returncode}): {diagnostic}')
    return result


def notarize(artifact, authentication, diagnostics):
    print(f'Notarizing {artifact.name}', flush=True)
    result = run(['xcrun', 'notarytool', 'submit', artifact, *authentication, '--output-format', 'json', '--no-wait'], allow_failure=True)
    try:
        submission = json.loads(result.stdout)
    except json.JSONDecodeError:
        raise RuntimeError(f'Notarization submission did not return JSON: {(result.stderr or result.stdout)[-3000:]}') from None
    submission_id = submission.get('id')
    if result.returncode or not submission_id:
        raise RuntimeError(f'Notarization submission failed: {submission.get("message", result.stderr[-3000:])}')
    diagnostics.mkdir(parents=True, exist_ok=True)
    report_path = diagnostics / (artifact.name + '.json')
    deadline = time.monotonic() + 35 * 60
    last_status = None
    while True:
        result = run(['xcrun', 'notarytool', 'info', submission_id, *authentication, '--output-format', 'json'], allow_failure=True)
        try:
            report = json.loads(result.stdout)
        except json.JSONDecodeError:
            raise RuntimeError(f'Notarization status query failed for {submission_id}: {(result.stderr or result.stdout)[-3000:]}') from None
        report_path.write_text(json.dumps(report, indent=2))
        if result.returncode:
            raise RuntimeError(f'Notarization status query failed for {submission_id}: {report.get("message", result.stderr[-3000:])}')
        status = report.get('status')
        if status != last_status:
            print(f'Notarization {artifact.name}: {status or "unknown status"} ({submission_id})', flush=True)
            last_status = status
        if status == 'Accepted':
            print(f'Notarization accepted: {submission_id}', flush=True)
            return
        if status != 'In Progress':
            run(['xcrun', 'notarytool', 'log', submission_id, *authentication, diagnostics / (artifact.name + '.log.json')], allow_failure=True)
            raise RuntimeError(f'Notarization not accepted: status={status}, id={submission_id}; inspect notarization diagnostics')
        if time.monotonic() >= deadline:
            raise RuntimeError(f'Notarization still in progress after 35 minutes: id={submission_id}; inspect {report_path} and resume this submission instead of resubmitting it')
        time.sleep(30)


def build(app, assets):
    env = os.environ
    method = configuration(env)
    info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    if info['CFBundleIdentifier'] != 'com.devstart.public':
        raise ValueError('Only the public-edition bundle can be published')
    version = info['CFBundleShortVersionString']
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise ValueError('Invalid bundle version')
    assets.mkdir(parents=True, exist_ok=True)
    diagnostics = Path(env.get('RUNNER_TEMP', tempfile.gettempdir())) / 'devstart-notary-diagnostics'
    previous_keychains = run(['security', 'list-keychains', '-d', 'user']).stdout
    keychains = re.findall(r'"([^"]+)"', previous_keychains)
    with tempfile.TemporaryDirectory(prefix='devstart-signing-') as temporary:
        work = Path(temporary)
        keychain = work / 'release.keychain-db'
        keychain_password = secrets.token_urlsafe(40)
        created = False
        try:
            certificate = work / 'signing.p12'
            private_file(certificate, decode(env['MACOS_CERTIFICATE_P12_BASE64']))
            run(['security', 'create-keychain', '-p', keychain_password, keychain])
            created = True
            run(['security', 'set-keychain-settings', '-lut', '7200', keychain])
            run(['security', 'unlock-keychain', '-p', keychain_password, keychain])
            run(['security', 'import', certificate, '-P', env['MACOS_CERTIFICATE_PASSWORD'], '-k', keychain, '-T', '/usr/bin/codesign', '-T', '/usr/bin/security'])
            run(['security', 'set-key-partition-list', '-S', 'apple-tool:,apple:,codesign:', '-s', '-k', keychain_password, keychain])
            run(['security', 'list-keychains', '-d', 'user', '-s', keychain, *keychains])
            if method == 'api_key':
                key = work / 'notary.p8'
                private_file(key, decode(env['APPLE_API_KEY_P8_BASE64']))
                authentication = ['--key', key, '--key-id', env['APPLE_API_KEY_ID']]
                if env.get('APPLE_API_ISSUER_ID'):
                    authentication += ['--issuer', env['APPLE_API_ISSUER_ID']]
            else:
                authentication = ['--apple-id', env['APPLE_ID'], '--password', env['APPLE_APP_SPECIFIC_PASSWORD'], '--team-id', env['APPLE_TEAM_ID']]
            identity = env['MACOS_SIGNING_IDENTITY']
            # Rust bundle has one executable and no nested frameworks or helpers to sign first.
            run(['codesign', '--force', '--options', 'runtime', '--timestamp', '--keychain', keychain, '--sign', identity, app])
            run(['codesign', '--verify', '--deep', '--strict', app])
            signature = run(['codesign', '-d', '--verbose=4', app]).stderr
            if 'Authority=Developer ID Application:' not in signature or f'TeamIdentifier={env["APPLE_TEAM_ID"]}' not in signature or 'runtime' not in signature:
                raise RuntimeError('Bundle does not have the required Developer ID team signature and hardened runtime')
            submission = work / 'notarization.zip'
            run(['ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', app, submission])
            notarize(submission, authentication, diagnostics)
            run(['xcrun', 'stapler', 'staple', app])
            run(['xcrun', 'stapler', 'validate', app])
            run(['spctl', '--assess', '--type', 'execute', '--verbose=2', app])
            dmg_root = work / 'dmg-root'
            dmg_root.mkdir()
            # ditto preserves the app's stapled ticket and extended attributes in the disk image input.
            run(['ditto', app, dmg_root / 'DevStart.app'])
            (dmg_root / 'Applications').symlink_to('/Applications')
            dmg = assets / f'DevStart-{version}-macOS-universal.dmg'
            archive = assets / f'DevStart-{version}-macOS-universal.zip'
            run(['hdiutil', 'create', '-volname', 'DevStart', '-srcfolder', dmg_root, '-format', 'UDZO', '-ov', dmg])
            run(['codesign', '--force', '--timestamp', '--keychain', keychain, '--sign', identity, dmg])
            notarize(dmg, authentication, diagnostics)
            run(['xcrun', 'stapler', 'staple', dmg])
            run(['xcrun', 'stapler', 'validate', dmg])
            run(['spctl', '--assess', '--type', 'open', '--context', 'context:primary-signature', '--verbose=2', dmg])
            run(['ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', app, archive])
            run(['hdiutil', 'verify', dmg])
            run(['unzip', '-tq', archive])
            lines = []
            for package in [dmg, archive]:
                with package.open('rb') as source:
                    digest = hashlib.file_digest(source, 'sha256').hexdigest()
                lines.append(f'{digest}  {package.name}\n')
            (assets / 'SHA256SUMS.txt').write_text(''.join(lines))
            print('Signed, notarized, stapled and Gatekeeper-verified packages are ready.', flush=True)
        finally:
            if created:
                run(['security', 'list-keychains', '-d', 'user', '-s', *keychains], allow_failure=True)
                run(['security', 'delete-keychain', keychain], allow_failure=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check-config', action='store_true')
    parser.add_argument('--app', type=Path, default=Path('dist/DevStart.app'))
    parser.add_argument('--assets', type=Path, default=Path('release-assets'))
    arguments = parser.parse_args()
    try:
        if arguments.check_config:
            configuration(os.environ)
            print('Signing and notarization configuration is present.')
        else:
            build(arguments.app.resolve(), arguments.assets.resolve())
    except (ValueError, RuntimeError, OSError) as error:
        raise SystemExit(str(error)) from None
