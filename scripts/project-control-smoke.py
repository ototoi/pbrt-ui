#!/usr/bin/env python3
"""Exercise native project operations through the real host, CLI and MCP bridge."""
import json
import os
import base64
from pathlib import Path
import re
import select
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='pui-project-control-') as temporary:
    directory = Path(temporary)
    directory.chmod(0o700)
    endpoint = directory / 'control.sock'
    source = directory / 'source.pbrt'
    source.write_text('WorldBegin\nTranslate 1 0 0\nShape "sphere" "float radius" [1]\n')
    project = directory / 'project'
    bundle = directory / 'bundle'
    host = subprocess.Popen([str(ROOT / 'target/debug/pbrt-ui-host'), '--endpoint', str(endpoint)],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                            env=dict(os.environ, XDG_CACHE_HOME=str(directory / 'cache')))
    bridge = None
    try:
        assert select.select([host.stdout], [], [], 10)[0]
        assert json.loads(host.stdout.readline())['protocol'] == '2'

        def call(*args, success=True):
            result = subprocess.run([str(ROOT / 'target/debug/pbrt-ui-ctl'), '--endpoint', str(endpoint), *args],
                                    capture_output=True, text=True, timeout=20)
            if success:
                assert result.returncode == 0, result.stdout + result.stderr
                return json.loads(result.stdout)
            assert result.returncode != 0
            return json.loads(result.stderr)

        job = call('project', 'import', str(source), str(project), '--wait')
        assert job['state'] == 'succeeded'
        initial = call('project', 'state')
        world = call('project', 'nodes')['nodes'][0]['node_id']
        # The import resolves the world-leading Translate into a transform
        # scope ("World Transform"); the state op and the shape nest inside.
        scope = call('project', 'nodes', world)['nodes'][0]['node_id']
        children = call('project', 'nodes', scope)['nodes']
        transform, shape = children[0]['node_id'], children[1]['node_id']
        edits = [{'kind': 'set_transform', 'node_id': transform,
                  'value': {'kind': 'translate', 'value': [2, 3, 4]}},
                 {'kind': 'rename_node', 'node_id': shape, 'label': 'Edited sphere'}]
        changed = call('project', 'apply', json.dumps(edits))
        assert call('project', 'state')['dirty'] is True
        assert call('project', 'close', success=False)['error']['code'] == 'unsaved_changes'
        old = call('raw', 'project.undo', json.dumps({'project': initial['context']['project']}), success=False)
        assert old['error']['code'] == 'revision_conflict'
        call('project', 'undo')
        call('project', 'redo')
        save = call('project', 'save', '--wait')
        assert save['state'] == 'succeeded', save
        # The save's receipt reports the content
        # id it published - the anchor dirty compares against.
        assert re.fullmatch('[0-9a-f]{64}', save['result']['content_id']), save['result']
        assert call('project', 'state')['dirty'] is False

        # Dirty follows content, not the communication revision:
        # one edit turns dirty, and undoing back to the just-saved content
        # is clean at a higher revision.
        saved = call('project', 'state')['context']['project']['revision']
        call('project', 'apply', json.dumps(
            [{'kind': 'rename_node', 'node_id': shape, 'label': 'D2 toggle'}]))
        assert call('project', 'state')['dirty'] is True
        call('project', 'undo')
        state = call('project', 'state')
        assert state['dirty'] is False, 'undo returned the project to the saved content'
        assert state['context']['project']['revision'] != saved

        manifest = json.loads((project / 'project.pbrtui.json').read_text())
        assert manifest['format'] == 'pbrt-ui.project/3'
        source.rename(directory / 'moved-source.pbrt')
        exported = call('project', 'export', str(bundle), '--wait')
        assert exported['state'] == 'succeeded'
        assert 'Translate 2 3 4' in (bundle / 'scene.pbrt').read_text()
        assert exported['result']['warnings'] == [], exported
        # An imagemap without an image nothing uses is left out with a warning.
        pending = call('project', 'apply', json.dumps([
            {'kind': 'create_resource', 'resource_kind': 'texture', 'implementation': 'imagemap',
             'label': 'Pending', 'texture_value_type': 'spectrum', 'parameters': []}]))['created'][0]['id']
        skipped = call('project', 'export', str(directory / 'pending-bundle'), '--wait')
        assert skipped['state'] == 'succeeded', skipped
        assert [(w['code'], w['resource_id']) for w in skipped['result']['warnings']] == [
            ('imagemap_without_image_skipped', pending)], skipped
        assert 'imagemap' not in (directory / 'pending-bundle' / 'scene.pbrt').read_text()
        call('project', 'undo')

        presets = call('project', 'presets')['presets']
        assert len(presets) == 15 and all(preset['builtin'] for preset in presets)
        created = call('project', 'create', 'shape.cube')['created']
        assert len(created) == 2 and all(item['kind'] == 'node' for item in created)
        # A parameter's values page past the parameter preview's 32.
        cube = next(item['id'] for item in created
                    if call('project', 'node', item['id']).get('data', {}).get('implementation') == 'trianglemesh')
        points = call('project', 'values', 'node', cube, 'P', '3', '--limit', '6')
        assert points['value_type'] == 'point3' and points['offset'] == 3, points
        assert points['values']['kind'] == 'floats' and len(points['values']['value']) == 6, points
        assert int(points['total']) >= 9 and points['next_offset'] == 9, points
        # Windows of values change as one job (undoable), length unchanged.
        changed = call('project', 'set-values', 'node', cube, 'P',
                       json.dumps([{'offset': 0, 'values': {'kind': 'floats', 'value': [0.25]}},
                                   {'offset': 4, 'values': {'kind': 'floats', 'value': [0.5, 0.75]}}]), '--wait')
        assert changed['state'] == 'succeeded', changed
        after = call('project', 'values', 'node', cube, 'P', '0', '--limit', '6')['values']['value']
        assert [v['value'] for v in after[:1] + after[4:6]] == [0.25, 0.5, 0.75], after
        call('project', 'undo')
        scene_root = call('project', 'nodes', '--view', 'scene_graph')['nodes']
        assert [node['object'] for node in scene_root] == ['scene']
        # The world's objects sit directly under the scene: the imported
        # Translate resolved into a leading group, then the created cube.
        world_objects = call('project', 'nodes', scene_root[0]['node_id'], '--view', 'scene_graph')['nodes']
        assert [node['object'] for node in world_objects] == ['group', 'shape'], world_objects
        imported = call('project', 'nodes', world_objects[0]['node_id'], '--view', 'scene_graph')['nodes']
        assert [node['node_id'] for node in imported] == [shape], imported
        assert imported[0]['components'][:2] == ['transform', 'shape']
        assert world_objects[1]['node_id'] == cube, world_objects
        cube = call('project', 'node', created[1]['id'])
        assert cube['components'][0]['shared_with'] == 0
        assert cube['components'][1]['statistics'] == {'vertices': 8, 'faces': 12}
        hit = call('project', 'search', 'Cube', '--view', 'scene_graph')['nodes'][0]
        assert hit['path'][-1] == scene_root[0]['node_id']
        call('project', 'undo')

        # The regenerable cache: written by core, read by path.
        entry = directory / 'entry.bin'
        entry.write_bytes(b'cache bytes')
        item = call('project', 'state')['context']['project']['project_id']
        put = call('project', 'cache-put', item, 'smoke-key', 'thumbnail', 'application/octet-stream', str(entry))
        assert str(directory) not in put['path'] or 'Library/Cache' in put['path'], put
        cached = call('project', 'cache-get', item, 'smoke-key')
        assert cached['hit'] is True and Path(cached['path']).read_bytes() == b'cache bytes'
        assert call('project', 'cache-get', item, 'missing-key')['hit'] is False

        # The static r4 parameter table: defaults and per-type textures.
        sphere = call('project', 'schema', 'Shape', 'sphere')
        radius = next(field for field in sphere['parameters'] if field['name'] == 'radius')
        assert radius['default'] == {'value_type': 'float', 'values': [1.0]}, radius
        zmin = next(field for field in sphere['parameters'] if field['name'] == 'zmin')
        assert zmin['default_depends'] == {'parameter': 'radius', 'operation': 'negated'}
        mix_spectrum = call('project', 'schema', 'Texture', 'mix', 'spectrum')
        tex1 = next(field for field in mix_spectrum['parameters'] if field['name'] == 'tex1')
        assert tex1['default']['values'] == [1.0, 1.0, 1.0], tex1
        assert tex1['group'] is None and tex1['when'] == [], tex1
        # The 2D mapping rows are their own group, each naming when r4 reads it.
        checker = call('project', 'schema', 'Texture', 'checkerboard', 'spectrum')
        uscale = next(field for field in checker['parameters'] if field['name'] == 'uscale')
        assert uscale['group'] == 'mapping', uscale
        assert uscale['when'] == [{'parameter': 'dimension', 'values': ['2']},
                                  {'parameter': 'mapping', 'values': ['uv']}], uscale
        assert uscale['choices'] == [], uscale
        mode = next(field for field in checker['parameters'] if field['name'] == 'mapping')
        assert mode['choices'] == ['uv', 'spherical', 'cylindrical', 'planar'], mode
        imagemap = call('project', 'schema', 'Texture', 'imagemap', 'spectrum')
        wrap = next(field for field in imagemap['parameters'] if field['name'] == 'wrap')
        assert wrap['choices'] == ['repeat', 'black', 'clamp', 'octahedralsphere'], wrap
        encoding = next(field for field in imagemap['parameters'] if field['name'] == 'encoding')
        assert encoding['choices'] == [] and encoding['suggestions'] == ['sRGB', 'linear', 'gamma 2.2'], encoding
        assert call('project', 'schema', 'Shape', 'cone')['parameters'] is None

        # Create a resource through an edit using the parameter table's
        # defaults and shared placement, then undo the edit.
        created = call('project', 'apply', json.dumps([
            {'kind': 'create_resource', 'resource_kind': 'material',
             'implementation': 'diffuse', 'label': 'Smoke'}]))['created']
        material = next(item for item in created if item['kind'] == 'resource')
        materials = call('project', 'contents', 'Materials')
        row = next(entry for entry in materials['entries'] if entry['id'] == material['id'])
        assert row['location'] == 'Materials/Smoke.material', row
        parameters = call('project', 'parameters', 'resource', material['id'])['parameters']
        reflectance = next(p for p in parameters if p['name'] == 'reflectance')
        assert reflectance['values']['kind'] == 'floats'
        assert len(reflectance['values']['value']) == 3
        call('project', 'undo')
        materials_after = call('project', 'contents', 'Materials')
        assert not any(entry['id'] == material['id'] for entry in materials_after['entries'])

        # Component edits: an added area light declares L [1 1 1] plus the
        # table's defaults, reverse orientation lands in a
        # dedicated group, and a group binding applies below it, then clears.
        call('project', 'apply', json.dumps([
            {'kind': 'set_area_light', 'node_id': shape,
             'light': {'implementation': 'diffuse'}}]))
        scene = call('project', 'export', str(project / 'g4-bundle'), '--wait')
        assert scene['state'] == 'succeeded', scene
        text = (project / 'g4-bundle' / 'scene.pbrt').read_text()
        assert 'AreaLightSource "diffuse"' in text and '[ 1 1 1 ]' in text, text
        call('project', 'apply', json.dumps([
            {'kind': 'set_reverse_orientation', 'node_id': shape, 'enabled': True}]))
        detail = call('project', 'node', shape)
        reverse = next(c for c in detail['components'] if c['kind'] == 'reverse_orientation')
        assert reverse['enabled'] is True and reverse['local'] is True, detail
        call('project', 'undo')
        # An object's local TRS is read back as sent.
        call('project', 'apply', json.dumps([
            {'kind': 'set_local_trs', 'node_id': shape, 'translation': [1, 2, 3],
             'rotation_euler_degrees': [0, 90, 0], 'scale': [2, 2, 2]}]))
        local = call('project', 'node', shape)['components'][0]['local']
        assert local['parent'] == 'group', local
        assert [round(v, 9) for v in local['trs']['rotation_euler_degrees']] == [0, 90, 0], local
        assert [round(v, 9) for v in local['trs']['scale']] == [2, 2, 2], local
        call('project', 'undo')
        call('project', 'undo')
        error = call('project', 'apply', json.dumps([
            {'kind': 'clear_local_binding', 'node_id': shape,
             'material': False, 'medium': False}]), success=False)
        assert error['error']['code'] == 'invalid_request', error

        # Preview an implementation change, apply its edits, and verify the
        # export; one undo reverts the change.
        plan = call('project', 'plan-implementation', 'node', shape, 'disk')
        assert plan['changed'] is True and plan['implementation'] == 'disk', plan
        assert plan['kept_unknown'] == [] and plan['breaks_reference'] == [], plan
        applied = call('project', 'apply', json.dumps(plan['edits']))
        assert applied['kept_unknown'] == [], applied
        scene = call('project', 'export', str(project / 'g5-bundle'), '--wait')
        assert scene['state'] == 'succeeded', scene
        assert 'Shape "disk"' in (project / 'g5-bundle' / 'scene.pbrt').read_text()
        assert call('project', 'plan-implementation', 'node', shape, 'disk')['edits'] == []
        refused = call('project', 'apply', json.dumps([
            {'kind': 'set_implementation', 'target': {'kind': 'node', 'id': shape},
             'implementation': 'cone2', 'parameters': 'keep_compatible'}]), success=False)
        assert refused['error']['details']['reason'] == 'unsupported_implementation', refused
        call('project', 'undo')
        assert call('project', 'plan-implementation', 'node', shape, 'disk')['changed'] is True

        # A material's upstream graph through the CLI: a texture
        # wired into reflectance shows as a node and an edge — the spectrum
        # names, and the Alpha Mask component; all undone.
        created = call('project', 'apply', json.dumps([
            {'kind': 'create_resource', 'resource_kind': 'texture', 'implementation': 'constant',
             'label': 'Tone', 'texture_value_type': 'spectrum'},
            {'kind': 'create_resource', 'resource_kind': 'material', 'implementation': 'diffuse',
             'label': 'Graphed'}]))['created']
        tone, graphed = [item['id'] for item in created if item['kind'] == 'resource']
        call('project', 'apply', json.dumps([
            {'kind': 'set_parameter', 'target': {'kind': 'resource', 'id': graphed},
             'name': 'reflectance', 'value_type': 'rgb', 'value': None},
            {'kind': 'set_parameter', 'target': {'kind': 'resource', 'id': graphed},
             'name': 'reflectance', 'value_type': 'texture',
             'value': {'name': 'reflectance', 'value_type': 'texture', 'color_space': None,
                       'values': {'kind': 'resources', 'value': {'kind': 'texture', 'ids': [tone]}}}}]))
        graph = call('project', 'graph', graphed)
        # The constant's inline `value` joins the graph as a spectrum_value
        # node: the material, the texture, then its RGB value.
        assert [node['id'] for node in graph['nodes']] == [graphed, tone, f'{tone}:value'], graph
        assert graph['edges'] == [
            {'from': tone, 'to': graphed, 'parameter': 'reflectance'},
            {'from': f'{tone}:value', 'to': tone, 'parameter': 'value'},
        ], graph
        assert graph['truncated'] is False
        assert call('project', 'graph', graphed, '1')['truncated'] is True
        assert 'metal-Cu-eta' in call('project', 'spectra')['spectra']
        call('project', 'apply', json.dumps([
            {'kind': 'set_parameter', 'target': {'kind': 'node', 'id': shape}, 'name': 'alpha',
             'value_type': 'float', 'value': {'name': 'alpha', 'value_type': 'float', 'color_space': None,
                                              'values': {'kind': 'floats', 'value': [0.5]}}}]))
        mask = next(c for c in call('project', 'node', shape)['components'] if c['kind'] == 'alpha_mask')
        assert mask['declared'] == [{'name': 'alpha', 'value_type': 'float'}], mask
        # The alpha mask takes a float texture like a material parameter.
        connected = call('project', 'apply', json.dumps([
            {'kind': 'connect_texture', 'target': {'kind': 'node', 'id': shape}, 'parameter': 'alpha',
             'create': {'implementation': 'constant', 'label': 'Mask'}}]))
        masked_by = next(item['id'] for item in connected['created'] if item['kind'] == 'resource')
        alpha = next(p for p in call('project', 'parameters', 'node', shape)['parameters']
                     if p['name'] == 'alpha')
        assert alpha['value_type'] == 'texture' and alpha['values']['ids'] == [masked_by], alpha
        # The desktop's Disconnect on a mask row: back to opaque, mask kept.
        call('project', 'apply', json.dumps([
            {'kind': 'disconnect_texture', 'target': {'kind': 'node', 'id': shape}, 'parameter': 'alpha'},
            {'kind': 'set_parameter', 'target': {'kind': 'node', 'id': shape}, 'name': 'alpha',
             'value_type': 'float', 'value': {'name': 'alpha', 'value_type': 'float', 'color_space': None,
                                              'values': {'kind': 'floats', 'value': [1.0]}}}]))
        mask = next(c for c in call('project', 'node', shape)['components'] if c['kind'] == 'alpha_mask')
        assert mask['declared'] == [{'name': 'alpha', 'value_type': 'float'}], mask
        for _ in range(2):
            call('project', 'undo')
        for _ in range(3):
            call('project', 'undo')

        # Connect a new texture into a material in one edit, see
        # the graph follow, refuse a cycle with its reason, disconnect.
        created = call('project', 'apply', json.dumps([
            {'kind': 'create_resource', 'resource_kind': 'material', 'implementation': 'diffuse',
             'label': 'Wired'}]))['created']
        wired = next(item['id'] for item in created if item['kind'] == 'resource')
        target = {'kind': 'resource', 'id': wired}
        applied = call('project', 'apply', json.dumps([
            {'kind': 'connect_texture', 'target': target, 'parameter': 'reflectance',
             'create': {'implementation': 'scale', 'label': 'Scaled'}}]))
        scaled = next(item['id'] for item in applied['created'] if item['kind'] == 'resource')
        graph = call('project', 'graph', wired)
        assert graph['edges'] == [{'from': scaled, 'to': wired, 'parameter': 'reflectance'}], graph
        refused = call('project', 'apply', json.dumps([
            {'kind': 'connect_texture', 'target': {'kind': 'resource', 'id': scaled},
             'parameter': 'tex', 'texture': scaled}]), success=False)
        assert refused['error']['details']['reason'] == 'resource_cycle', refused
        lineage = call('project', 'apply', json.dumps([
            {'kind': 'connect_texture', 'target': {'kind': 'resource', 'id': scaled},
             'parameter': 'scale', 'texture': scaled}]), success=False)
        assert lineage['error']['details']['reason'] == 'texture_lineage', lineage
        # The scale texture's preview is cached until an input
        # changes.
        preview = call('project', 'texture-preview', scaled, '32')
        assert preview['cached'] is False and Path(preview['path']).is_file(), preview
        assert call('project', 'texture-preview', scaled, '32')['cached'] is True
        call('project', 'apply', json.dumps([
            {'kind': 'set_parameter', 'target': {'kind': 'resource', 'id': scaled}, 'name': 'scale',
             'value_type': 'float', 'value': {'name': 'scale', 'value_type': 'float', 'color_space': None,
                                              'values': {'kind': 'floats', 'value': [0.25]}}}]))
        edited = call('project', 'texture-preview', scaled, '32')
        assert edited['cached'] is False and edited['key'] != preview['key'], edited
        call('project', 'undo')
        call('project', 'apply', json.dumps([
            {'kind': 'disconnect_texture', 'target': target, 'parameter': 'reflectance'}]))
        assert call('project', 'graph', wired)['edges'] == []
        for _ in range(3):
            call('project', 'undo')

        # The material's sphere preview through the CLI is
        # written once, cached on repeats, rebuilt when an upstream
        # texture changes.
        created = call('project', 'apply', json.dumps([
            {'kind': 'create_resource', 'resource_kind': 'material', 'implementation': 'diffuse',
             'label': 'Sphere'}]))['created']
        sphere_material = next(item['id'] for item in created if item['kind'] == 'resource')
        applied = call('project', 'apply', json.dumps([
            {'kind': 'connect_texture', 'target': {'kind': 'resource', 'id': sphere_material},
             'parameter': 'reflectance', 'create': {'implementation': 'scale', 'label': 'Hue'}}]))
        hue = next(item['id'] for item in applied['created'] if item['kind'] == 'resource')
        sphere = call('project', 'material-preview', sphere_material, '32')
        assert sphere['cached'] is False and Path(sphere['path']).is_file(), sphere
        assert call('project', 'material-preview', sphere_material, '32')['cached'] is True
        call('project', 'apply', json.dumps([
            {'kind': 'set_parameter', 'target': {'kind': 'resource', 'id': hue}, 'name': 'scale',
             'value_type': 'float', 'value': {'name': 'scale', 'value_type': 'float', 'color_space': None,
                                              'values': {'kind': 'floats', 'value': [0.25]}}}]))
        rebuilt = call('project', 'material-preview', sphere_material, '32')
        assert rebuilt['cached'] is False and rebuilt['key'] != sphere['key'], rebuilt
        for _ in range(3):
            call('project', 'undo')


        # The where-used list through the CLI: a bound material
        # names its holding object with its via and its reveal path, a
        # connected texture names the material through its parameter, a
        # scene answers empty (the model carries no such links), and the
        # delete refusal repeats the same rows.
        created = call('project', 'apply', json.dumps([
            {'kind': 'create_resource', 'resource_kind': 'material',
             'implementation': 'diffuse', 'label': 'Smoker'}]))['created']
        smoker = next(item['id'] for item in created if item['kind'] == 'resource')
        call('project', 'apply', json.dumps([
            {'kind': 'set_local_binding', 'node_id': shape,
             'material': {'kind': 'resource', 'value': smoker}, 'medium': None}]))
        applied = call('project', 'apply', json.dumps([
            {'kind': 'connect_texture', 'target': {'kind': 'resource', 'id': smoker},
             'parameter': 'reflectance',
             'create': {'implementation': 'scale', 'label': 'Hazed'}}]))
        hazed = next(item['id'] for item in applied['created'] if item['kind'] == 'resource')
        bound = call('project', 'list-references', 'resource', smoker)
        node_row = next(row for row in bound['references'] if row['kind'] == 'node')
        assert node_row['via'] == 'material' and node_row['node']['object'] == 'shape', node_row
        assert node_row['scene']['label'] and node_row['node']['path'], node_row
        hazed_rows = call('project', 'list-references', 'resource', hazed)
        assert hazed_rows['total'] == 1 and hazed_rows['references'][0]['kind'] == 'resource', hazed_rows
        assert hazed_rows['references'][0]['via'] == 'parameter:reflectance', hazed_rows
        active_scene = call('project', 'state')['active_scene']
        assert call('project', 'list-references', 'scene', active_scene)['total'] == 0
        assert call('project', 'list-references', 'resource', smoker,
                    '0')['total'] == bound['total']
        refused = call('project', 'apply', json.dumps([
            {'kind': 'delete_item', 'item': {'kind': 'resource', 'id': smoker}}]), success=False)
        details = refused['error']['details']
        assert details['reason'] == 'referenced', refused
        assert any(row['via'] == 'material' for row in details['references']['items']), details


        bridge = subprocess.Popen([str(ROOT / 'target/debug/pbrt-ui-mcp'), '--endpoint', str(endpoint)],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        number = 0

        def rpc(method, params):
            global number
            number += 1
            bridge.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': number, 'method': method, 'params': params}) + '\n')
            bridge.stdin.flush()
            assert select.select([bridge.stdout], [], [], 10)[0]
            reply = json.loads(bridge.stdout.readline())
            assert 'error' not in reply, reply
            return reply['result']

        rpc('initialize', {'protocolVersion': '2025-11-25', 'clientInfo': {'name': 'project-smoke', 'version': '1'}})
        bridge.stdin.write(json.dumps({'jsonrpc': '2.0', 'method': 'notifications/initialized'}) + '\n')
        bridge.stdin.flush()
        tools = rpc('tools/list', {})['tools']
        apply_tool = next(tool for tool in tools if tool['name'] == 'project_apply')
        assert any(tool['name'] == 'project_list_presets' for tool in tools)
        assert any(tool['name'] == 'project_search_nodes' for tool in tools)
        assert any(tool['name'] == 'project_get_cache' for tool in tools)
        assert any(tool['name'] == 'project_put_cache' for tool in tools)
        assert any(tool['name'] == 'project_get_parameter_schema' for tool in tools)
        assert any(tool['name'] == 'project_plan_implementation' for tool in tools)
        assert any(tool['name'] == 'project_get_resource_graph' for tool in tools)
        assert any(tool['name'] == 'project_list_spectra' for tool in tools)
        assert any(tool['name'] == 'project_list_references' for tool in tools)
        assert apply_tool['inputSchema']['properties']['edits']['maxItems'] == 64
        current = rpc('tools/call', {'name': 'project_get_state', 'arguments': {}})
        assert not current['isError']
        reference = current['structuredContent']['context']['project']
        edited = rpc('tools/call', {'name': 'project_apply', 'arguments': {'project': reference,
                     'edits': [{'kind': 'rename_node', 'node_id': shape, 'label': 'Edited through MCP'}]}})
        assert not edited['isError'], edited
        assert call('project', 'node', shape)['label'] == 'Edited through MCP'
        reference = rpc('tools/call', {'name': 'project_get_state', 'arguments': {}})['structuredContent']['context']['project']
        planned = rpc('tools/call', {'name': 'project_plan_implementation', 'arguments': {
            'project': reference, 'target': {'kind': 'node', 'id': shape}, 'implementation': 'disk'}})
        assert not planned['isError'], planned
        assert planned['structuredContent']['edits'][0]['kind'] == 'set_implementation', planned
        listed = rpc('tools/call', {'name': 'project_list_references', 'arguments': {
            'project': reference, 'item': {'kind': 'resource', 'id': smoker}}})
        assert not listed['isError'], listed
        assert any(row['via'] == 'material'
                   for row in listed['structuredContent']['references']), listed
        bridge.stdin.close()
        bridge.wait(timeout=5)
        assert bridge.returncode == 0, bridge.stderr.read()

        # A PLY asset reports its header statistics, and the
        # plymesh Shape its topology, through the CLI.
        (directory / 'tri.ply').write_text(
            'ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\n'
            'property float z\nproperty float u\nproperty float v\nelement face 1\n'
            'property list uchar int vertex_indices\nend_header\n'
            '0 0 0 0 0\n1 0 0 1 0\n0 1 0 0 1\n3 0 1 2\n')
        meshed = directory / 'meshed.pbrt'
        meshed.write_text('WorldBegin\nShape "plymesh" "string filename" "tri.ply"\n')
        job = call('project', 'import', str(meshed), str(directory / 'meshed'), '--wait', '--discard-unsaved')
        assert job['state'] == 'succeeded', job
        ply = next(entry for entry in call('project', 'contents', 'Meshes')['entries']
                   if entry['media_type'] == 'application/x-ply')
        assert call('project', 'asset', ply['id'])['mesh'] == {
            'vertices': 3, 'faces': 1, 'normals': False, 'uvs': True, 'format': 'ascii'}
        world = next(node['node_id'] for node in call('project', 'nodes')['nodes'] if node['label'] == 'World')
        mesh_shape = next(node['node_id'] for node in call('project', 'nodes', world)['nodes']
                          if node['kind'] == 'entity')
        body = next(c for c in call('project', 'node', mesh_shape)['components'] if c['kind'] == 'shape')
        assert body['statistics']['vertices'] == 3 and body['statistics']['faces'] == 1, body
        # Close a scene without discarding edits; resource queries keep working.
        before = call('project', 'state')
        current_scene = before['active_scene']
        closed = call('project', 'close-scene')
        assert closed['active_scene'] is None and closed['scene_count'] == 1, closed
        assert closed['context']['project']['revision'] == before['context']['project']['revision']
        assert call('project', 'contents', 'Meshes')['entries']
        assert call('project', 'asset', ply['id'])['mesh']['vertices'] == 3
        rejected = call('project', 'nodes', success=False)
        assert rejected['error']['details']['reason'] == 'no_active_scene', rejected
        reopened = call('project', 'open-scene', current_scene)
        assert reopened['active_scene'] == current_scene
        deleted = call('project', 'apply', json.dumps([{'kind': 'delete_scene', 'scene': current_scene}]))
        empty = call('project', 'state')
        assert empty['scene_count'] == 0 and empty['active_scene'] is None, empty
        assert call('project', 'contents', 'Meshes')['entries']
        call('project', 'undo')
        restored = call('project', 'state')
        assert restored['scene_count'] == 1 and restored['active_scene'] is None, restored
        call('project', 'open-scene', current_scene)
        call('project', 'redo')
        assert call('project', 'state')['scene_count'] == 0
        created = call('project', 'apply', json.dumps([{'kind':'create_resource','resource_kind':'texture','implementation':'constant','label':'TypeCheck','texture_value_type':'spectrum','parameters':[]}]))['created']
        texture = next(item['id'] for item in created if item['kind'] == 'resource')
        call('project', 'apply', json.dumps([{'kind':'set_texture_value_type','resource_id':texture,'value':'float'}]))
        assert call('project', 'resource', texture)['texture_value_type'] == 'float'
        call('project', 'undo')
        assert call('project', 'resource', texture)['texture_value_type'] == 'spectrum'
        call('project', 'redo')
        # Add scenes and standalone resources to the open project.
        # The CLI's own help advertises the new commands and --folder option.
        usage = json.loads(subprocess.run(
            [str(ROOT / 'target/debug/pbrt-ui-ctl'), '--help'],
            capture_output=True, text=True).stdout)
        assert 'import-scene SOURCE [--folder FOLDER]' in usage['usage'], usage['usage']
        assert '--folder FOLDER' in usage['query_options'], usage
        assert 'capture [--scene ID] [--camera scene|view] [--debug-albedo|--debug-ao]' in usage['usage'], usage
        # Capture accepts --camera like render; pick does not.
        rejected = call('project', 'capture', '--camera', 'bogus', success=False)
        assert rejected['error']['code'] == 'invalid_request', rejected
        rejected = call('project', 'pick', '0.5', '0.5', '--camera', 'scene', success=False)
        assert rejected['error']['code'] == 'client_error', rejected
        rejected = call('project', 'state', '--debug-albedo', success=False)
        assert rejected['error']['code'] == 'client_error', rejected
        assert '--debug-albedo applies to project capture' in rejected['error']['message'], rejected
        rejected = call('project', 'state', '--debug-ao', success=False)
        assert '--debug-ao applies to project capture' in rejected['error']['message'], rejected

        # --mode/--size/--time configure the shared view, and a
        # project view result round-trips as the JSON argument.
        call('project', 'view', '--mode', 'lit', '--size', '976x1280')
        shared = call('project', 'view')
        assert shared['view']['mode'] == 'lit', shared
        assert shared['view']['size'] == [976, 1280], shared
        kept = call('project', 'view', json.dumps(shared))
        assert kept['view']['camera'] == shared['view']['camera'], kept

        # The project is scene-less and dirty here; imports must work in that
        # state, publish as dirty, and never auto-open the added scene.
        added_scene = directory / 'added.pbrt'
        added_scene.write_text('WorldBegin\nShape "sphere" "float radius" [1]\nWorldEnd\n')
        imported = call('project', 'import-scene', str(added_scene), '--wait')
        assert imported['state'] == 'succeeded', imported
        assert [item['kind'] for item in imported['result']['created']] == ['scene']
        imported_scene = imported['result']['created'][0]
        assert imported_scene['location'] == 'Scenes/added.scene', imported_scene
        state = call('project', 'state')
        assert state['scene_count'] == 1, state
        assert state['active_scene'] is None, 'added scenes are not opened automatically'
        assert state['dirty'] is True, 'publication does not save the project'
        # The same input again: distinct ids, case-insensitive name suffix.
        second = call('project', 'import-scene', str(added_scene), '--wait')
        assert second['state'] == 'succeeded', second
        assert second['result']['created'][0]['location'] == 'Scenes/added_1.scene'
        assert second['result']['created'][0]['id'] != imported_scene['id']
        # An invalid folder is a synchronous rejection, not a cancelled job.
        rejected = call('project', 'import-scene', str(added_scene), '--folder', 'Missing', success=False)
        assert rejected['error']['code'] == 'invalid_request', rejected
        # Images import as one image asset in the named folder, with no
        # Texture; a resource import without a folder is refused up front.
        wood = directory / 'wood.png'
        wood.write_bytes(base64.b64decode(
            'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=='))
        folderless = call('project', 'import-resource', str(wood), success=False)
        assert folderless['error']['code'] == 'invalid_request', folderless
        resources_before = call('project', 'state')['resource_count']
        image = call('project', 'import-resource', str(wood), '--folder', 'Images', '--wait')
        assert image['state'] == 'succeeded', image
        assert [item['kind'] for item in image['result']['created']] == ['asset'], image
        assert image['result']['created'][0]['location'] == 'Images/wood.png'
        assert call('project', 'state')['resource_count'] == resources_before
        images = call('project', 'contents', 'Images')['entries']
        wood_entry = next(entry for entry in images if entry['id'] == image['result']['created'][0]['id'])
        assert wood_entry['kind'] == 'asset' and wood_entry['location'] == 'Images/wood.png', wood_entry
        assert not any(entry['location'].endswith('.meta') for entry in images)
        # Connecting the image makes an imagemap of the row's lineage once,
        # then reuses it for another row.
        woods = [item['id'] for item in call('project', 'apply', json.dumps([
            {'kind': 'create_resource', 'resource_kind': 'material', 'implementation': 'diffuse',
             'label': label} for label in ('Wood', 'Oak')]))['created']]
        wood_id = wood_entry['id']
        def connect_wood(material):
            return call('project', 'apply', json.dumps([
                {'kind': 'connect_texture', 'target': {'kind': 'resource', 'id': material},
                 'parameter': 'reflectance', 'image': wood_id}]))['created']
        made = connect_wood(woods[0])
        assert [item['kind'] for item in made] == ['resource'], made
        texture = call('project', 'resource', made[0]['id'])
        assert (texture['implementation'], texture['label'], texture['texture_value_type']) == (
            'imagemap', 'wood', 'spectrum'), texture
        assert connect_wood(woods[1]) == []
        for _ in range(3):
            call('project', 'undo')
        assert call('project', 'state')['resource_count'] == resources_before
        # A rejected-to-import entry fails as a job with an unpublished reason.
        broken = directory / 'broken.xyz'
        broken.write_bytes(b'not a resource')
        resources_before = call('project', 'state')['resource_count']
        refused = call('project', 'import-resource', str(broken), '--folder', 'Other', '--wait', success=False)['error']['details']
        assert refused['published'] is False and refused['reason'] == 'invalid_request', refused
        assert call('project', 'state')['resource_count'] == resources_before
        # PLY imports become mesh assets in Meshes without touching scenes.
        toy = directory / 'toy.ply'
        toy.write_text('ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\n'
                       'property float y\nproperty float z\nelement face 1\n'
                       'property list uchar int vertex_indices\nend_header\n'
                       '0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n')
        mesh = call('project', 'import-resource', str(toy), '--folder', 'Meshes', '--wait')
        assert mesh['state'] == 'succeeded', mesh
        asset = next(item for item in mesh['result']['created'] if item['kind'] == 'asset')
        assert asset['location'] == 'Meshes/toy.ply', mesh
        state = call('project', 'state')
        assert state['scene_count'] == 2 and state['dirty'] is True, state
        # One import is one history entry: undo removes the PLY batch whole,
        # redo brings it back; the two scene imports stay untouched.
        call('project', 'undo')
        meshes = call('project', 'contents', 'Meshes')['entries']
        assert all(entry['id'] != asset['id'] for entry in meshes)
        assert call('project', 'state')['scene_count'] == 2
        call('project', 'redo')
        assert any(entry['id'] == asset['id'] for entry in call('project', 'contents', 'Meshes')['entries'])
        # Saving persists the imported items; reopening shows them again.
        saved = call('project', 'save', '--wait')
        assert saved['state'] == 'succeeded', saved
        assert call('project', 'state')['dirty'] is False
        current_path = Path(call('project', 'state')['path']) or project
        call('project', 'close')
        reopened = call('project', 'open', str(current_path), '--wait')
        assert reopened['state'] == 'succeeded', reopened
        state = call('project', 'state')
        assert state['scene_count'] == 2 and state['dirty'] is False, state
        assert state['active_scene'] is None, 'reopen does not open imported scenes'
        images = call('project', 'contents', 'Images')['entries']
        assert any(entry['id'] == image['result']['created'][0]['id'] for entry in images)
        assert call('project', 'asset', asset['id'])['mesh']['vertices'] == 3
        call('project', 'close')
        assert call('quit')['quitting'] is True
        host.wait(timeout=15)
        assert host.returncode == 0, host.stderr.read()
        assert not endpoint.exists(), 'quit removes the control socket'
        print(json.dumps({'project_control_smoke': 'passed', 'checks': ['protocol_v2', 'import', 'hierarchy',
              'revision_conflict', 'dirty_guard', 'undo_redo', 'content_dirty_baseline', 'folder_save', 'source_independent_export',
              'scene_graph', 'presets', 'cache', 'parameter_schema', 'create_resource', 'component_edits', 'set_implementation', 'resource_graph', 'texture_connections', 'mcp_schema', 'shared_cli_mcp_edits', 'mesh_statistics', 'texture_preview', 'material_preview', 'where_used', 'optional_scene', 'texture_value_type', 'import_jobs', 'quit']}))
    finally:
        if bridge and bridge.poll() is None:
            bridge.terminate()
            bridge.wait(timeout=5)
        if host.poll() is None:
            host.terminate()
            host.wait(timeout=15)
        assert host.returncode == 0, host.stderr.read()
