#!/usr/bin/env python3
"""Fresh-session lifecycle using only the packaged sibling executables."""
import base64,json,os,select,subprocess,sys,tempfile,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
binaries=Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='pui-dist-') as tmp:
    directory=Path(tmp);directory.chmod(0o700);endpoint=directory/'control.sock'
    host=subprocess.Popen([str(binaries/'pbrt-ui-host'),'--endpoint',str(endpoint)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,env=dict(os.environ,XDG_CACHE_HOME=str(directory/'cache')))
    def ctl(*arguments):
        result=subprocess.run([str(binaries/'pbrt-ui-ctl'),'--endpoint',str(endpoint),*arguments],capture_output=True,text=True,timeout=60)
        assert result.returncode==0,result.stdout+result.stderr
        return json.loads(result.stdout)
    try:
        assert select.select([host.stdout],[],[],5)[0];host.stdout.readline()
        capabilities=ctl('capabilities');assert capabilities['scene_capture'] and capabilities['render']
        assert ctl('project','import',str(ROOT/'tests/fixtures/render-preview.pbrt'),str(directory/'project'),'--wait')['state']=='succeeded'
        assert ctl('project','state')['mode']=='project'
        captured=ctl('project','capture','--wait');assert captured['state']=='succeeded',captured
        preview=ctl('artifact',captured['artifacts'][0]);assert preview['mime']=='image/png'
        destination=directory/'preview.png';assert ctl('export',preview['artifact_id'],str(destination),'--wait')['state']=='succeeded'
        ctl('settings','{"samples_per_pixel":null,"threads":1,"max_depth":null}')
        assert ctl('project','render','--wait','--timeout-ms','45000')['state']=='succeeded'
        messages=[{'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','clientInfo':{'name':'dist','version':'1'},'capabilities':{}}},
            {'jsonrpc':'2.0','method':'notifications/initialized'},
            {'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'app_get_state','arguments':{}}},
            {'jsonrpc':'2.0','id':3,'method':'resources/read','params':{'uri':'pbrt://artifact/'+preview['artifact_id']}}]
        bridge=subprocess.run([str(binaries/'pbrt-ui-mcp'),'--endpoint',str(endpoint)],input=''.join(json.dumps(m)+'\n' for m in messages),capture_output=True,text=True,timeout=10)
        assert bridge.returncode==0,bridge.stderr
        responses=[json.loads(line) for line in bridge.stdout.splitlines()];assert len(responses)==3
        assert responses[1]['result']['structuredContent']['project']['mode']=='project'
        assert base64.b64decode(responses[2]['result']['contents'][0]['blob']).startswith(b'\x89PNG')
    finally:
        host.terminate();host.wait(timeout=10);assert host.returncode==0,host.stderr.read()
    assert not endpoint.exists();assert destination.read_bytes().startswith(b'\x89PNG')
    print(json.dumps({'distribution_smoke':'passed','checks':['sibling_discovery','project_import','project_capture','export','real_renderer','mcp','shutdown','export_survives_shutdown']}))
