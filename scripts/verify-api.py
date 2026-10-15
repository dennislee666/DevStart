#!/usr/bin/env python3
"""Exercise the shipped binary in a disposable workspace; never touch business projects."""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else 'target/release/dev-start').resolve())
checks = []
with tempfile.TemporaryDirectory(prefix='ds-api-', dir='/tmp') as temporary:
    root = Path(temporary)
    data = root / 'data'
    project = root / 'project'
    project.mkdir()
    (project / 'dev.sh').write_text('#!/bin/bash\ntrap "exit 0" TERM\nprintf "API_READY 中文日志\\n"\nwhile true; do sleep 1; done\n')
    env = os.environ | {'DEV_START_DATA_DIR': str(data)}
    endpoint = data / 'api/control.sock'
    log = open(root / 'service.log', 'wb')
    service = None

    def start():
        process = subprocess.Popen([binary, '--serve'], env=env, stdout=log, stderr=log)
        deadline = time.monotonic() + 10
        while not endpoint.exists():
            if process.poll() is not None or time.monotonic() > deadline:
                raise AssertionError('API service did not start')
            time.sleep(.03)
        return process

    def rpc(method, params=None, fail=False):
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(20)
            client.connect(str(endpoint))
            client.sendall((json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': method, 'params': params or {}}) + '\n').encode())
            with client.makefile('rb') as stream:
                result = json.loads(stream.readline())
        if fail:
            assert 'error' in result, result
            return result['error']
        assert 'result' in result, result
        return result['result']

    def until(method, params, predicate):
        deadline = time.monotonic() + 15
        while True:
            result = rpc(method, params)
            if predicate(result):
                return result
            assert time.monotonic() < deadline, result
            time.sleep(.05)

    try:
        service = start()
        assert endpoint.stat().st_mode & 0o777 == 0o600
        assert endpoint.parent.stat().st_mode & 0o777 == 0o700
        with socket.socket(socket.AF_UNIX) as delayed:
            delayed.settimeout(5)
            delayed.connect(str(endpoint))
            time.sleep(.1)
            delayed.sendall(b'{"jsonrpc":"2.0","id":1,"method":"groups.list","params":{}}\n')
            with delayed.makefile('rb') as stream:
                assert 'result' in json.loads(stream.readline())
        schema = rpc('api.describe')
        assert set(schema['methods']) == set(schema['contracts']['methods'])
        checks.append('socket permissions and complete method discovery')
        cli = subprocess.run([binary, '--api', 'groups.list', '{}'], env=env, capture_output=True, check=True)
        assert json.loads(cli.stdout)['result'][0]['name'] == '默认分组'
        bad = subprocess.run([binary, '--api', 'groups.delete', '{"id":1}'], env=env, capture_output=True)
        assert bad.returncode == 1 and 'error' in json.loads(bad.stdout)
        checks.append('CLI success/error and default group')
        prefs = rpc('settings.get')
        assert prefs['theme'] == 'dark' and prefs['console_font_size'] == 12
        rpc('settings.update', {'patch': {'console_font_size': 16, 'sidebar_collapsed': True}})
        assert rpc('settings.get')['console_font_size'] == 16
        rpc('settings.update', {'patch': {'ui_font_size': 99}}, fail=True)
        assert rpc('settings.get')['console_font_size'] == 16
        rpc('settings.reset')
        checks.append('appearance preferences update, validation and reset')
        group = rpc('groups.create', {'name': 'API测试'})['id']
        item = rpc('projects.create', {'path': str(project), 'name': '示例项目', 'description': 'API验证'})
        pid = item['id']
        assert item['group'] == 1
        rpc('projects.update', {'id': pid, 'patch': {'group': group, 'favorite': True}})
        sid = item['scripts'][0]['id']
        rpc('scripts.update', {'id': sid, 'patch': {'name': '本地调试', 'description': '测试说明', 'display_command': 'dev.sh 本地', 'favorite': True}})
        rpc('projects.refresh', {'id': pid})
        assert rpc('scripts.get', {'id': sid})['display_command'] == 'dev.sh 本地'
        manifest = (project / '.dev-start.json').read_bytes()
        rpc('scripts.update', {'id': sid, 'patch': {'file': 'missing.sh'}}, fail=True)
        assert (project / '.dev-start.json').read_bytes() == manifest
        checks.append('project/script edits, source persistence, refresh and rollback')
        source = project / 'dev.sh'
        original_source = source.read_text()
        source.chmod(0o755)
        doc = rpc('editor.open', {'script_id': sid})
        for version in range(12):
            doc = rpc('editor.change', {'script_id': sid, 'expected_revision': doc['revision'], 'content': original_source + f'# edit-version-{version}\n'})
            doc = rpc('editor.save', {'script_id': sid, 'expected_revision': doc['revision']})
        versions = rpc('editor.versions', {'script_id': sid})
        assert len(versions) == 10 and all('content' not in v for v in versions)
        selected = versions[-2]
        doc = rpc('editor.restore', {'script_id': sid, 'expected_revision': doc['revision'], 'version_id': selected['id']})
        assert source.read_text() == doc['content'] and doc['revision'] == selected['revision']
        assert source.stat().st_mode & 0o777 == 0o755
        doc = rpc('editor.change', {'script_id': sid, 'expected_revision': doc['revision'], 'content': original_source + '# unsaved\n'})
        rpc('editor.close', {'script_id': sid, 'expected_revision': doc['revision']}, fail=True)
        source.write_text(original_source + '# external edit\n')
        rpc('editor.save', {'script_id': sid, 'expected_revision': doc['revision']}, fail=True)
        assert rpc('editor.read', {'script_id': sid})['dirty']
        doc = rpc('editor.reload', {'script_id': sid, 'expected_revision': doc['revision'], 'discard': True})
        doc = rpc('editor.restore', {'script_id': sid, 'expected_revision': doc['revision'], 'version_id': selected['id']})
        rpc('editor.close', {'script_id': sid, 'expected_revision': doc['revision']})
        assert not rpc('editor.list')[0]['open']
        assert (project / '.dev-start.json').read_bytes() == manifest
        checks.append('editor save/restore, ten-version ring, executable permissions, dirty-close and external conflict protection, history metadata')
        run_id = rpc('runs.start', {'script_id': sid})['run_id']
        until('runs.get', {'id': run_id}, lambda r: r['state'] == 'running')
        until('logs.read', {'run_id': run_id}, lambda r: 'API_READY' in r['text'])
        before = rpc('runs.get', {'id': run_id})
        assert before['started_at'] and before['started_at_unix_ms'] > 0
        assert before['elapsed_display'].endswith('秒') and ' - ' in before['timing_display']
        checks.append('actual start timestamp and backend day/hour/minute/second duration DTO')

        before_project = rpc('projects.get', {'id': pid})
        rpc('projects.update', {'id': pid, 'patch': {'name': 'example-java'}})
        rpc('groups.update', {'id': group, 'name': '重命名分组'})
        renamed = rpc('projects.get', {'id': pid})
        running = rpc('runs.get', {'id': run_id})
        assert running['pid'] == before['pid'] and running['title'].startswith('example-java / ')
        assert renamed['scripts'] == before_project['scripts'] and renamed['path'] == before_project['path']
        assert (project / '.dev-start.json').read_bytes() == manifest
        rpc('projects.update', {'id': pid, 'patch': {'name': before_project['name']}})
        checks.append('project/group rename during execution preserves PID, source manifest, paths and complete launch specs')

        rpc('scripts.update', {'id': sid, 'patch': {'display_name': 'Java-Starter'}})
        after = rpc('runs.get', {'id': run_id})
        assert after['pid'] == before['pid'] and after['display_command'] == before['display_command']
        assert after['launch_name'] == 'Java-Starter'
        assert (project / '.dev-start.json').read_bytes() == manifest
        rpc('projects.refresh', {'id': pid})
        assert rpc('scripts.get', {'id': sid})['display_name'] == 'Java-Starter'
        rpc('scripts.update', {'id': sid, 'patch': {'display_name': '好' * 51}}, fail=True)
        rpc('scripts.update', {'id': sid, 'patch': {'display_name': None}})
        assert rpc('scripts.get', {'id': sid})['display_name'] is None
        checks.append('live display aliases preserve PID, argv, manifest and survive refresh; 50-character validation and reset')
        found = rpc('logs.search', {'run_id': run_id, 'keyword': 'api_ready', 'limit': 10})
        assert found['source'] == 'console_cache' and found['matched_lines'] >= 1
        assert any('API_READY' in row['text'] for row in found['rows'])
        assert rpc('logs.search', {'run_id': run_id, 'keyword': 'this-keyword-is-absent'})['matched_lines'] == 0
        checks.append('bounded Console keyword search and no-match result')
        rpc('projects.delete', {'id': pid}, fail=True)
        rpc('scripts.delete', {'id': sid}, fail=True)
        run2 = rpc('runs.restart', {'script_id': sid})['run_id']
        assert run2 != run_id
        until('runs.get', {'id': run2}, lambda r: r['state'] == 'running')
        assert not rpc('runs.get', {'id': run_id})['active']
        rpc('runs.stop', {'id': run2})
        until('runs.get', {'id': run2}, lambda r: not r['active'])
        cursor = 0
        combined = bytearray()
        while True:
            page = rpc('logs.read', {'run_id': run2, 'offset': cursor, 'limit': 7})
            combined.extend(page['bytes'])
            cursor = page['next_offset']
            if page['eof']:
                break
        assert combined == (data / 'logs' / f'run-{run2}.log').read_bytes()
        follow = subprocess.run([binary, '--logs', str(run2)], env=env, capture_output=True, check=True)
        assert follow.stdout == combined
        checks.append('checked start/restart/stop, active deletion protection, exact log cursors and CLI follow')
        messages = [
            {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'test', 'version': '1'}}},
            {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
            {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'},
            {'jsonrpc': '2.0', 'id': 3, 'method': 'tools/call', 'params': {'name': 'dev_start', 'arguments': {'method': 'groups.list', 'params': {}}}},
        ]
        mcp = subprocess.run([binary, '--mcp'], input=''.join(json.dumps(v)+'\n' for v in messages).encode(), env=env, capture_output=True, check=True)
        replies = [json.loads(v) for v in mcp.stdout.splitlines()]
        assert len(replies) == 3 and replies[0]['result']['protocolVersion'] == '2025-06-18'
        assert replies[1]['result']['tools'][0]['name'] == 'dev_start'
        assert replies[2]['result']['isError'] is False
        checks.append('MCP initialize, notifications, tool discovery and live invocation')
        added = rpc('scripts.create', {'project_id': pid, 'spec': {'name': '另一个启动项', 'file': 'dev.sh', 'entry': {'File': {'interpreter': '/bin/bash'}}, 'description': '只配置不启动', 'enabled': False, 'args': ['--port=8123']}})['id']
        port_info = rpc('scripts.ports', {'id': added})
        assert port_info['configured_port'] == 8123 and port_info['actual_ports'] == []
        assert port_info['source'] == 'configured' and port_info['display'] == '8123'
        checks.append('prelaunch declared port and observation provenance')
        rpc('scripts.delete', {'id': added})
        rpc('groups.delete', {'id': group})
        assert rpc('projects.get', {'id': pid})['group'] == 1
        rpc('projects.delete', {'id': pid})
        assert (project / 'dev.sh').exists() and (project / '.dev-start.json').exists()
        service.terminate()
        service.wait(timeout=10)
        assert not endpoint.exists()
        service = start()
        assert rpc('logs.list')['logs']
        assert rpc('logs.read', {'run_id': run2})['bytes']
        checks.append('non-destructive removal, group reassignment, graceful shutdown and historical logs after restart')
    except Exception:
        log.flush()
        print((root / 'service.log').read_text(errors='replace'), file=sys.stderr)
        raise
    finally:
        if service and service.poll() is None:
            service.terminate()
            service.wait(timeout=10)
        log.close()

report = {'result': 'passed', 'checks': checks, 'methods': schema['methods'], 'scope': 'isolated workspace; no business project execution'}
print(json.dumps(report, ensure_ascii=False, indent=2))
