#!/usr/bin/env python3
"""Render edited native snapshots through CLI/MCP and a real external r4.

Requires numpy and OpenEXR; does not build or modify pbrt-r4.
"""
import hashlib
import json
import os
from pathlib import Path
import select
import struct
import subprocess
import sys
import tempfile
import time

import numpy as np
import OpenEXR

ROOT = Path(__file__).resolve().parents[1]
RENDERER = Path(sys.argv[1]).resolve()

with tempfile.TemporaryDirectory(prefix='pui-project-render-control-') as temporary:
    directory = Path(temporary)
    directory.chmod(0o700)
    endpoint = directory / 'control.sock'
    source = directory / 'scene.pbrt'
    source.write_text('''LookAt 0 1 5 0 0 0 0 1 0
Camera "perspective" "float fov" [45]
Film "rgb" "integer xresolution" [32] "integer yresolution" [32]
Sampler "independent"
Integrator "path" "integer maxdepth" [2]
WorldBegin
LightSource "infinite" "rgb L" [1 1 1]
AttributeBegin
Translate 0 0 0
Material "diffuse" "rgb reflectance" [0.7 0.2 0.2]
Shape "sphere"
AttributeEnd
''')
    host = subprocess.Popen([str(ROOT / 'target/debug/pbrt-ui-host'), '--endpoint', str(endpoint),
                             '--renderer-path', str(RENDERER)], text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            env=dict(os.environ, XDG_CACHE_HOME=str(directory / 'cache')))
    bridge = None

    def call(*args):
        result = subprocess.run([str(ROOT / 'target/debug/pbrt-ui-ctl'), '--endpoint', str(endpoint), *args],
                                text=True, capture_output=True, timeout=30)
        assert result.returncode == 0, result.stdout + result.stderr
        return json.loads(result.stdout)

    def raw(method, params=None):
        return call('raw', method, json.dumps(params or {}))

    def wait(job, expected='succeeded'):
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            result = raw('job.get', {'job_id': job['job_id']})
            if result['state'] in ('succeeded', 'failed', 'cancelled'):
                if result['state'] != expected:
                    for identifier in result['artifacts']:
                        artifact = raw('artifact.get', {'artifact_id': identifier})
                        if artifact['mime'] == 'text/plain':
                            print(Path(artifact['path']).read_text(), file=sys.stderr)
                assert result['state'] == expected, result
                return result
            time.sleep(0.05)
        raise AssertionError('project render timed out')

    def artifacts(job):
        return [raw('artifact.get', {'artifact_id': identifier}) for identifier in job['artifacts']]

    try:
        assert select.select([host.stdout], [], [], 10)[0]
        assert json.loads(host.stdout.readline())['protocol'] == '2'
        wait(call('project', 'import', str(source), str(directory / 'project')))
        raw('render.set_settings', {'expected_settings_revision': '0',
                                   'settings': {'samples_per_pixel': None, 'threads': 1, 'max_depth': None}})
        first = wait(call('project', 'render'))
        before = next(a for a in artifacts(first) if a['mime'] == 'image/x-exr')
        world = call('project', 'nodes')['nodes'][-1]['node_id']
        # Traverse the actual native tree; a transform is an operation node.
        pending = [world]
        transform = None
        while pending:
            parent = pending.pop()
            for node in call('project', 'nodes', parent)['nodes']:
                detail = call('project', 'node', node['node_id'])
                if detail.get('data', {}).get('operation', {}).get('kind') == 'transform':
                    transform = node['node_id']
                pending.append(node['node_id'])
        assert transform
        call('project', 'apply', json.dumps([{'kind': 'set_transform', 'node_id': transform,
                                            'value': {'kind': 'translate', 'value': [1, 0, 0]}}]))
        source.rename(directory / 'moved-source.pbrt')
        second = wait(call('project', 'render'))
        records = artifacts(second)
        exr = next(a for a in records if a['mime'] == 'image/x-exr')
        assert exr['metadata']['project'] == second['project']
        assert exr['metadata']['project']['revision'] == '1'
        assert len(exr['metadata']['render_input_sha256']) == 64
        assert np.abs(OpenEXR.File(before['path']).channels()['RGB'].pixels -
                      OpenEXR.File(exr['path']).channels()['RGB'].pixels).max() > 0
        bundle = directory / 'bundle'
        wait(call('project', 'export', str(bundle)))
        reference = directory / 'reference.exr'
        subprocess.run([str(RENDERER), '--outfile', str(reference), '--spp', '4', '--nthreads', '1',
                        '--maxdepth', '2', str(bundle / 'scene.pbrt')], check=True, capture_output=True, timeout=60)
        assert np.array_equal(OpenEXR.File(str(reference)).channels()['RGB'].pixels,
                              OpenEXR.File(exr['path']).channels()['RGB'].pixels)

        bridge = subprocess.Popen([str(ROOT / 'target/debug/pbrt-ui-mcp'), '--endpoint', str(endpoint)],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        number = 0

        def rpc(method, params):
            global number
            number += 1
            bridge.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': number, 'method': method, 'params': params}) + '\n')
            bridge.stdin.flush()
            assert select.select([bridge.stdout], [], [], 10)[0]
            response = json.loads(bridge.stdout.readline())
            assert 'error' not in response, response
            return response['result']

        rpc('initialize', {'protocolVersion': '2025-11-25', 'clientInfo': {'name': 'native-render-smoke', 'version': '1'}})
        bridge.stdin.write(json.dumps({'jsonrpc': '2.0', 'method': 'notifications/initialized'}) + '\n')
        bridge.stdin.flush()
        tool = next(tool for tool in rpc('tools/list', {})['tools'] if tool['name'] == 'project_render')
        assert 'expected_settings_revision' in tool['inputSchema']['required']
        result = rpc('tools/call', {'name': 'project_render', 'arguments': {
            'project': second['project'], 'expected_settings_revision': '1',
            'view': {'camera': {'eye': [0, 1, 5], 'target': [0, 0, 0], 'up': [0, 1, 0],
                                'projection': 'perspective', 'fov_y_degrees': 45, 'near': 0.01, 'far': 100},
                     'size': [40, 24], 'mode': 'solid', 'time': 0.5}}})
        assert not result['isError'], result
        view_job = wait(result['structuredContent'])
        png = next(a for a in artifacts(view_job) if a['mime'] == 'image/png' and not a['metadata'].get('partial'))
        assert png['metadata']['project'] == second['project']
        assert struct.unpack('>II', Path(png['path']).read_bytes()[16:24]) == (40, 24)
        exported = directory / 'result.png'
        wait(call('export', png['artifact_id'], str(exported)))
        raw('render.set_settings', {'expected_settings_revision': '1',
                                   'settings': {'samples_per_pixel': None, 'threads': 1, 'max_depth': None}})
        arguments = {'project': second['project'], 'expected_settings_revision': '2',
                     'view': {'camera': {'eye': [0, 1, 5], 'target': [0, 0, 0], 'up': [0, 1, 0],
                                         'projection': 'perspective', 'fov_y_degrees': 45, 'near': 0.01, 'far': 100},
                              'size': [192, 128], 'mode': 'solid', 'time': 0.5}}
        progress_job = raw('project.render', arguments)
        try:
            deadline = time.monotonic() + 30
            partial = None
            while time.monotonic() < deadline:
                status = raw('job.get', {'job_id': progress_job['job_id']})
                partial = next((a for a in artifacts(status) if a['metadata'].get('partial')), None)
                if partial or status['state'] in ('succeeded', 'failed', 'cancelled'):
                    break
                time.sleep(0.1)
            assert partial and status['state'] == 'running', status
            assert partial['metadata']['project'] == second['project']
        finally:
            raw('job.cancel', {'job_id': progress_job['job_id']})
            wait(progress_job, expected='cancelled')
        bridge.stdin.close()
        bridge.wait(timeout=5)
        assert bridge.returncode == 0, bridge.stderr.read()
        print(json.dumps({'project_render_control_smoke': 'passed',
                          'renderer_sha256': hashlib.sha256(RENDERER.read_bytes()).hexdigest(),
                          'checks': ['cli_native_render', 'unsaved_transform', 'source_independent',
                                     'snapshot_metadata', 'export_pixel_equality', 'mcp_view_render',
                                     'view_resolution', 'artifact_export', 'partial_snapshot_metadata', 'cancel']}))
    finally:
        if bridge and bridge.poll() is None:
            bridge.terminate()
            bridge.wait(timeout=5)
        host.terminate()
        host.wait(timeout=15)
        assert host.returncode == 0, host.stderr.read()
