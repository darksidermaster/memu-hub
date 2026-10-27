const {chromium}=require('playwright');
const http=require('http'),fs=require('fs'),path=require('path'),os=require('os');
(async()=>{
 const root=path.resolve(__dirname,'../ui');
 const server=http.createServer((req,res)=>{const f=path.join(root,req.url==='/'?'index.html':req.url);const mime=f.endsWith('.js')?'application/javascript':f.endsWith('.css')?'text/css':f.endsWith('.svg')?'image/svg+xml':f.endsWith('.webp')?'image/webp':f.endsWith('.png')?'image/png':f.endsWith('.json')?'application/json':'text/html';res.setHeader('Content-Type',mime);res.setHeader('Content-Security-Policy',"default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; frame-src 'none'");try{res.end(fs.readFileSync(f))}catch{res.statusCode=404;res.end()}}).listen(8767,'127.0.0.1');
 const browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:350,height:450}});const errors=[];page.on('pageerror',e=>errors.push(e.message));page.on('console',m=>{if(/Content Security Policy|Refused to/i.test(m.text()))errors.push('CSP: '+m.text())});page.on('requestfailed',r=>{if(/sites\.json|assets\//.test(r.url()))errors.push('requestfailed: '+r.url())});
 await page.addInitScript(()=>{
 const s={memuc_path:'',adb_path:'',package_name:'com.away.game.br',demo_emulators:false,solver_timeout:180,manual_interval_s:0,manual_solve_s:0,use_manual_timing:false,solver_proxy:'',sites:[{id:'a',name:'Site A (exemplo)',icon:'A'},{id:'b',name:'Site B (exemplo)',icon:'B'}],keys:[],priority:[],instances:{}};
 let config=s,saved='',injectArgs=null;
 window.__injectCalls=[];window.__autoCalls=[];
 window.__TAURI__={core:{invoke:async(cmd,args)=>{
  switch(cmd){
   case'is_demo':return true;
   case'authenticate':if(args.key!==null)saved=args.key;return {authenticated:saved==='DEMO-2026',demo:true};
   case'get_settings':return structuredClone(config);
   case'save_settings':config=structuredClone(args.settings);return;
   case'list_emulators':return [{id:0,name:'MEmu',pid:1234},{id:1,name:'MEmu 2',pid:4567}];
   case'list_sessions':window.__lastSessionsArgs=args;return [
     {vm_id:0,vm_name:'MEmu',serial:'127.0.0.1:21503',model:'MEmu',app_running:true,app_pid:3217,socket:'webview_devtools_remote_3217',pages:[{id:'P1',title:'MuAwaY',url:'https://captcha.muaway.net/android.html?sitekey=x',captcha:true}],error:null},
     {vm_id:1,vm_name:'MEmu 2',serial:'127.0.0.1:21513',model:'MEmu',app_running:true,app_pid:4321,socket:null,pages:[],error:'WebView sem socket DevTools.'}
   ];
   case'start_auto_inject':{window.__autoCalls.push({cmd,args});window.__autoRunning=true;
     const hasKeys=config.keys&&config.keys.length;
     if(!hasKeys&&!config.token_file)throw 'Cadastre uma API-key de um site suportado (ou informe um arquivo de token) antes de ativar.';
     return {running:true,vm_id:args.vmId,token_file:hasKeys?'solvers:'+config.keys.length:config.token_file,attempts:1,injections:0,last_result:null,last_error:null};}
   case'stop_auto_inject':{window.__autoCalls.push({cmd,args});window.__autoRunning=false;
     return {running:false,vm_id:args.vmId,token_file:'',attempts:3,injections:1,last_result:'token de 10 chars injetado',last_error:null};}
   case'auto_inject_status':return window.__autoRunning
     ?{running:true,vm_id:args.vmId,token_file:'solvers:1',stage:'injected',stage_label:'Token injetado',
       attempts:2,passes:1,failures:1,injections:1,last_elapsed_ms:1234,
       pre_solve:!!window.__autoPreSolve,prepared:false,prepared_age_s:null,
       interval_mean_s:window.__autoTimingReady?64:null,solve_mean_s:window.__autoTimingReady?12:null,timing_samples:window.__autoTimingReady?3:0,
       last_result:'token de 10 chars injetado',last_error:null,
       history:[{at:'2026-10-24T07:00:00Z',provider:'nonecap',elapsed_ms:1234,passed:true,token_length:10,detail:'pass: true'},
                {at:'2026-10-24T06:59:00Z',provider:'nonecap',elapsed_ms:900,passed:false,token_length:0,detail:'pass: false — erro'}]}
     :{running:false,vm_id:args.vmId,token_file:'',stage:null,stage_label:'',attempts:0,passes:0,failures:0,
       injections:0,last_elapsed_ms:null,last_result:null,last_error:null,history:[]};
   case'set_pre_solve':{window.__autoCalls.push({cmd,args});
     if(args.enabled&&!window.__autoTimingReady)throw 'A pré-resolução só pode ser ativada depois de algumas captchas: ainda não há tempo médio de intervalo.';
     return {running:true,vm_id:args.vmId,pre_solve:args.enabled,prepared:false,
       prepared_age_s:null,interval_mean_s:64,solve_mean_s:12,timing_samples:3};}
   case'logout':saved='';return;
   case'open_android_settings':return;
   default:throw cmd;
  }}},window:{getCurrentWindow:()=>({minimize:async()=>{},close:async()=>{}})}};
 window.__getInjectArgs=()=>injectArgs;window.__getConfig=()=>config;
 });
 await page.goto('http://127.0.0.1:8767');await page.locator('#license').fill('errada');await page.locator('#login button').click();await page.getByText('Chave inválida.',{exact:false}).waitFor();
 await page.locator('#license').fill('DEMO-2026');await page.locator('#login button').click();await page.locator('.vm').first().waitFor();
 if(await page.locator('#captcha-all').count())throw Error('botão de injetar em todos deveria ter sido removido');await page.screenshot({path:path.join(os.tmpdir(),'memu-hub-home-qa.png')});
 // A configuração da instância não tem mais Observações nem captcha manual.
 await page.locator('.vm').first().click();
 // ← Emuladores precisa voltar para a lista.
 await page.locator('#back').click();
 await page.locator('.vm').first().waitFor();
 if(await page.locator('#alias').count())throw Error('back não voltou para a lista de emuladores');
 await page.locator('.vm').first().click();
 if(await page.locator('#notes').count())throw Error('campo Observações deveria ter sido removido');
 if(await page.locator('#captcha-response').count())throw Error('bloco de captcha manual deveria ter sido removido');
 if(await page.locator('#captcha-check').count())throw Error('verificação manual deveria ter sido removida');
 if(await page.locator('#auto-switch').count())throw Error('switch não deveria ficar dentro da configuração');
 await page.locator('#alias').fill('Instância principal');
 await page.locator('#instance button').click();
 await page.getByText('Instância principal',{exact:true}).waitFor();
 // Switch na lista, ao lado da engrenagem do card.
 const rowSwitch=page.locator('#list .auto-toggle').first();
 if(!await rowSwitch.count())throw Error('switch deveria estar no card do emulador');
 const gearBox=await page.locator('#list .vm').first().boundingBox();
 const switchBox=await rowSwitch.boundingBox();
 if(!gearBox||!switchBox)throw Error('layouts ausentes');
 if(switchBox.y+switchBox.height/2 > gearBox.y+gearBox.height+8)throw Error('switch deveria alinhar com a engrenagem');
 // Sem API-key nem arquivo: recusa e desliga.
 await rowSwitch.click({force:true});
 await page.getByText(/Cadastre uma API-key/i).waitFor();
 if(await rowSwitch.isChecked())throw Error('switch não deveria ligar sem credencial');
 // Cadastra uma API-key do catálogo (NoneCap) e tenta de novo.
 await page.locator('#settings').click();
 await page.locator('#newsite').click();
 await page.locator('.catalog-item[data-site="nonecap"]').click();
 await page.locator('#secret').fill('nc_test_key');
 await page.locator('#keyform button').click();
 await page.locator('#back').click();
 await page.locator('#list .auto-toggle').first().click({force:true});
 await page.getByText(/Injeção automática ativa/).waitFor();
 // O card exibe a etapa atual e os contadores pass/fail com o tempo.
 await page.waitForFunction(()=>{
  const el=document.querySelector('.auto-stage');
  return el && /Token injetado/.test(el.textContent) && /✓ 1/.test(el.textContent)
    && /✕ 1/.test(el.textContent) && /1234 ms/.test(el.textContent);
 },null,{timeout:8000});
 const stageTitle=await page.locator('.auto-stage').first().getAttribute('title');
 if(!stageTitle||!stageTitle.includes('pass'))throw Error('histórico deveria estar no tooltip');
 let calls=await page.evaluate(()=>window.__autoCalls);
 const startCall=calls.find(c=>c.cmd==='start_auto_inject');
 if(!startCall)throw Error('start_auto_inject não foi chamado');
 if(startCall.args.vmId!==0)throw Error('vmId errado no start');
 if('tokenFile' in startCall.args)throw Error('tokenFile não deve ser enviado pela UI (é global)');
 if(startCall.args.enableDebug!==true)throw Error('o switch deve habilitar a depuração automaticamente');
 const keysCount=await page.evaluate(()=>window.__getConfig().keys.length);
 if(keysCount!==1)throw Error('API-key do catálogo não foi salva');
 const persisted=await page.evaluate(()=>Boolean(window.__getConfig().instances[0].auto_inject));
 // Segundo switch (pré-resolução), abaixo do switch de injeção.
 const preSwitch=page.locator('#list .pre-toggle').first();
 if(!await preSwitch.count())throw Error('switch de pré-resolução ausente');
 if(!await preSwitch.isDisabled())throw Error('pré-resolução deve começar desabilitada sem tempo médio');
 // Os dois switches precisam estar na MESMA div, um abaixo do outro, cada um
 // com um rótulo acima.
 const stack=page.locator('#list .switch-stack').first();
 if(!await stack.count())throw Error('os switches deveriam ficar em uma div .switch-stack');
 if(await stack.locator('.auto-toggle').count()!==1)throw Error('switch de injeção fora da div');
 if(await stack.locator('.pre-toggle').count()!==1)throw Error('switch de pré-resolução fora da div');
 const labels=await stack.locator('.switch-label').allTextContents();
 if(labels.length!==2)throw Error('esperava 2 rótulos, achei '+labels.length);
 if(!labels[0].includes('Injetar'))throw Error('primeiro rótulo deveria ser Injetar: '+labels[0]);
 if(!labels[1].includes('resolver'))throw Error('segundo rótulo deveria ser Pré-resolver: '+labels[1]);
 const autoBox=await page.locator('#list .auto-toggle').first().boundingBox();
 const preBox=await preSwitch.boundingBox();
 const labelBoxes=await stack.locator('.switch-label').evaluateAll(els=>els.map(e=>e.getBoundingClientRect().top));
 if(!autoBox||!preBox)throw Error('layout do switch ausente');
 if(preBox.y <= autoBox.y)throw Error('pré-resolução deveria ficar abaixo do switch de injeção');
 if(!(labelBoxes[0] < autoBox.y))throw Error('rótulo deveria ficar acima do primeiro switch');
 if(!(labelBoxes[1] < preBox.y))throw Error('rótulo deveria ficar acima do segundo switch');
 // Simula tempo médio já calculado: o switch habilita no próximo poll (1 s).
 await page.evaluate(()=>{window.__autoTimingReady=true});
 await page.waitForFunction(()=>{
  const el=document.querySelector('.pre-toggle');
  return el && !el.disabled;
 },null,{timeout:15000});
 await page.locator('#list .pre-toggle').first().click({force:true});
 await page.getByText(/Pré-resolução ativa/).waitFor();
 let calls2=await page.evaluate(()=>window.__autoCalls);
 const preCall=calls2.find(c=>c.cmd==='set_pre_solve');
 if(!preCall||preCall.args.enabled!==true)throw Error('set_pre_solve não foi chamado com enabled=true');
 const preSaved=await page.evaluate(()=>Boolean(window.__getConfig().instances[0].pre_solve));
 if(!preSaved)throw Error('estado do switch de pré-resolução não persistiu');
 if(!persisted)throw Error('estado do switch não persistiu na instância');
 await page.screenshot({path:path.join(os.tmpdir(),'memu-hub-auto-qa.png')});
 // O card não pode estourar a largura de 350 nem cortar os rótulos.
 const cardOverflow=await page.evaluate(()=>document.documentElement.scrollWidth>350);
 if(cardOverflow)throw Error('overflow horizontal no card com os switches');
 const stackBox=await page.locator('#list .switch-stack').first().boundingBox();
 if(!stackBox||stackBox.x+stackBox.width>350)throw Error('switch-stack fora da largura');
 const labelClipped=await page.locator('#list .switch-label').evaluateAll(els=>els.some(e=>e.scrollWidth>e.clientWidth+1));
 if(labelClipped)throw Error('rótulo do switch cortado');
 // Desliga.
 await page.locator('#list .auto-toggle').first().click({force:true});
 await page.getByText(/Injeção automática desativada/).waitFor();
 calls=await page.evaluate(()=>window.__autoCalls);
 if(!calls.find(c=>c.cmd==='stop_auto_inject'))throw Error('stop_auto_inject não foi chamado');
 await page.locator('#settings').click();
 if(await page.locator('#customsite').count())throw Error('botão de site personalizado ainda existe');
 await page.getByText('Prioridade 1',{exact:true}).waitFor();
 // Adiciona uma segunda chave ao MESMO site (NoneCap) e confere que não duplica.
 const nonecapCard=page.locator('#sites > section').filter({hasText:'NoneCap'});
 await nonecapCard.locator('.add').click();
 await page.locator('#label').fill('Conta 2');
 await page.locator('#secret').fill('nc_second_key');
 await page.locator('#keyform button').click();
 const nonecapKeys=await page.locator('#sites > section').filter({hasText:'NoneCap'}).locator('.key .remove').count();
 if(nonecapKeys!==2)throw Error('deveria haver 2 chaves no mesmo site, há '+nonecapKeys);
 // Segunda chave em outro site para exercitar a prioridade.
 // Proxy por credencial: só aparece no formulário do Onyx, não nas Configurações.
 if(await page.locator('#solver-proxy').count())throw Error('proxy não deveria ficar nas configurações gerais');
 await page.locator('.add').nth(0).click();
 await page.locator('#secret').fill('sonic_test_key');
 await page.locator('#keyform button').click();
 if(await page.locator('.up').count()===0)throw Error('controles de prioridade ausentes');
 await page.screenshot({path:path.join(os.tmpdir(),'memu-hub-settings-qa.png')});
 const beforeRemoval=await page.locator('.key .remove').count();
 await page.locator('.remove').first().click();
 if(await page.locator('.key .remove').count()!==beforeRemoval-1)throw Error('Removal failed');
 // catálogo: cards com ícone -> pede API-key
 await page.locator('#newsite').click();
 await page.locator('.catalog-item').first().waitFor();
 await page.screenshot({path:path.join(os.tmpdir(),'memu-hub-catalog-qa.png')});
 const cards=await page.locator('.catalog-item').count();
 if(cards<2)throw Error('catálogo deveria listar os sites do sites.json');
 if(!await page.locator('.catalog-item img').first().getAttribute('src').then(s=>s.includes('assets/logos/')))throw Error('ícone do catálogo não renderizou');
 const iconOk=await page.locator('.catalog-item img').first().evaluate(img=>img.complete&&img.naturalWidth>0);
 if(!iconOk)throw Error('imagem do ícone não decodificou');
 const cardBox=await page.locator('.catalog-item').first().boundingBox();
 if(!cardBox||cardBox.width>316||cardBox.height<44)throw Error('card do catálogo fora do layout: '+JSON.stringify(cardBox));
 const catalogOverflow=await page.evaluate(()=>document.documentElement.scrollWidth>350);
 if(catalogOverflow)throw Error('overflow horizontal no catálogo');
 const catalogNames=await page.locator('.catalog-item .catalog-name strong').allTextContents();
 // Só considera "livre" o site sem NENHUMA chave no estado salvo (a tela do
 // catálogo não mostra a lista de sites, então lê do config).
 const saved=await page.evaluate(()=>window.__getConfig());
 const hasKeys=(name)=>{
  const site=(saved.sites||[]).find(s=>s.name===name);
  if(!site)return false;
  return (saved.keys||[]).some(k=>k.site_id===site.id);
 };
 const cleanIndex=catalogNames.findIndex(name=>!hasKeys(name));
 if(cleanIndex<0)throw Error('nenhum site do catálogo está livre para o teste');
 const cleanName=catalogNames[cleanIndex];
 const firstName=catalogNames[cleanIndex];
 await page.locator('.catalog-item').nth(cleanIndex).click();
 await page.locator('#secret').waitFor();
 if(!(await page.locator('h1').textContent()).includes('API-key'))throw Error('clique no card deveria pedir a API-key');
 if(!(await page.locator('p.small, p').first().textContent()).includes(cleanName))throw Error('formulário deveria mostrar o site escolhido');
 await page.locator('#secret').fill('catalog-key-only');
 await page.locator('#keyform button').click();
 await page.getByText('Prioridade 1',{exact:true}).waitFor();
 if(!await page.locator('#sites').textContent().then(t=>t.includes(cleanName)))throw Error('site do catálogo não foi salvo');
 // segundo clique no mesmo site adiciona outra chave sem duplicar o site
 const sectionsBeforeSecond=await page.locator('#sites > section').count();
 const siteSection=page.locator('#sites > section').filter({hasText:cleanName});
 await siteSection.first().waitFor();
 const keysBeforeSecond=await siteSection.locator('.key .remove').count();
 await page.locator('#newsite').click();
 await page.locator('.catalog-item').nth(cleanIndex).click();
 await page.locator('#secret').fill('catalog-key-second');await page.locator('#keyform button').click();
 const sectionsAfterSecond=await page.locator('#sites > section').count();
 if(sectionsAfterSecond!==sectionsBeforeSecond)throw Error('site do catálogo foi duplicado');
 const sameSite=await page.locator('#sites > section').filter({hasText:cleanName}).count();
 if(sameSite!==1)throw Error('site do catálogo deveria aparecer uma única vez');
 const keysAfterSecond=await siteSection.locator('.key .remove').count();
 if(keysBeforeSecond!==1){
  const dump=await page.locator('#sites').textContent();
  const sections=await page.locator('#sites > section').count();
  const adds=await page.locator('#sites .add').count();
  throw Error(`primeira chave não apareceu (antes=${keysBeforeSecond}, depois=${keysAfterSecond}, firstName=${cleanName}, sections=${sections}, adds=${adds}) html=${dump.slice(0,400)}`);
 }
 if(keysAfterSecond!==2)throw Error('segunda chave não foi adicionada ao mesmo site');
 if(await page.locator('#customsite').count())throw Error('opção de site personalizado deveria ter sido removida');
 const catalogImgs=await page.locator('#sites img[src*="assets/logos/"]').count();
 if(catalogImgs<1)throw Error('ícone do site do catálogo deveria aparecer nas configurações');
 const firstSiteSection=page.locator('#sites > section').filter({hasText:cleanName});
 if(!await firstSiteSection.locator('img[src*="assets/logos/"]').count())throw Error('o site cadastrado pelo catálogo deveria exibir o ícone');
 const overflow=await page.evaluate(()=>document.documentElement.scrollWidth>350);if(overflow)throw Error('Horizontal overflow');
 await page.locator('#logout').click();await page.locator('#license').waitFor();if(errors.length)throw Error(errors.join('\n'));
 console.log('PASS: login, listing, dois switches na mesma div com rótulos (posição, bloqueio sem tempo médio, ativação e persistência), config sem Observações/captcha manual, token global, catálogo, prioridade, logout, layout 350x450.');
 await browser.close();server.close();
})().catch(e=>{console.error(e);process.exit(1)});
