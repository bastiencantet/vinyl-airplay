'use strict';
const $=id=>document.getElementById(id);
const desktop = Boolean(window.__TAURI__?.core?.invoke);
if (desktop) { document.body.classList.add('desktop'); $('backgroundNote').hidden = false; }
let artworkUrl = '';
let desktopVisible = true;
let inputDevices = [], legacyInput = null;

let saved=null,busy=false,online=false,artworkKey='',needsRestart=false,polling=false,meterPolling=false;
let selected=new Set(),devices=new Map(),lastMeter=null;
const cards=new Map(),volumeBusy=new Set(),volumeDirty=new Set();
const phases={stopped:'Diffusion arrêtée',starting:'Démarrage…',monitoring:'En veille · à l’écoute de l’entrée',connecting:'Connexion aux enceintes…',playing:'Diffusion en cours',disconnecting:'Libération des enceintes…',error:'À vérifier'};
const frequencies=['60','120','200','320','500','800','1,2k','2k','3,2k','5k','8k','14k'];
const bars=frequencies.map((hz,i)=>{const bar=document.createElement('span');bar.className='spectrum-band';const fill=document.createElement('i');fill.style.height='0%';bar.append(fill);$('spectrum').append(bar);return fill;});
function draft(){return {input_device:$('inputDevice').value,enabled:saved?.enabled??false,auto_connect:$('auto').checked,targets:[...selected].sort(),latency_ms:Number($('latency').value),gain_db:Number($('gain').value),recognize:$('recognize').checked};}
function dirty(){return saved&&JSON.stringify(draft())!==JSON.stringify({...saved,targets:[...saved.targets].sort()});}
function labels(){
 $('latencyValue').textContent=$('latency').value+' ms';const gain=Number($('gain').value);$('gainValue').textContent=(gain>0?'+':'')+gain+' dB';
 document.querySelectorAll('[data-ms]').forEach(b=>b.classList.toggle('selected',b.dataset.ms===$('latency').value));
 $('gainHelp').textContent=gain!==0?'Le gain reste fixe. Surveille la marge avant saturation.':'0 dB conserve les échantillons capturés.';
 $('apply').disabled=!online||busy||!dirty()||(Boolean(saved?.enabled)&&!selected.size);
 $('apply').textContent=saved?.enabled?'Appliquer les réglages':'Enregistrer les réglages';
 $('inputDevice').disabled=busy||!online||Boolean(legacyInput);
 if(legacyInput){$('inputHelp').textContent='Capture ALSA : '+legacyInput;$('refreshInputs').hidden=true;}
 $('changeHint').textContent=dirty()?(saved?.enabled?'Appliquer reconnectera brièvement les enceintes.':'Les réglages seront utilisés au prochain démarrage.'):'Les réglages actuels sont chargés.';
 $('power').disabled=!online||busy||(!saved?.enabled&&!selected.size);
 for(const [id,card] of cards){const on=selected.has(id);card.root.classList.toggle('selected',on);card.select.setAttribute('aria-pressed',String(on));card.check.textContent=on?'✓':'+';card.volume.hidden=!on;}
}
function fill(c){saved={input_device:c.input_device||'default',enabled:c.enabled,auto_connect:c.auto_connect,targets:c.targets,latency_ms:c.latency_ms,gain_db:c.gain_db,recognize:c.recognize};ensureInput(saved.input_device);$('inputDevice').value=saved.input_device;selected=new Set(c.targets);$('auto').checked=c.auto_connect;$('recognize').checked=c.recognize;$('latency').value=c.latency_ms;$('gain').value=c.gain_db;labels();}
async function api(path,config){if(desktop){try{return await window.__TAURI__.core.invoke('control_request',{operation:path,payload:config??null});}catch(error){throw new Error(typeof error==='string'?error:'Le moteur audio ne répond pas.');}}const r=await fetch('/api/'+path,{method:config?'POST':'GET',headers:config?{'Content-Type':'application/json','X-Vinyl-Control':'1'}:{},body:config?JSON.stringify(config):undefined,signal:AbortSignal.timeout(path==='meter'?1200:path==='video'?25000:20000),cache:'no-store'});const data=await r.json();if(!r.ok)throw new Error(data.error||'Le serveur ne répond pas correctement.');return data;}
function showError(e){$('error').hidden=!e;$('error').textContent=e||'';}
function element(tag,cls,text){const e=document.createElement(tag);if(cls)e.className=cls;if(text)e.textContent=text;return e;}
// Select artwork from discovered device metadata, never a room name or device ID.
function artworkForDevice(device){
 const model=device.model,color=device.enclosure_color;
 if(/^AudioAccessory[0-9]+,[0-9]+$/.test(model)&&Number.isInteger(color)&&color>0&&color<=255){
  
  
  return {src:`https://statici.icloud.com/fmipmobile/deviceImages-9.0/HomePod/${model}-${color}-${color}-0/online-sourcelist__3x.png`,label:model==='AudioAccessory5,1'?'HomePod mini':'HomePod'};
 }
 const mac=/^(MacBookPro|MacBookAir|MacBook|Macmini|MacPro|MacStudio|iMacPro|iMac|Mac)[0-9]+,[0-9]+$/.exec(model);
 if(mac){
  const family=mac[1],label=family.replace('MacBookPro','MacBook Pro').replace('MacBookAir','MacBook Air').replace('Macmini','Mac mini').replace('MacStudio','Mac Studio').replace('MacPro','Mac Pro').replace('iMacPro','iMac Pro');
  return {src:`https://statici.icloud.com/fmipmobile/deviceImages-9.0/${family}/${model}/online-sourcelist__3x.png`,label};
 }
 return null;
}
function speakerIcon(device){
 const icon=element('span','speaker-icon');icon.setAttribute('aria-hidden','true');
 const artwork=artworkForDevice(device);
 if(artwork){
  icon.classList.add('homepod-icon');icon.title=artwork.label;
  const img=element('img');img.src=artwork.src;img.alt='';img.width=50;img.height=50;img.decoding='async';img.referrerPolicy='no-referrer';img.onerror=()=>{img.remove();icon.classList.remove('homepod-icon');icon.textContent=device.model.startsWith('AudioAccessory')?'◉':'▱';};icon.append(img);
 }else icon.textContent=device.model.startsWith('AudioAccessory')?'◉':device.model.startsWith('Mac')?'▱':'▣';
 return icon;
}
function makeCard(device){
 const id=device.id,root=element('article','speaker-card'),select=element('button','speaker-select');select.type='button';
 const icon=speakerIcon(device);
 const heading=element('span','speaker-heading'),name=element('strong','',device.name),status=element('small'),check=element('span','speaker-check','+');check.setAttribute('aria-hidden','true');heading.append(name,status);select.append(icon,heading,check);
 select.onclick=()=>{if(selected.has(id))selected.delete(id);else if(devices.get(id)?.available)selected.add(id);labels();};
 const volume=element('div','speaker-volume'),line=element('div','volume-label'),label=element('label','','Volume'),output=element('output','','—');const input=element('input');input.type='range';input.min='0';input.max='100';input.value='0';input.disabled=true;input.id='volume-'+id;label.htmlFor=input.id;input.setAttribute('aria-label','Volume · '+device.name);line.append(label,output);volume.append(line,input);
 const note=element('small','volume-note','Disponible pendant la diffusion.');volume.append(note);root.append(select,volume);$('devices').append(root);
 input.addEventListener('input',()=>{volumeDirty.add(id);output.textContent=input.value+' %';input.style.setProperty('--level',input.value+'%');});
 input.addEventListener('change',async()=>{if(volumeBusy.has(id))return;volumeBusy.add(id);card.volumeError='';input.disabled=true;note.textContent='Réglage…';try{await api('volume',{target:id,percent:Number(input.value)});note.textContent='Volume de l’enceinte · sans reconnexion.';}catch(e){note.textContent=e.message;card.volumeError=e.message;}finally{volumeBusy.delete(id);volumeDirty.delete(id);await poll();}});
 const card={root,select,name,status,check,volume,input,output,note,icon,iconKey:device.model+":"+device.enclosure_color};cards.set(id,card);return card;
}
function updateDevices(data){
 devices=new Map((data.devices||[]).map(d=>[d.id,d]));
 // A saved selection stays visible even if Bonjour has not found it yet.
 for(const id of selected)if(!devices.has(id))devices.set(id,{id,name:'Enceinte mémorisée',model:'AirPlay',available:false});
 const visible=[...devices.values()].filter(d=>d.available||selected.has(d.id)).sort((a,b)=>a.name.localeCompare(b.name,'fr'));
 $('deviceCount').textContent=visible.filter(d=>d.available).length+' disponible'+(visible.filter(d=>d.available).length>1?'s':'');
 $('deviceEmpty').hidden=!!visible.length;
 $('deviceEmpty').textContent='Aucune enceinte détectée pour le moment. Vérifie qu’elle est sur le même réseau.';
 for(const d of visible){const c=cards.get(d.id)||makeCard(d);c.name.textContent=d.name;
  const iconKey=d.model+":"+d.enclosure_color;if(c.iconKey!==iconKey){const icon=speakerIcon(d);c.icon.replaceWith(icon);c.icon=icon;c.iconKey=iconKey;}
  c.select.disabled=busy||!online||(!d.available&&!selected.has(d.id));c.status.textContent=d.available?(d.model.startsWith('AudioAccessory')?(d.model==='AudioAccessory5,1'?'HomePod mini · disponible':'HomePod · disponible'):'AirPlay · compatibilité à vérifier'):'Hors ligne · sélection conservée';c.root.classList.toggle('offline',!d.available);
  const address=data.active_targets?.[d.id]||d.address,entry=(data.volumes||[]).find(v=>v.target===address),known=entry&&Number.isFinite(entry.percent)&&online;
  c.input.disabled=!known||busy||volumeBusy.has(d.id);
  if(!volumeDirty.has(d.id)&&!volumeBusy.has(d.id)){if(known)c.input.value=Math.round(entry.percent);c.output.textContent=known?Math.round(entry.percent)+' %':'—';c.input.style.setProperty('--level',known?c.input.value+'%':'0%');}
  if(!volumeBusy.has(d.id))c.note.textContent=c.volumeError||(known?'Volume de l’enceinte · sans reconnexion.':'Disponible pendant la diffusion.');
 }
 const ids=new Set(visible.map(d=>d.id));for(const [id,c] of cards)if(!ids.has(id)&&!volumeBusy.has(id)){c.root.remove();cards.delete(id);}
 labels();
}
function renderMeter(s){
 lastMeter=s;
 const fresh=online&&Number.isFinite(s.rms_dbfs)&&s.server_time-s.meter_at<2;
 $('spectrumState').textContent=fresh?(s.peak_dbfs>-58?'Signal capturé':'Silence'):'En attente du signal';
 $('spectrumPanel').classList.toggle('quiet',!fresh||s.peak_dbfs<-58);
 bars.forEach((b,i)=>{const db=fresh&&Number.isFinite(s.spectrum_dbfs?.[i])?s.spectrum_dbfs[i]:-90;b.style.height=Math.max(0,Math.min(100,(db+75)/65*100))+'%';});
 for(let c=0;c<2;c++){const rms=fresh?(s.channels_rms_dbfs?.[c]??s.rms_dbfs):-90,peak=fresh?(s.channels_peak_dbfs?.[c]??s.peak_dbfs):-90;const fill=$('channel'+c),marker=$('peak'+c),meter=fill.parentElement;fill.style.width=Math.max(0,Math.min(100,(rms+60)/60*100))+'%';marker.style.left=Math.max(0,Math.min(100,(peak+60)/60*100))+'%';meter.setAttribute('aria-valuenow',Math.max(-60,rms));$('channelValue'+c).textContent=fresh?rms.toFixed(1)+' dBFS':'—';}
 $('peakValue').textContent=fresh?s.peak_dbfs.toFixed(1)+' dBFS':'—';
 const after=fresh?s.peak_dbfs+Number($('gain').value):null,clipping=after!==null&&after>=0;
 $('headroom').textContent=after===null?'Mesure avant gain':clipping?'Saturation possible avec ce gain':Math.max(0,-after).toFixed(1)+' dB de marge avec ce gain';$('headroom').classList.toggle('warning',clipping);
}
async function poll(force=false){if(polling)return;polling=true;try{const data=await api('state');legacyInput=data.legacy_input||null;online=true;$('connection').textContent=desktop?'Moteur audio prêt':'Contrôleur connecté';$('connection').className='connection online';if(!saved||force||!dirty())fill(data.config);updateDevices(data);const s=data.status;
 $('phase').textContent=phases[s.phase]||s.phase;needsRestart=data.config.enabled&&!data.running;$('powerText').textContent=needsRestart?'Relancer':data.config.enabled?'Arrêter':'Démarrer';$('powerIcon').textContent=data.config.enabled?'■':'▶';$('power').disabled=busy||(!data.config.enabled&&!selected.size);
 $('route').textContent=data.config.targets.map(id=>devices.get(id)?.name||'Enceinte mémorisée').join(' + ')||'Choisis une enceinte pour démarrer';
 const track=s.track||{};$('title').textContent=track.title||(s.phase==='playing'?'À l’écoute':'Prêt à diffuser');$('artist').textContent=track.artist||'Entrée audio';$('album').textContent=track.album||(data.config.recognize?'Reconnaissance après environ 12 s de musique.':'Reconnaissance des morceaux désactivée.');
 const key=track.artwork?track.key:'';if(key!==artworkKey){artworkKey=key;$('artwork').hidden=!key;if(key)loadArtwork(key);}
 renderMeter({...s,server_time:data.server_time});$('technical').textContent=`Entrée audio native · stéréo · PCM 44,1 kHz / 16 bits. Délai demandé : ${data.config.latency_ms} ms. Spectre : 12 bandes, calculé sur l’entrée avant gain. Les barres représentent le signal capturé, avec le délai de rafraîchissement du navigateur. La latence acoustique n’est pas mesurée.`;
 $('logs').textContent=(s.logs||[]).join('\n');showError(s.error||'');labels();
 }catch(e){online=false;$('connection').textContent='Connexion interrompue';$('connection').className='connection';$('power').disabled=true;$('apply').disabled=true;for(const c of cards.values()){c.input.disabled=true;c.select.disabled=true;}renderMeter({});showError('Le contrôleur est injoignable. Nouvelle tentative automatique.');}finally{polling=false;}}
async function meterPoll(){if(meterPolling||!online||document.hidden||!desktopVisible)return;meterPolling=true;try{renderMeter(await api('meter'));}catch{renderMeter({});}finally{meterPolling=false;}}
async function submit(c){busy=true;labels();$('power').disabled=true;showError('');try{await api('config',c);fill(c);await poll(true);}catch(e){showError(e.message);}finally{busy=false;await poll();labels();}}
$('apply').onclick=()=>submit(draft());$('power').onclick=()=>{if(saved&&!busy)submit(saved.enabled&&!needsRestart?{...saved,enabled:false}:{...draft(),enabled:true});};
[$('auto'),$('recognize'),$('gain'),$('latency'),$('inputDevice')].forEach(el=>el.addEventListener('input',()=>{labels();if(lastMeter)renderMeter(lastMeter);}));
// Meter freshness uses the capture host clock, never the phone clock.
document.querySelectorAll('[data-ms]').forEach(b=>b.onclick=()=>{$('latency').value=b.dataset.ms;labels();});
$('refreshInputs').onclick=refreshInputs;
refreshInputs();
$('artwork').onerror=()=>{$('artwork').hidden=true;artworkKey='';};
poll();setInterval(()=>{if(!busy&&!document.hidden&&desktopVisible)poll();},2000);setInterval(meterPoll,200);document.addEventListener('visibilitychange',()=>{if(!document.hidden&&!busy){poll();meterPoll();}});


function ensureInput(id) {
 if (![...$('inputDevice').options].some(option=>option.value===id)) {
  const option=document.createElement('option'); option.value=id; option.textContent='Entrée mémorisée · indisponible'; $('inputDevice').append(option);
 }
}
async function refreshInputs() {
 const button=$('refreshInputs'); button.disabled=true;
 const current=$('inputDevice').value;
 try {
  const data=await api('inputs'); inputDevices=data.inputs||[];
  $('inputDevice').replaceChildren();
  const fallback=document.createElement('option'); fallback.value='default'; fallback.textContent='Entrée par défaut du système'; $('inputDevice').append(fallback);
  for (const input of inputDevices) {
   const option=document.createElement('option'); option.value=input.id; option.textContent=input.name+(input.default?' · par défaut':''); $('inputDevice').append(option);
  }
  ensureInput(current); $('inputDevice').value=current;
  $('inputHelp').textContent=inputDevices.length ? `${data.backend} · ${inputDevices.length} entrée${inputDevices.length>1?'s':''} disponible${inputDevices.length>1?'s':''}. Autorise la capture audio au premier démarrage.` : 'Aucune entrée détectée. Branche une interface audio et actualise.';
 } catch(error) { $('inputHelp').textContent=error.message; }
 finally { button.disabled=false; labels(); }
}
async function loadArtwork(key) {
 try {
  if (!desktop) { $('artwork').src='/api/artwork?v='+encodeURIComponent(key); return; }
  const bytes=await window.__TAURI__.core.invoke('track_artwork');
  if(artworkKey!==key)return;
  if(artworkUrl)URL.revokeObjectURL(artworkUrl);
  artworkUrl=URL.createObjectURL(new Blob([new Uint8Array(bytes)],{type:'image/jpeg'}));
  $('artwork').src=artworkUrl;
 } catch { $('artwork').hidden=true; artworkKey=''; }
}

if(desktop)window.__TAURI__.event.listen('desktop-visible',({payload})=>{
 desktopVisible=payload;
 if(payload){poll();meterPoll();}
});
