import {esc,normalize,move} from './model.js';
import {loadCatalog,catalogCardHtml,siteIconHtml} from './catalog.js';
const $ = s=>document.querySelector(s), app=$('#app');
const icons={gear:'<path d="M9 3h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1z"/><circle cx="12" cy="12" r="3"/>',lock:'<rect x="5" y="10" width="14" height="11" rx="2"/><path d="M8 10V6a4 4 0 018 0v4M12 14v3"/>',refresh:'<path d="M20 7v5h-5M4 17v-5h5M20 12a8 8 0 00-14-5M4 12a8 8 0 0014 5"/>'};
const icon=n=>`<svg viewBox="0 0 24 24" aria-hidden="true">${icons[n]}</svg>`;
const invoke = (cmd,args={}) => window.__TAURI__?.core.invoke(cmd,args) ?? Promise.reject('Abra o aplicativo Tauri. Esta página depende do backend Rust.');
let settings, emulators=[], selected, timer, busy=false, screen=0;
const toast=message=>{ clearTimeout(timer); $('#toast').textContent=message; $('#toast').hidden=false; timer=setTimeout(()=>$('#toast').hidden=true,4000); };
function render(html){screen++;app.innerHTML=html;app.scrollTop=0;}
function error(message){let e=$('#error'); if(e)e.textContent=String(message);else toast(String(message));}
async function task(fn){if(busy)return;busy=true;const controls=[...document.querySelectorAll('button,input,select,textarea')].filter(e=>!e.disabled);controls.forEach(e=>e.disabled=true);try{await fn()}catch(e){error(e)}finally{busy=false;controls.forEach(e=>{if(e.isConnected)e.disabled=false})}}
async function save(next){normalize(next);await invoke('save_settings',{settings:next});settings=next;}
const clone=()=>structuredClone(settings);
const back=()=>'<button class="back" id="back">← Emuladores</button>';
function login(message=''){
 settings=null; emulators=[]; $('#settings').hidden=true;
 render(`<div class="login"><div class="hero">${icon('lock')}</div><div class="eyebrow">SEU ESPAÇO DE CONTROLE</div><h1>Bem-vindo de volta.</h1><p>Insira sua chave para acessar<br>seus emuladores e configurações.</p><form id="login"><label for="license">Chave de autenticação</label><input id="license" type="password" placeholder="Informe sua chave" maxlength="512" autocomplete="off" required><button class="primary wide" type="submit">Acessar aplicativo →</button></form><div id="error" class="error">${esc(message)}</div><div class="hint">A chave será protegida e salva neste usuário do Windows.</div></div>`);
 $('#login').onsubmit=e=>{e.preventDefault();task(async()=>{const r=await invoke('authenticate',{key:$('#license').value});if(!r.authenticated)throw 'Chave inválida. Confira e tente novamente.';settings=await invoke('get_settings');$('#settings').hidden=false;home();});};
}
function home(){
 render(`<div class="row between"><div><div class="eyebrow">WORKSPACE</div><h1>Seus emuladores</h1></div><button id="refresh" class="icon-btn" title="Atualizar lista" aria-label="Atualizar lista">${icon('refresh')}</button></div><p class="small">Instâncias MEmu em execução neste computador.</p><div class="row between"><span class="muted">DISPOSITIVOS ATIVOS</span><span id="count" class="chip">…</span></div><div id="list"><div class="empty">Buscando emuladores…</div></div><div id="error" class="error"></div>`);
 $('#refresh').onclick=()=>task(refresh); refresh().catch(error);
}
async function refresh(){
 const version=screen;const list=$('#list');$('#refresh').disabled=true;$('#error').textContent='';
 try{
 const result=await invoke('list_emulators');if(version!==screen)return;emulators=result;$('#count').textContent=`${result.length} online`;
 list.innerHTML=result.length?result.map(vm=>{const cfg=settings.instances[vm.id]||{};return `<div class="card row"><div class="device"><img src="assets/logos/MEMU.png" alt=""></div><div class="grow"><strong>${esc(cfg.alias||vm.name)}</strong><small><span class="dot"></span>Ativo · PID ${vm.pid} · #${vm.id}</small></div><div class="switch-stack"><div class="switch-field"><span class="switch-label">Injetar token</span><label class="switch" title="Injetar o token automaticamente quando o captcha abrir"><input class="auto-toggle" data-id="${vm.id}" type="checkbox" role="switch" aria-label="Injetar token automaticamente em ${esc(vm.name)}" ${cfg.auto_inject?'checked':''}><span class="slider"></span></label></div><div class="switch-field"><span class="switch-label">Pré-resolver</span><label class="switch" title="${cfg.timing_ready?'Resolver antes da janela abrir, deixando o token pronto':'Precisa de tempo médio (rode algumas captchas)'}"><input class="pre-toggle" data-id="${vm.id}" type="checkbox" role="switch" ${cfg.pre_solve?'checked':''} ${cfg.timing_ready?'':'disabled'} aria-label="Pré-resolver captcha em ${esc(vm.name)}"><span class="slider"></span></label></div></div><button class="icon-btn vm" data-id="${vm.id}" title="Configurar ${esc(vm.name)}" aria-label="Configurar ${esc(vm.name)}">${icon('gear')}</button></div>`}).join(''):'<div class="empty">Nenhum emulador ativo.<br><small>Inicie uma instância MEmu e atualize a lista.</small></div>';
 document.querySelectorAll('.vm').forEach(b=>b.onclick=()=>instance(Number(b.dataset.id)));
 document.querySelectorAll('.auto-toggle').forEach(input=>{input.onchange=()=>task(()=>toggleAutoFromList(Number(input.dataset.id),input))});
 document.querySelectorAll('.pre-toggle').forEach(input=>{input.onchange=()=>task(()=>togglePreSolve(Number(input.dataset.id),input))});
 }catch(e){if(version===screen){$('#count').textContent='—';list.innerHTML='<div class="empty">Não foi possível obter a lista.</div>';error(e)}}finally{if(version===screen)$('#refresh').disabled=false;}
}
function config(){
 render(`${back()}<h1>Configurações</h1><p class="small">Suas conexões, na ordem que você preferir.</p><div class="section-title">Prioridade dos sites · 1 = maior</div><div id="sites"></div><button id="newsite" class="wide">+ Cadastrar site</button><div class="section-title">MEmu neste computador</div><form id="general"><label for="path">Caminho do memuc.exe (opcional)</label><input id="path" value="${esc(settings.memuc_path)}" placeholder="C:\\Microvirt\\MEmu\\memuc.exe"><label for="solver-timeout">Timeout da resolução (segundos)</label><input id="solver-timeout" type="number" min="30" max="1800" value="${esc(settings.solver_timeout||180)}"><div class="section-title">Tempo médio (opcional)</div><label class="inline-check"><input id="use-manual-timing" type="checkbox" ${settings.use_manual_timing?'checked':''}> Usar tempo médio definido manualmente</label><label for="manual-interval">Captcha a cada N segundos</label><input id="manual-interval" type="number" min="0" max="3600" value="${settings.manual_interval_s||0}" placeholder="ex.: 370"><label for="manual-solve">Resolução leva N segundos</label><input id="manual-solve" type="number" min="0" max="3600" value="${settings.manual_solve_s||0}" placeholder="ex.: 35"><div class="hint tight">Libera a pré-resolução sem esperar o cálculo automático. O cálculo real substitui estes valores assim que houver amostras.</div><button type="submit" class="wide primary">Salvar ajustes</button></form><div class="notice">As API-keys são usadas para resolver o captcha quando a injeção automática está ligada.</div><div id="error" class="error"></div><div class="separator"></div><button id="logout" class="wide danger">Sair e esquecer chave de acesso</button>`);
 $('#back').onclick=home;$('#newsite').onclick=newSite;$('#logout').onclick=()=>task(async()=>{await invoke('logout');login()});
 $('#general').onsubmit=e=>{e.preventDefault();task(async()=>{const n=clone();n.memuc_path=$('#path').value.trim();n.solver_timeout=Math.min(1800,Math.max(30,Number($('#solver-timeout').value)||180));n.manual_interval_s=Math.max(0,Number($('#manual-interval').value)||0);n.manual_solve_s=Math.max(0,Number($('#manual-solve').value)||0);n.use_manual_timing=$('#use-manual-timing').checked;await save(n);toast('Ajustes salvos.');});};
 const ordered=[...settings.priority,...settings.sites.map(s=>s.id).filter(id=>!settings.priority.includes(id))];
 $('#sites').innerHTML=ordered.map(id=>{
 const site=settings.sites.find(s=>s.id===id), keys=settings.keys.filter(k=>k.site_id===id), pos=settings.priority.indexOf(id);
 return `<section class="card"><div class="row">${siteIconHtml(site)}<div class="grow"><strong>${esc(site.name)}</strong><small>${keys.length} chave(s) cadastrada(s)</small></div><button class="icon-btn add" data-site="${esc(id)}" title="Adicionar API-key" aria-label="Adicionar API-key">+</button></div>${keys.length?`<div class="row between"><small>Prioridade ${pos+1}</small><div class="tools"><button class="up" data-site="${esc(id)}" ${pos===0?'disabled':''} aria-label="Aumentar prioridade">↑</button><button class="down" data-site="${esc(id)}" ${pos===settings.priority.length-1?'disabled':''} aria-label="Diminuir prioridade">↓</button></div></div>`:''}${keys.map(k=>`<div class="key"><small class="grow">${esc(k.label||'API-key')} · ••••••••</small><button class="edit" data-key="${esc(k.id)}" title="Substituir chave">Editar</button><button class="remove danger" data-key="${esc(k.id)}" title="Remover chave" aria-label="Remover chave">×</button></div>`).join('')}<div class="key"><button class="delete-site danger" data-site="${esc(id)}">Excluir site</button></div></section>`;
 }).join('');
 document.querySelectorAll('.add').forEach(b=>b.onclick=()=>keyForm(b.dataset.site));
 document.querySelectorAll('.edit').forEach(b=>b.onclick=()=>{const k=settings.keys.find(k=>k.id===b.dataset.key);keyForm(k.site_id,k)});
 document.querySelectorAll('.remove').forEach(b=>b.onclick=()=>task(async()=>{const n=clone();n.keys=n.keys.filter(k=>k.id!==b.dataset.key);await save(n);config();toast('Chave removida.')}));
 document.querySelectorAll('.delete-site').forEach(b=>b.onclick=()=>confirmDelete(b.dataset.site));
 for(const [cls,delta] of [['up',-1],['down',1]])document.querySelectorAll('.'+cls).forEach(b=>b.onclick=()=>task(async()=>{await save(move(clone(),b.dataset.site,delta));config()}));
}
function confirmDelete(id){const site=settings.sites.find(s=>s.id===id);render(`<h1>Excluir site?</h1><p>${esc(site.name)} e suas chaves serão removidos. Os emuladores vinculados voltarão à prioridade global.</p><button id="cancel">Cancelar</button> <button id="confirm" class="danger">Excluir</button><div id="error" class="error"></div>`);$('#cancel').onclick=config;$('#confirm').onclick=()=>task(async()=>{const n=clone();n.keys=n.keys.filter(k=>k.site_id!==id);n.sites=n.sites.filter(s=>s.id!==id);await save(n);config()});}
function keyForm(siteId,existing){
 const site=settings.sites.find(s=>s.id===siteId);
 const slug=siteId.replace(/-/g,'').toLowerCase();
 const needsProxy=slug.startsWith('onyx');
 render(`<button class="back" id="back">← Configurações</button><h1>${existing?'Substituir':'Adicionar'} API-key</h1><p>${esc(site.name)}</p><form id="keyform"><label for="label">Identificação da chave</label><input id="label" maxlength="100" value="${esc(existing?.label||'')}" placeholder="Ex.: Conta principal"><label for="secret">${existing?'Nova API-key':'API-key'}</label><input id="secret" type="password" maxlength="4096" required autocomplete="off" placeholder="Cole sua chave aqui">${needsProxy?`<label for="proxy">Proxy (obrigatório para ${esc(site.name)})</label><input id="proxy" spellcheck="false" value="${esc(existing?.proxy||'')}" placeholder="usuario:senha@host:porta"><div class="hint tight">Formato sem esquema, ex.: <code>user:pass@host:port</code>. O provedor exige proxy para hCaptcha.</div>`:''}<label class="inline-check"><input type="checkbox" id="show"> Mostrar chave digitada</label><button class="primary wide">Salvar chave</button></form><div id="error" class="error"></div><div class="hint">A chave não será enviada ao site. A ordem dos sites pode ser alterada nas configurações.</div>`);
 $('#back').onclick=config;$('#show').onchange=e=>$('#secret').type=e.target.checked?'text':'password';
 $('#keyform').onsubmit=e=>{e.preventDefault();task(async()=>{const secret=$('#secret').value.trim();if(!secret)throw 'Informe a API-key.';const n=clone();const proxyField=$('#proxy');const proxy=proxyField?proxyField.value.trim():'';if($('#proxy')&&!proxy)throw 'Informe o proxy deste provedor.';const k={id:existing?.id||crypto.randomUUID(),site_id:siteId,label:$('#label').value.trim(),secret,proxy};if(existing)n.keys=n.keys.map(x=>x.id===existing.id?k:x);else n.keys.push(k);await save(n);config();toast('API-key salva com proteção do Windows.')})};
}
// Catálogo de sites (ui/sites.json): cards com ícone; clique pede a API-key.
async function newSite(){
 render(`${back()}<h1>Cadastrar site</h1><p class="small">Escolha o site. A API-key será solicitada em seguida.</p><div id="catalog"><div class="empty">Carregando catálogo…</div></div><div id="error" class="error"></div><div class="notice">O catálogo vem de <code>ui/sites.json</code>. Cadastrar um site não implementa sua API.</div>`);
 $('#back').onclick=config;
 try{
  const entries=await loadCatalog();
  const saved=new Set(settings.sites.map(s=>s.id));
  $('#catalog').innerHTML=entries.map(entry=>catalogCardHtml(entry,saved.has(entry.id))).join('');
  document.querySelectorAll('.catalog-item').forEach(button=>button.onclick=()=>task(async()=>{
   const entry=entries.find(e=>e.id===button.dataset.site);
   if(!entry)throw 'Site não encontrado no catálogo.';
   await registerSite(entry);
  }));
 }catch(e){$('#catalog').innerHTML='<div class="empty">Catálogo indisponível.</div>';error(e)}
}
// Garante o site no catálogo local (sem duplicar) e abre o formulário de chave.
async function registerSite(entry){
 const existing=settings.sites.find(s=>s.id===entry.id);
 if(!existing){
  const n=clone();
  n.sites.push({id:entry.id,name:entry.name,icon:entry.icon||''});
  await save(n);
 }
 keyForm(entry.id);
}
function instance(id){
 selected=id;const vm=emulators.find(v=>v.id===id), value=settings.instances[id]||{};
 render(`${back()}<div class="eyebrow">INSTÂNCIA #${id}</div><h1>${esc(vm.name)}</h1><p class="small">PID ${vm.pid} · Configuração individual no aplicativo</p><form id="instance"><label for="alias">Nome de exibição</label><input id="alias" maxlength="100" value="${esc(value.alias||'')}" placeholder="${esc(vm.name)}"><label for="preferred">Site preferencial</label><select id="preferred"><option value="">Usar prioridade global</option>${settings.priority.map(id=>`<option value="${esc(id)}" ${id===value.site_id?'selected':''}>${esc(settings.sites.find(s=>s.id===id).name)}</option>`).join('')}</select><button class="wide primary">Salvar configuração</button></form><button id="android" class="wide">Abrir configurações do Android ↗</button><div id="error" class="error"></div><div class="hint">Preferências salvas por índice MEmu. Site preferencial preparado para integração futura.</div>`);

 $('#instance').onsubmit=e=>{e.preventDefault();task(async()=>{
  const n=clone(), current=n.instances[selected]||{};
  n.instances[selected]=Object.assign({},current,{
   alias:$('#alias').value.trim(),
   site_id:$('#preferred').value,
  });
  await save(n);home();toast('Configuração do emulador salva.');
 })};
 $('#back').onclick=home;
 $('#android').onclick=()=>task(async()=>{await invoke('open_android_settings',{id});toast('Configurações abertas no emulador.')});
}
// Liga/desliga o worker automático de um emulador a partir do switch da lista.
// O arquivo do token é global (Configurações); aqui só se controla o worker.
async function toggleAutoFromList(id,input){
 const n=clone(), current=n.instances[id]||{alias:'',site_id:''};
 n.instances[id]=Object.assign({},current,{auto_inject:input.checked});
 await save(n);
 try{
  const status=input.checked
   ?await invoke('start_auto_inject',{vmId:id,enableDebug:true})
   :await invoke('stop_auto_inject',{vmId:id});
  toast(input.checked
   ?`Injeção automática ativa${status&&status.running?'':' (aguardando)'} · ${status.injections||0} injeção(ões)`
   :'Injeção automática desativada.');
  if(input.checked)watchAuto(id,input);
 }catch(e){
  input.checked=false;
  const s=clone();s.instances[id]=Object.assign({},s.instances[id],{auto_inject:false});
  await save(s);
  error(e);
 }
}
// Liga/desliga a pré-resolução (resolve antes da janela abrir).
async function togglePreSolve(id,input){
 const n=clone(), current=n.instances[id]||{};
 n.instances[id]=Object.assign({},current,{pre_solve:input.checked});
 await save(n);
 try{
  const status=await invoke('set_pre_solve',{vmId:id,enabled:input.checked});
  toast(input.checked
   ?`Pré-resolução ativa · resolução média ${(status.solve_mean_s||0).toFixed(0)}s · captcha a cada ${(status.interval_mean_s||0).toFixed(0)}s`
   :'Pré-resolução desativada.');
 }catch(e){
  input.checked=false;
  const s=clone();s.instances[id]=Object.assign({},s.instances[id],{pre_solve:false});
  await save(s);
  error(e);
 }
}
// Atualiza o card com a etapa atual, contadores pass/fail e último tempo.
function watchAuto(id,input){
 const title=input.closest('.switch');
 const card=input.closest('.card');
 if(title)title.title='Verificando o WebView a cada segundo…';
 setTimeout(async()=>{
  if(!input.isConnected||!input.checked)return;
  try{
   const status=await invoke('auto_inject_status',{vmId:id});
   const stage=(status.stage_label||'Aguardando captcha');
   if(title)title.title=status.last_error
    ?`${stage} · erro: ${status.last_error}`
    : (status.last_result?`${stage} · ${status.last_result}`:stage);
   if(card){
    let line=card.querySelector('.auto-stage');
    if(!line){
     line=document.createElement('div');
     line.className='auto-stage';
     card.querySelector('.grow').appendChild(line);
    }
    const passed=status.passes||0, failed=status.failures||0;
    const elapsed=status.last_elapsed_ms!=null?` · ${status.last_elapsed_ms} ms`:'';
    line.textContent=`${stage} · ✓ ${passed} · ✕ ${failed}${elapsed}`;
    if(status.last_error)line.textContent+=` · ${status.last_error}`;
    if(status.pre_solve&&status.prepared){
     const left=status.prepared_age_s!=null?` (${Math.round(status.prepared_age_s)}s)`:'';
     line.textContent+=` · token pronto${left}`;
    }
    // O segundo switch só ativa quando já existe tempo médio calculado.
    const preInput=card.querySelector('.pre-toggle');
    if(preInput){
     const ready=status.interval_mean_s!=null&&status.solve_mean_s!=null;
     preInput.disabled=!ready;
     const label=preInput.closest('.switch');
     if(label)label.title=ready
      ?`Pré-resolve ~${(status.solve_mean_s||0).toFixed(0)}s antes (captcha a cada ~${(status.interval_mean_s||0).toFixed(0)}s)`
      :'Precisa de tempo médio: deixe a injeção rodar algumas captchas';
    }
    line.dataset.stage=status.stage||'watching';
    line.title=status.history&&status.history.length
      ? status.history.slice(0,5).map(r=>`${r.at} ${r.passed?'pass':'fail'} ${r.provider} ${r.elapsed_ms}ms ${r.detail}`).join('\n')
      : '';
   }
  }catch(e){/* instância pode ter mudado de tela */}
  if(input.isConnected&&input.checked)watchAuto(id,input);
 },1000);
}
// Reinicia os workers ativos (usado quando o arquivo de token muda).
async function restartAutoWorkers(){
 const running=emulators.filter(vm=>settings.instances[vm.id]?.auto_inject);
 for(const vm of running){
  try{await invoke('stop_auto_inject',{vmId:vm.id})}catch(e){}
  try{await invoke('start_auto_inject',{vmId:vm.id,enableDebug:true})}
  catch(e){error(e);const n=clone();n.instances[vm.id]=Object.assign({},n.instances[vm.id],{auto_inject:false});await save(n)}
 }
}
$('#settings').innerHTML=icon('gear');$('#settings').onclick=config;
for(const action of ['minimize','close'])$('#'+action).onclick=()=>window.__TAURI__?.window.getCurrentWindow()[action]().catch(error);
(async()=>{try{$('#mode').textContent='MEmu Hub';const r=await invoke('authenticate',{key:null});if(r.authenticated){settings=await invoke('get_settings');$('#settings').hidden=false;home()}else login()}catch(e){login(String(e))}})();
