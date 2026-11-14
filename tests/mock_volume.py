"""Independent encrypted volume test: per-speaker control, native dvlc and bit-exact PCM.
No real ALSA device or HomePod is contacted.
"""
import asyncio,json,os,pathlib,struct,sys,tempfile,uuid
from mock_homepods import Receiver

async def main(binary):
    peers=[Receiver(0),Receiver(1)]
    for p in peers:p.packet_count=1000
    peers[0].volume_test=True
    ports=[await p.start() for p in peers]
    with tempfile.TemporaryDirectory() as temp:
        path=pathlib.Path(r'\\.\pipe\vinyl-volume-'+str(uuid.uuid4())) if os.name == 'nt' else pathlib.Path(temp)/'audio.sock'
        pcm=struct.pack('>hh',1000,1000)*352*1000
        proc=await asyncio.create_subprocess_exec(binary,'stream','--target',f'127.0.0.1:{ports[0]}','--target',f'127.0.0.1:{ports[1]}',stdin=asyncio.subprocess.PIPE,stdout=asyncio.subprocess.PIPE,stderr=asyncio.subprocess.PIPE,env={**os.environ,'VINYL_CONTROL_SOCKET':str(path)})
        output=asyncio.create_task(proc.communicate(pcm))
        async def rpc(request):
            if os.name == 'nt':
                loop=asyncio.get_running_loop()
                reader=asyncio.StreamReader()
                protocol=asyncio.StreamReaderProtocol(reader)
                transport,_=await loop.create_pipe_connection(lambda: protocol,str(path))
                writer=asyncio.StreamWriter(transport,protocol,reader,loop)
            else:
                reader,writer=await asyncio.open_unix_connection(str(path))
            writer.write(json.dumps(request).encode()+b'\n');await writer.drain()
            response=json.loads(await asyncio.wait_for(reader.readline(),5))
            writer.close();await writer.wait_closed();return response
        try:
            await asyncio.wait_for(peers[0].native_volume_sent.wait(),6)
            status=await rpc({'op':'status'});assert status['ok']
            entries={r['target']:r for r in status['receivers']}
            targets=[f'127.0.0.1:{port}' for port in ports]
            assert abs(entries[targets[0]]['percent']-30)<1e-6,entries
            assert abs(entries[targets[1]]['percent']-60)<1e-6,entries
            assert peers[0].volume_updates==[], 'native dvlc must not echo to speaker'
            if os.name != 'nt': assert (path.stat().st_mode & 0o777)==0o600
            for i,level,db in [(0,20,-24),(1,80,-6),(0,0,-144),(0,100,0)]:
                reply=await rpc({'op':'set','target':targets[i],'percent':level})
                assert reply['ok'] and reply['db']==db,reply
            before=[list(p.volume_updates) for p in peers]
            for request in [{'op':'set','target':targets[0],'percent':101},{'op':'set','target':'127.0.0.1:1','percent':30},{'op':'set','target':targets[0],'percent':-1},{'op':'set','target':targets[0],'percent':20,'shell':'bad'}]:
                assert not (await rpc(request))['ok']
            assert [p.volume_updates for p in peers]==before
            assert peers[0].volume_updates==[-24,-144,0]
            assert peers[1].volume_updates==[-6]
            stdout,stderr=await asyncio.wait_for(output,15)
            assert proc.returncode==0,stderr.decode()
            await asyncio.gather(*(p.finish() for p in peers))
            assert all(b''.join(p.pcm)==pcm for p in peers),'volume control modified source PCM'
            assert not path.exists(),'control socket was not cleaned up'
            print('PASS: native dvlc acknowledged/observed without echo; invalid events ignored; private socket; encrypted SET_PARAMETER; independent receiver volumes/mute; invalid requests rejected; PCM unchanged; clean teardown')
        finally:
            if proc.returncode is None:proc.kill();await proc.wait()
            if not output.done():output.cancel()

asyncio.run(main(sys.argv[1]))
