#!/usr/bin/env python3
"""Exercise the real HTTP supervisor with a fake sender, without opening ALSA/AirPlay."""
import json, os, pathlib, signal, socket, subprocess, sys, tempfile, time, urllib.request, urllib.error
binary = pathlib.Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory() as temp:
    root = pathlib.Path(temp); (root/'bin').mkdir(); (root/'control').mkdir()
    fake = root/'bin/vinyl-airplay'
    fake.write_text('''#!/usr/bin/python3
import os,time,signal,sys,pathlib
root=pathlib.Path(os.environ['VINYL_ROOT'])
with (root/'starts').open('a') as f:f.write(str(os.getpid())+' '+repr(sys.argv)+'\\n')
signal.signal(signal.SIGINT,lambda *_:sys.exit(0))
if (root/'fail-next-start').exists():
 (root/'fail-next-start').unlink()
 sys.exit(2)
print('Starting ALSA capture on fake',file=sys.stderr,flush=True)
print('Monitoring input: no HomePod session;',file=sys.stderr,flush=True)
while True:
 print('control_event={"type":"meter","rms_dbfs":-40,"peak_dbfs":-20}',file=sys.stderr,flush=True)
 time.sleep(.1)
''');fake.chmod(0o755)
    mixer=root/'amixer';mixer.write_text('#!/bin/sh\nexit 0\n');mixer.chmod(0o755)
    fixtures=[{'id':'airplay:001122334455','name':'Salon','address':'192.168.50.20:7000','model':'AudioAccessory1,1','available':True,'last_seen':0,'service':'Salon._airplay._tcp.local.'},{'id':'airplay:66778899aabb','name':'Chambre','address':'192.168.50.21:7000','model':'AudioAccessory5,1','available':True,'last_seen':0,'service':'Chambre._airplay._tcp.local.'},{'id':'airplay:112233445566','name':'Cuisine','address':'192.168.50.22:7100','model':'AirPlay','available':True,'last_seen':0,'service':'Cuisine._airplay._tcp.local.'}]
    (root/'control/receivers.json').write_text(json.dumps(fixtures))
    (root/'control/config.json').write_text(json.dumps({'enabled':True,'auto_connect':True,'targets':['salon'],'latency_ms':100,'gain_db':0,'recognize':False}))
    token='a'*64;(root/'token').write_text(token)
    with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
    env={**os.environ,'VINYL_ROOT':str(root),'VINYL_TOKEN_FILE':str(root/'token'),'VINYL_BIND':f'127.0.0.1:{port}','VINYL_AMIXER_PROGRAM':str(mixer)}
    proc=subprocess.Popen([str(binary)],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    def req(path='state',config=None,auth=True,csrf=True):
        headers={}
        if auth:headers['Authorization']='Bearer '+token
        if config is not None:
            headers['Content-Type']='application/json'
            if csrf:headers['X-Vinyl-Control']='1'
        request=urllib.request.Request(f'http://127.0.0.1:{port}/api/{path}',data=json.dumps(config).encode() if config is not None else None,headers=headers)
        try:
            with urllib.request.urlopen(request,timeout=15) as r:return r.status,json.load(r)
        except urllib.error.HTTPError as e:return e.code,None
    try:
        for _ in range(100):
            try:code,data=req();break
            except (OSError,urllib.error.URLError):time.sleep(.05)
        else:raise AssertionError('agent did not start')
        assert req(auth=False)[0]==401
        assert req('volume',{'target':'localhost','percent':50})[0]==400
        assert req('volume',{'target':'salon','percent':101})[0]==400
        assert req('volume',{'target':'salon','percent':50},csrf=False)[0]==403
        assert req('volume',{'target':'salon','percent':50})[0]==409
        c=data['config'];assert c['targets']==['airplay:001122334455'] and c['latency_ms']==100
        assert req('config',c,csrf=False)[0]==403
        assert req('config',{**c,'targets':['127.0.0.1:7000']})[0]==400
        assert req('config',{**c,'gain_db':99})[0]==400
        assert req('config',{**c,'shell':'touch /tmp/no'})[0]==422
        time.sleep(.2);assert req()[1]['status']['rms_dbfs']==-40
        assert req('config',c)[0]==200
        assert len((root/'starts').read_text().splitlines())==1,'unchanged settings restarted sender'
        assert req('config',{**c,'targets':['airplay:ffffffffffff']})[0]==400
        assert req('meter')[1]['rms_dbfs']==-40
        assert any(d['id']=='airplay:112233445566' for d in data['devices'])
        assert data['active_targets']['airplay:001122334455']=='192.168.50.20:7000'
        c={**c,'targets':['airplay:001122334455','airplay:66778899aabb','airplay:112233445566'],'gain_db':3}
        assert req('config',c)[0]==200
        starts=(root/'starts').read_text().splitlines();assert len(starts)==2
        assert '192.168.50.22:7100' in starts[-1] and '192.168.50.21:7000' in starts[-1] and "'3'" in starts[-1]
        oldpid=int(starts[0].split()[0])
        try:os.kill(oldpid,0)
        except ProcessLookupError:pass
        else:raise AssertionError('previous sender is still alive')
        assert json.loads((root/'control/config.json').read_text())==c
        (root/'fail-next-start').touch()
        assert req('config',{**c,'gain_db':4})[0]==500,'failed start must fail the request'
        assert req()[1]['config']==c,'failed start must restore previous settings'
        assert json.loads((root/'control/config.json').read_text())==c
        assert req()[1]['running'],'rollback must restart previous capture'
        assert req('config',{**c,'enabled':False})[0]==200
        assert req()[1]['status']['phase']=='stopped'
        assert req('config',c)[0]==200
        pid=int((root/'starts').read_text().splitlines()[-1].split()[0]);os.kill(pid,signal.SIGTERM);time.sleep(.1)
        assert req()[1]['status']['phase']=='error'
        assert req('config',c)[0]==200
        print('PASS: auth, CSRF, validation, meter, targets/gain, persistence, single capture, stop/start and crash reporting')
    finally:
        proc.send_signal(signal.SIGINT)
        try:proc.communicate(timeout=12)
        except subprocess.TimeoutExpired:proc.kill();proc.communicate();raise
