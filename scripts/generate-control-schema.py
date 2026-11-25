#!/usr/bin/env python3
"""Generate the v2 control envelope, viewer and native-project parameter schemas.

Run without arguments to rewrite schemas/control-v2.schema.json, or with
--check to fail when the checked-in schema differs from the generated one.
"""
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]


def obj(properties, required=()):
    return {'type': 'object', 'properties': properties, 'required': list(required), 'additionalProperties': False}


def ref(name):
    return {'$ref': '#/$defs/' + name}


def nullable(schema):
    return {'anyOf': [schema, {'type': 'null'}]}


def counter_pattern():
    maximum = str(2**64 - 1)
    alternatives = ['0', '[1-9][0-9]{0,18}']
    for index, digit in enumerate(maximum):
        low = 1 if index == 0 else 0
        high = int(digit) - 1
        if high >= low:
            alternatives.append(maximum[:index] + (str(low) if high == low else f'[{low}-{high}]')
                                + (f'[0-9]{{{19-index}}}' if index < 19 else ''))
    alternatives.append(maximum)
    return '^(?:' + '|'.join(alternatives) + ')$'


def generate():
    text = {'type': 'string'}
    identifier = {'type': 'string', 'format': 'uuid'}
    page_limit = {'type': 'integer', 'minimum': 1, 'maximum': 1000, 'default': 100}
    counter = {'type': 'string', 'pattern': counter_pattern()}
    definitions = {'counter': counter}
    parameter_schemas = {
        'app.hello': obj({'client_name': {'type': 'string', 'minLength': 1},
                          'supported_protocols': {'type': 'array', 'items': text}}, ['client_name', 'supported_protocols']),
        'app.get_state': nullable(obj({})),
        'app.get_capabilities': nullable(obj({})),
        'app.quit': obj({'discard_unsaved': {'type': 'boolean', 'default': False}}),
        'job.get': obj({'job_id': identifier}, ['job_id']),
        'job.cancel': obj({'job_id': identifier}, ['job_id']),
        'job.list': nullable(obj({'cursor': nullable(text), 'limit': page_limit})),
        'events.read': obj({'after': ref('counter'), 'limit': page_limit,
                            'wait_ms': {'type': 'integer', 'minimum': 0, 'maximum': 30000, 'default': 0}}, ['after']),
    }
    vec3={'type':'array','items':{'type':'number'},'minItems':3,'maxItems':3}
    size={'type':'array','items':{'type':'integer','minimum':1,'maximum':8192},'minItems':2,'maxItems':2}
    camera=obj({'eye':vec3,'target':vec3,'up':vec3,'projection':{'const':'perspective'},'fov_y_degrees':{'type':'number','exclusiveMinimum':0.1,'exclusiveMaximum':179},'near':{'type':'number','exclusiveMinimum':0},'far':{'type':'number','exclusiveMinimum':0},'mirror':{'type':'boolean'}},['eye','target','up','projection','fov_y_degrees','near','far'])
    settings=obj({'samples_per_pixel':{'type':'null','description':'Deprecated compatibility field; must be null. Final render quality comes from the scene Sampler.'},'threads':nullable({'type':'integer','minimum':1,'maximum':1024}),'gpu':{'type':'boolean','default':False},'max_depth':{'type':'null','description':'Deprecated compatibility field; must be null. Final render depth comes from the scene Integrator.'}},['samples_per_pixel','threads','max_depth'])
    display_mode={'enum':['solid','normal','wireframe','lit']}
    view_state=obj({'camera':camera,'size':size,'mode':display_mode,'time':{'type':'number'}},['camera','size','mode','time'])
    parameter_schemas.update({
        'render.get_settings':obj({}),
        'render.set_settings':obj({'settings':settings,'expected_settings_revision':ref('counter')},['settings','expected_settings_revision']),
        'render.ipr_set':obj({'enabled':{'type':'boolean'},'settings':obj({'samples_per_pixel':{'type':'integer','minimum':1,'maximum':256},'threads':nullable({'type':'integer','minimum':1,'maximum':1024}),'max_depth':nullable({'type':'integer','minimum':1,'maximum':1024}),'debounce_ms':{'type':'integer','minimum':10,'maximum':5000}},['samples_per_pixel','threads','max_depth','debounce_ms'])},['enabled','settings']),
        'artifact.get':obj({'artifact_id':identifier},['artifact_id']),
        'artifact.export':obj({'artifact_id':identifier,'absolute_path':{'type':'string','pattern':'^/'},'overwrite':{'type':'boolean','default':False}},['artifact_id','absolute_path']),
        'workspace.reset_layout':obj({}),
    })
    project_ref = obj({'project_id': identifier, 'revision': ref('counter'), 'activation': ref('counter')}, ['project_id', 'revision', 'activation'])
    context = obj({'activation': ref('counter'), 'project': nullable(project_ref)}, ['activation', 'project'])
    definitions['project_ref'] = project_ref
    definitions['project_context'] = context
    def tagged(kind, payload=None):
        return obj({'kind': {'const': kind}, **({'value': payload} if payload is not None else {})}, ['kind'] + (['value'] if payload is not None else []))
    def fixed(n):
        return {'type':'array','items':{'type':'number'},'minItems':n,'maxItems':n}
    transform = {'oneOf':[tagged('identity'),tagged('translate',fixed(3)),tagged('scale',fixed(3)),tagged('look_at',fixed(9)),tagged('replace',fixed(16)),tagged('concat',fixed(16)),tagged('rotate',obj({'degrees':{'type':'number'},'axis':fixed(3)},['degrees','axis']))]}
    target = {'oneOf':[obj({'kind':{'const':kind},'id':identifier},['kind','id']) for kind in ['node','resource']]}
    resource_kind = {'enum':['material','texture','medium','object_definition','spectrum_data']}
    values = {'oneOf':[tagged(kind,{'type':'array','items':item,'maxItems':4096}) for kind,item in [
        ('floats',{'type':'number'}),('integers',{'type':'string','pattern':'^(0|-[1-9][0-9]*|[1-9][0-9]*)$'}),('booleans',{'type':'boolean'}),('strings',{'type':'string','maxLength':4096}),('builtin_spectra',{'type':'string','maxLength':4096}),('assets',identifier)]] + [tagged('resources',obj({'kind':resource_kind,'ids':{'type':'array','items':identifier,'maxItems':4096}},['kind','ids']))]}
    parameter = obj({'name':text,'value_type':text,'color_space':nullable(text),'values':values},['name','value_type','values'])
    binding = {'oneOf':[tagged('default'),tagged('none'),tagged('resource',identifier)]}
    project_item = {'oneOf':[obj({'kind':{'const':kind},'id':identifier},['kind','id']) for kind in ['scene','resource','asset']]}
    edit = {'oneOf':[
        obj({'kind':{'const':'create_camera'},'scene':identifier},['kind','scene']),
        obj({'kind':{'const':'create_configuration'},'scene':identifier,'configuration':{'enum':['film','sampler','integrator','pixel_filter','accelerator']},
             'implementation':nullable({'type':'string','minLength':1,'maxLength':128})},
            ['kind','scene','configuration']),
        obj({'kind':{'const':'set_render_camera'},'scene':identifier,'camera':nullable(identifier)},['kind','scene','camera']),
        obj({'kind':{'const':'set_scene_coordinate_system'},'scene':identifier,'up':{'enum':['+X','-X','+Y','-Y','+Z','-Z']},'handedness':{'enum':['left','right']}},['kind','scene','up','handedness']),
        obj({'kind':{'const':'create_scene'},'location':text},['kind','location']),
        obj({'kind':{'const':'duplicate_scene'},'scene':identifier,'location':text},['kind','scene','location']),
        obj({'kind':{'const':'delete_scene'},'scene':identifier},['kind','scene']),
        obj({'kind':{'const':'move_item'},'item':project_item,'location':text},['kind','item','location']),
        obj({'kind':{'const':'delete_item'},'item':project_item},['kind','item']),
        obj({'kind':{'const':'create_folder'},'path':text},['kind','path']),
        obj({'kind':{'const':'move_folder'},'from':text,'to':text},['kind','from','to']),
        obj({'kind':{'const':'delete_folder'},'path':text},['kind','path']),
        obj({'kind':{'const':'rename_node'},'node_id':identifier,'label':{'type':'string','maxLength':1024}},['kind','node_id','label']),
        obj({'kind':{'const':'set_node_active'},'node_id':identifier,'active':{'type':'boolean'}},['kind','node_id','active']),
        obj({'kind':{'const':'rename_resource'},'resource_id':identifier,'label':{'type':'string','maxLength':1024}},['kind','resource_id','label']),
        obj({'kind':{'const':'set_texture_value_type'},'resource_id':identifier,'value':{'enum':['float','spectrum']}},['kind','resource_id','value']),
        obj({'kind':{'const':'set_transform'},'node_id':identifier,'value':transform},['kind','node_id','value']),
        obj({'kind':{'const':'set_material_binding'},'node_id':identifier,'value':binding},['kind','node_id','value']),
        obj({'kind':{'const':'set_parameter'},'target':target,'name':text,'value_type':text,'value':nullable(parameter)},['kind','target','name','value_type','value']),
        obj({'kind':{'const':'set_parameter_values'},'target':target,'name':text,'offset':{'type':'integer','minimum':0},'values':values},['kind','target','name','offset','values']),
        obj({'kind':{'const':'create_preset'},'preset':identifier,'parent':identifier,'after':nullable(identifier),'definition':nullable(identifier),
             'view':nullable(obj({'position':fixed(3),'look_at':fixed(3)},['position','look_at']))},
            ['kind','preset','parent']),
        obj({'kind':{'const':'create_resource'},'id':identifier,'resource_kind':resource_kind,'implementation':text,'label':text,'folder':nullable(text),
             'texture_value_type':nullable({'enum':['float','spectrum']}),
             'parameters':nullable({'type':'array','items':parameter,'maxItems':4096})},
            ['kind','resource_kind','implementation','label']),
        obj({'kind':{'const':'delete_node'},'node_id':identifier},['kind','node_id']),
        obj({'kind':{'const':'duplicate_node'},'node_id':identifier},['kind','node_id']),
        obj({'kind':{'const':'move_node'},'node_id':identifier,'parent':identifier,'after':nullable(identifier)},['kind','node_id','parent']),
        obj({'kind':{'const':'make_object_definition'},'node_id':identifier,'label':nullable(text)},['kind','node_id']),
        obj({'kind':{'const':'unpack_object_instance'},'node_id':identifier},['kind','node_id']),
        obj({'kind':{'const':'set_instance_definition'},'node_id':identifier,'definition':identifier},['kind','node_id','definition']),
        obj({'kind':{'const':'set_local_binding'},'node_id':identifier,'material':nullable(binding),'medium':nullable(obj({'inside':nullable(identifier),'outside':nullable(identifier)},['inside','outside']))},['kind','node_id']),
        obj({'kind':{'const':'set_area_light'},'node_id':identifier,'light':nullable(obj({'implementation':text,'parameters':nullable({'type':'array','items':parameter,'maxItems':4096})},['implementation']))},['kind','node_id']),
        obj({'kind':{'const':'clear_local_binding'},'node_id':identifier,'material':{'type':'boolean'},'medium':{'type':'boolean'}},['kind','node_id','material','medium']),
        obj({'kind':{'const':'set_reverse_orientation'},'node_id':identifier,'enabled':{'type':'boolean'}},['kind','node_id','enabled']),
        obj({'kind':{'const':'set_local_matrix'},'node_id':identifier,'matrix':fixed(16)},['kind','node_id','matrix']),
        obj({'kind':{'const':'set_local_trs'},'node_id':identifier,'translation':fixed(3),'rotation_euler_degrees':fixed(3),'scale':fixed(3)},['kind','node_id','translation','rotation_euler_degrees','scale']),
        obj({'kind':{'const':'set_asset_reference'},'target':target,'name':text,'asset':identifier},['kind','target','name','asset']),
        obj({'kind':{'const':'connect_texture'},'target':target,'parameter':text,'texture':nullable(identifier),
             'create':nullable(obj({'implementation':text,'label':text,'folder':nullable(text)},['implementation','label'])),
             'image':nullable(identifier)},
            ['kind','target','parameter']),
        obj({'kind':{'const':'disconnect_texture'},'target':target,'parameter':text},['kind','target','parameter']),
        obj({'kind':{'const':'set_implementation'},'target':target,'implementation':{'type':'string','minLength':1,'maxLength':128},'parameters':{'const':'keep_compatible'}},['kind','target','implementation','parameters'])]}
    common_project = {'project':ref('project_ref')}
    node_view = {'enum':['directives','scene_graph'],'default':'directives'}
    switch = {'expected_current':ref('project_context'),'discard_unsaved':{'type':'boolean','default':False}}
    parameter_schemas.update({
        'project.get_state':nullable(obj({})),
        'project.import':obj({**switch,'source_path':text,'destination':nullable(text)},['expected_current','source_path']),
        'project.import_scene':obj({**common_project,'source_path':text,'folder':nullable(text)},['project','source_path']),
        'project.import_resource':obj({**common_project,'source_path':text,'folder':text},['project','source_path','folder']),
        'project.open':obj({**switch,'path':text},['expected_current','path']),
        'project.create':obj({**switch,'path':text,'title':text,
                               'coordinate_system':nullable(obj({'up':{'enum':['+X','-X','+Y','-Y','+Z','-Z']},'handedness':{'enum':['left','right']}},[]))},
                              ['expected_current','path','title']),
        'project.close':obj(switch,['expected_current']),
        'project.save':obj({**common_project,'destination':nullable(text)},['project']),
        'project.export':obj({**common_project,'destination':text,'scene':nullable(identifier)},['project','destination']),
        'project.preview':obj({**common_project,'destination':text,'store':nullable(text),'stage_assets':{'type':'boolean','default':True}},['project','destination']),
        'project.preview_transforms':obj({**common_project,'base_revision':ref('counter')},['project','base_revision']),
        'project.render':obj({**common_project,'expected_settings_revision':ref('counter'),'view':nullable(view_state),'scene':nullable(identifier),'camera':nullable({'enum':['scene','view']})},['project','expected_settings_revision']),
        'project.capture':obj({**common_project,'view':nullable(view_state),'scene':nullable(identifier),'camera':nullable({'enum':['scene','view']}),'debug_albedo':{'type':'boolean','default':False},'debug_ao':{'type':'boolean','default':False}},['project']),
        'project.pick':obj({**common_project,'view':nullable(view_state),'scene':nullable(identifier),'normalized_xy':{'type':'array','items':{'type':'number','minimum':0,'maximum':1},'minItems':2,'maxItems':2}},['project','normalized_xy']),
        'project.apply':obj({**common_project,'edits':{'type':'array','items':edit,'minItems':1,'maxItems':64},'capture':nullable(obj({'view':nullable(view_state)}))},['project','edits']),
        'project.undo':obj(common_project,['project']),
        'project.redo':obj(common_project,['project']),
        'project.list_nodes':obj({**common_project,'parent':nullable(identifier),'view':node_view,'offset':{'type':'integer','minimum':0},'limit':page_limit},['project']),
        'project.list_contents':obj({**common_project,'folder':nullable(text),'offset':{'type':'integer','minimum':0},'limit':page_limit},['project']),
        'project.list_references':obj({**common_project,'item':project_item,'offset':{'type':'integer','minimum':0},'limit':page_limit},['project','item']),
        'project.list_scenes':obj(common_project,['project']),
        'project.open_scene':obj({**common_project,'scene':identifier},['project','scene']),
        'project.close_scene':obj(common_project,['project']),
        'project.list_presets':obj(common_project,['project']),
        'project.get_node':obj({**common_project,'node_id':identifier},['project','node_id']),
        'project.get_resource':obj({**common_project,'resource_id':identifier},['project','resource_id']),
        'project.get_asset':obj({**common_project,'asset_id':identifier},['project','asset_id']),
        'project.get_cache':obj({**common_project,'item':identifier,'key':text},['project','item','key']),
        'project.put_cache':obj({**common_project,'item':identifier,'key':text,
            'kind':{'enum':['thumbnail','preview']},'media_type':text,
            'data_base64':{'type':'string','maxLength':5592408}},
            ['project','item','key','kind','media_type','data_base64']),
        'project.get_parameters':obj({**common_project,'target':target,'offset':{'type':'integer','minimum':0},'limit':page_limit},['project','target']),
        'project.get_parameter_values':obj({**common_project,'target':target,'name':text,'offset':{'type':'integer','minimum':0},'limit':{'type':'integer','minimum':1,'maximum':4096}},['project','target','name']),
        'project.set_parameter_values':obj({**common_project,'target':target,'name':text,'windows':{'type':'array','minItems':1,'maxItems':4096,'items':obj({'offset':{'type':'integer','minimum':0},'values':values},['offset','values'])}},['project','target','name','windows']),
        'project.get_resource_graph':obj({**common_project,'resource_id':identifier,'limit':nullable({'type':'integer','minimum':1,'maximum':4096})},['project','resource_id']),
        'project.list_spectra':obj({}),
        'project.spectrum_color':obj({**common_project,'target':target,'parameter':text,
            'role':nullable({'enum':['albedo','unbounded','illuminant','chromaticity','conductor']})},['project','target','parameter']),
        'project.preview_texture':obj({**common_project,'resource_id':identifier,'size':nullable({'type':'integer','minimum':8,'maximum':512})},['project','resource_id']),
        'project.preview_material':obj({**common_project,'resource_id':identifier,'size':nullable({'type':'integer','minimum':8,'maximum':512})},['project','resource_id']),
        'project.preview_asset':obj({**common_project,'asset_id':identifier,'size':nullable({'type':'integer','minimum':8,'maximum':512})},['project','asset_id']),
        'project.plan_implementation':obj({**common_project,'target':target,'implementation':{'type':'string','minLength':1,'maxLength':128}},['project','target','implementation']),
        'project.get_parameter_schema':obj({
            'directive':{'enum':['Shape','Material','MakeNamedMaterial','LightSource','AreaLightSource',
                                  'Texture','MakeNamedMedium','Camera','Film','Sampler','Integrator',
                                  'PixelFilter','Accelerator']},
            'implementation':text,
            'texture_value_type':{'enum':['float','spectrum']},
        },['directive','implementation']),
        'project.search_nodes':obj({**common_project,'query':{'type':'string','minLength':1,'maxLength':256},'view':node_view,'offset':{'type':'integer','minimum':0},'limit':page_limit},['project','query']),
        'project.search_contents':obj({**common_project,'query':{'type':'string','minLength':1,'maxLength':256},'offset':{'type':'integer','minimum':0},'limit':page_limit},['project','query']),
        'project.get_view':obj(common_project,['project']),
        'project.set_camera':obj({**common_project,'expected_view_revision':ref('counter'),'camera':camera},['project','expected_view_revision','camera']),
        'project.configure_view':obj({**common_project,'expected_view_revision':ref('counter'),'size':nullable(size),'mode':nullable(display_mode),'time':nullable({'type':'number'})},['project','expected_view_revision']),
    })
    # render.ipr_submit takes an optional native project ref, so it follows the
    # project definitions while staying next to the other render methods.
    ipr_submit=obj({'project':nullable(ref('project_ref')),'view':nullable(view_state),'scene':nullable(identifier)})
    ordered = {}
    for method, parameters in parameter_schemas.items():
        ordered[method] = parameters
        if method == 'render.ipr_set':
            ordered['render.ipr_submit'] = ipr_submit
    parameter_schemas = ordered
    variants = []
    for method, parameters in parameter_schemas.items():
        definitions['params.' + method] = parameters
        required = ['protocol', 'request_id', 'method']
        if method not in ('app.get_state', 'app.get_capabilities', 'job.list'):
            required.append('params')
        variants.append(obj({'protocol': {'const': '2'}, 'request_id': {'type': 'string', 'minLength': 1, 'maxLength': 256},
                             'method': {'const': method}, 'params': ref('params.' + method)}, required))
    definitions['request'] = {'oneOf': variants}
    errors = re.search(r'pub enum ErrorCode \{(.*?)\n\}', (ROOT / 'crates/protocol/src/error.rs').read_text(), re.S).group(1)
    codes = [re.sub(r'(?<!^)(?=[A-Z])', '_', name).lower() for name in re.findall(r'^    (\w+),', errors, re.M)]
    definitions['error'] = obj({'code': {'enum': codes}, 'message': text,
                                'target_id': nullable(text), 'details': {}}, ['code', 'message'])
    definitions['response'] = obj({'protocol': {'const': '2'}, 'request_id': text,
                                   'result': {}, 'error': ref('error')}, ['protocol', 'request_id'])
    definitions['response']['oneOf'] = [{'required': ['result'], 'not': {'required': ['error']}},
                                         {'required': ['error'], 'not': {'required': ['result']}}]
    definitions['event'] = obj({'sequence': ref('counter'), 'kind': text,
                                'scene_id': nullable(identifier), 'job_id': nullable(identifier), 'payload': {}}, ['sequence', 'kind'])
    return {'$schema': 'https://json-schema.org/draft/2020-12/schema',
            'title': 'PBRT UI control v2 (viewer and native projects)',
            'description': 'UTF-8 NDJSON objects, at most 8 MiB excluding LF/CRLF. Duplicate keys are rejected before schema validation. Methods added in later phases are unsupported until their capabilities are enabled. UUID formats and request_id byte length are also checked by the Rust host.',
            'oneOf': [ref('request'), ref('response'), ref('event')], '$defs': definitions}


if __name__ == '__main__':
    destination = ROOT / 'schemas/control-v2.schema.json'
    generated = json.dumps(generate(), ensure_ascii=False, indent=2) + '\n'
    if sys.argv[1:] == ['--check']:
        if destination.read_text() != generated:
            sys.exit(f'{destination} differs from the generator; run {Path(__file__).name} to regenerate')
    elif sys.argv[1:]:
        sys.exit(f'usage: {Path(__file__).name} [--check]')
    else:
        destination.write_text(generated)
