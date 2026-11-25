#!/usr/bin/env python3
"""Exercise MCP initialization, core tools, error results and state resources."""
import json,os,select,subprocess,tempfile
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='pui-mcp-') as tmp:
    directory=Path(tmp);directory.chmod(0o700);endpoint=directory/'control.sock'
    host=subprocess.Popen([str(ROOT/'target/debug/pbrt-ui-host'),'--endpoint',str(endpoint)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    bridge=None
    try:
        assert select.select([host.stdout],[],[],5)[0];host.stdout.readline()
        bridge=subprocess.Popen([str(ROOT/'target/debug/pbrt-ui-mcp'),'--endpoint',str(endpoint)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        count=0
        def rpc(method,params=None):
            global count
            count+=1;bridge.stdin.write(json.dumps({'jsonrpc':'2.0','id':count,'method':method,'params':params or {}})+'\n');bridge.stdin.flush()
            assert select.select([bridge.stdout],[],[],10)[0]
            response=json.loads(bridge.stdout.readline());assert response['id']==count;return response
        assert rpc('tools/list')['error']['code']==-32002
        assert rpc('initialize',{'protocolVersion':'2025-11-25','clientInfo':{'name':'smoke','version':'1'},'capabilities':{}})['result']['capabilities']['tools']=={}
        bridge.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n');bridge.stdin.flush()
        tools=rpc('tools/list')['result']['tools'];names={tool['name'] for tool in tools};assert 'project_get_state' in names and not any(name.startswith(('scene_','view_','selection_')) for name in names)
        assert 'project_import_scene' in names and 'project_import_resource' in names, sorted(names)
        state=rpc('tools/call',{'name':'app_get_state','arguments':{}})['result'];assert not state['isError']
        assert 'session_id' in state['structuredContent']
        assert rpc('tools/call',{'name':'project_open','arguments':{}})['result']['isError']
        resource=rpc('resources/read',{'uri':'pbrt://state'})['result']['contents'][0]
        assert json.loads(resource['text'])['session_id']==state['structuredContent']['session_id']
        assert rpc('does/not/exist')['error']['code']==-32601
        bridge.stdin.close();bridge.wait(timeout=5);assert bridge.returncode==0,bridge.stderr.read()
        print(json.dumps({'mcp_smoke':'passed','checks':['initialize','notifications','tools','core_state','tool_errors','resources','clean_eof']}))
    finally:
        if bridge and bridge.poll() is None:bridge.terminate();bridge.wait(timeout=5)
        host.terminate();host.wait(timeout=10);assert host.returncode==0,host.stderr.read()
