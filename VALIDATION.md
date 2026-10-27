# Verificação do pacote

Executado neste ambiente (Windows 10/11, Rust 1.96.0 MSVC, Node 22.16.0, Python 3.13.5):

## Testes automatizados

- `node --check ui/app.js`, `ui/model.js`, `ui/captcha.js`: passaram.
- `npm test`: **27 testes** passaram (14 originais + 13 do catálogo).
  - anteriores: prioridade, remoção da última chave, escape HTML;
  - novos: parsing da resposta do captcha (`pass`, `generated_pass_UUID`,
    chaves alternativas, token cru), recusa de `pass:false`/token ausente/JSON
    inválido, preview sem expor o token, descrição do resultado, filtro de
    páginas de captcha.
- `npm run test:ui`: **passou**. Cobre login inválido/válido, listagem,
  configuração da instância, verificação e injeção de captcha (backend
  simulado, incluindo o JSON normalizado enviado ao Rust), resposta inválida
  (`pass:false`), "injetar em todos" com um sucesso e uma falha, cadastro e
  remoção de chaves, reordenação de prioridade, escape de HTML, logout e
  largura de 350 px.
- `cargo test`: **14 testes** passaram.
  - parser MEmu (nomes com vírgula, instâncias desligadas);
  - `parse_token`: JSON completo, token cru, chaves alternativas, `pass:false`,
    token ausente, resposta grande;
  - geração do JS de injeção com token escapado por `serde_json` (sem injeção
    de código), presença de `AndroidBridge.handleCallback`, modo sem preencher
    campos;
  - `/json/list`: parsing, filtro por host e recusa de JSON inválido;
  - inferência de VM a partir do serial, porta livre, erro de conexão HTTP.
- `cargo check` e `cargo build`: compilam sem erros (usar `-j 1` neste
  ambiente; o build paralelo estourou a pilha do `rustc`).

## Validado ao vivo com o MEmu

Ambiente: MEmu 9.5.6, Android 9 (API 28), x86_64, app `com.away.game.br`
aberto, Chromium WebView 116 com socket `webview_devtools_remote_15963`,
`frida-server` 16.7.19.

- `memu-hub.exe sessions`:
  - encontrou a VM `1` (`MEmu1`), serial `127.0.0.1:21513` (a chave
    `InstallLocation` do Registro foi necessária: o MEmu está em `D:\`);
  - PID atual `15963`, socket `webview_devtools_remote_15963`;
  - página `https://captcha.muaway.net/android.html?...` com título `MuAwaY`
    marcada como `captcha: true`.
- `memu-hub.exe inject --serial 127.0.0.1:21513 --file resposta.json` com o
  token real de teste:

```json
{
  "injected": true,
  "bridge_present": true,
  "bridge_called": true,
  "fields_filled": ["g-recaptcha-response", "h-captcha-response"],
  "readback_ok": true,
  "token_length": 2371,
  "elapsed_ms": 542,
  "error": null
}
```

- O endpoint `/json/list` do WebView Android mantém a conexão aberta mesmo com
  `Connection: close`; o leitor HTTP em Rust foi ajustado para respeitar
  `Content-Length` e finalizar por timeout, e passou a devolver a lista.
- O forward criado para a consulta é removido ao final (confirmado com
  `adb forward --list`).

## Correções aplicadas após teste do usuário

Sintoma relatado: ao verificar o WebView aparecia
`WebView sem socket DevTools...` mesmo com a opção Frida marcada.

Causas e correções:

1. **A verificação ignorava o checkbox.** `checkCaptcha` chamava
   `list_sessions` sem argumento e o comando nem aceitava o flag. Agora
   `list_sessions(enable_debug)` repassa para `webview::sessions`, que habilita
   a depuração quando o socket está ausente. A UI envia `enableDebug` e exibe
   "depuração habilitada" no status. Coberto pelo teste de UI.
2. **`frida-tools` 14.x era incompatível com `frida` 16.7.19**, produzindo
   `TypeError: not a function` / `getGlobalExportByName is not a function`.
   Alinhado para `frida-tools==13.7.1` e documentado.
3. **O script Rust checava o resultado antes da UI thread executar.**
   `Java.scheduleOnMainThread` apenas enfileira; o script agora espera o
   callback (limite de 5 s) e reporta `timeout aguardando a UI thread` quando
   aplicável.
4. O `frida-server` agora é iniciado automaticamente quando existe no aparelho,
   e o PID é relido após a instrumentação (o app pode reiniciar).

Validado ao vivo após as correções, com o app recém-reiniciado (sem socket):

```
MEMU_HUB_ENABLE_DEBUG=1 memu-hub.exe sessions
→ pid 6982, socket webview_devtools_remote_6982, debug_enabled: true, error: null
```

Com o socket já presente (PID 6394), a mesma chamada listou a página
`https://captcha.muaway.net/android.html` como `captcha: true`.
`inject --enable-debug` também percorreu o caminho até a checagem de páginas
("Nenhuma página aberta em captcha.muaway.net"), provando que a habilitação
funcionou naquele ciclo.

## Catálogo de sites (`ui/sites.json`)

Implementado e testado:

- `ui/catalog.js`: leitura do JSON, normalização do caminho do ícone
  (`.\\assets\\logos\\x.svg` → `assets/logos/x.svg`), rejeição de caminho
  absoluto/`..`/URL/caracteres inválidos, slug estável (`CapSonic` →
  `capsonic`) com sufixo em colisão, ícone textual de até 2 caracteres,
  fallback para a inicial do nome, escape de HTML e cards do catálogo.
- `ui/app.js`: **+ Cadastrar site** lista os cards com ícone; o clique garante o
  site no catálogo local (sem duplicar) e abre o formulário de API-key. O mesmo
  card pode receber várias chaves. O cadastro manual de site foi **removido** a
  pedido do usuário: o catálogo é a única origem dos sites.
- `scripts/validate_sites.py`: valida `ui/sites.json` e a existência dos
  arquivos de ícone dentro de `ui/`. Execução real:
  `OK CapSonic ... (6911 bytes)`, `OK NoneCap ... (368 bytes)`, `0 problema(s)`.

Dois problemas reais encontrados e corrigidos no caminho:

1. **CSP bloqueava o `fetch('sites.json')`.** `connect-src` não incluía
   `'self'`; sem isso o catálogo nunca carregaria dentro do Tauri. O teste de UI
   agora serve exatamente o mesmo CSP e falha em violações.
2. **Havia um `style="margin-top:0"` inline**, bloqueado por
   `style-src 'self'`. Trocado por uma classe `.small.tight`.

- `npm test`: **27 testes** (13 novos para o catálogo).
- `npm run test:ui`: passa sob CSP real, cobrindo cards, ícone decodificado
  (`naturalWidth > 0`), clique → formulário de API-key, site não duplicado ao
  adicionar a segunda chave, escape de HTML e ausência de overflow horizontal.
- `python scripts/validate_sites.py`: 2 sites, 0 problemas.

Não validado: abrir a janela Tauri real com o catálogo (o WebView2 não expôs a
porta de depuração neste ambiente, então a verificação foi feita no Chromium
com o mesmo CSP e no binário compilado). Também não foram testados ícones além
de SVG/webp.

## Correção: "Site inválido ou duplicado" ao cadastrar

Relato: ao clicar em um site do catálogo para cadastrar a chave, aparecia
`Site inválido ou duplicado.`, mesmo sem nenhum site cadastrado.

Causa: `check_settings` em `src-tauri/src/main.rs` validava o ícone com
`site.icon.chars().count() > 2`. Isso funcionava para o antigo ícone textual,
mas o catálogo passou a usar caminhos como `assets/logos/capsonic.svg`, que têm
muito mais de 2 caracteres. Toda gravação era recusada pelo backend Rust.

Por que não foi pego antes: o teste de UI usa um backend **simulado**
(`window.__TAURI__.core.invoke`), então `save_settings` e sua validação nunca
eram executados.

Correções:

1. `check_icon` aceita **texto de até 2 caracteres** ou **caminho relativo em
   `assets/`**, espelhando a validação de `ui/catalog.js` (normaliza `\` e
   `./`, recusa `..`, caminho absoluto, URL e caracteres inválidos).
2. As mensagens de erro passaram a citar o site e o motivo
   (`O ícone do site 'X' é inválido...`), em vez do genérico
   "Site inválido ou duplicado.".
3. `+ Site personalizado` foi removido da interface (botão, handler e função).
4. Testes Rust de `check_settings`: ícone de catálogo aceito, ícone textual
   aceito, caminhos inseguros recusados, duas chaves no mesmo site aceitas,
   site duplicado recusado.
5. Novo modo CLI `memu-hub.exe validate-settings` (lê `Settings` em JSON do
   stdin e executa o **mesmo** `check_settings`), usado por
   `scripts/validate_settings_flow.py`.

Execução real de `scripts/validate_settings_flow.py` (contra o binário Rust):

```
OK   catálogo com ícone de arquivo: exit=0 válido
OK   duas chaves no mesmo site: exit=0 válido
OK   dois sites do catálogo: exit=0 válido
OK   ícone textual legado: exit=0 válido
OK   ícone inválido (>2 chars): exit=1 O ícone do site 'A' é inválido...
OK   caminho fora de assets: exit=1 O ícone do site 'A' é inválido...
OK   site duplicado: exit=1 O site 'CapSonic' está duplicado no catálogo.
7 caso(s), 0 problema(s).
```

`cargo test`: 24 testes passando. `npm run test:ui` passa e agora verifica que
duas chaves ficam no mesmo site e que `#customsite` não existe mais.

## Injeção automática (switch na lista)

Alterações pedidas: remover o painel "Captcha do WebView" da configuração da
instância, remover "Injetar token para todos os emuladores", remover
"Observações" e colocar um **switch** de injeção automática ao lado da
engrenagem da instância.

Implementado:

- `src-tauri/src/auto_inject.rs`: `AutoInjector` com uma thread por emulador.
  Verifica a cada **1 segundo** (`POLL_INTERVAL`): lê o arquivo de token, resolve
  o PID atual, procura o socket DevTools, abre um forward temporário, consulta
  `/json/list` e tenta `AndroidBridge.handleCallback(token)` apenas nos alvos
  `captcha.muaway.net`. Se a página ainda estiver carregando, tenta de novo no
  segundo seguinte. O forward é sempre removido. `AtomicBool` encerra a thread e
  `Drop` garante o join.
- Token em arquivo **global** (`Settings.token_file`), configurado em
  Configurações, porque o mesmo token serve a todos os emuladores. O arquivo é
  relido em cada tentativa; `expiration` é comparado com o mtime do arquivo.
  Mudar o caminho reinicia os workers ativos.
- Comandos Tauri: `start_auto_inject`, `stop_auto_inject`, `auto_inject_status`.
  Os antigos `inject_captcha` / `inject_captcha_all` e o `InjectReport` foram
  removidos, assim como o modo CLI `inject` (substituído por `auto-inject`).
- UI: switch no card do emulador (ao lado da engrenagem), estado salvo em
  `InstanceConfig.auto_inject`, título do switch atualizado com o resultado do
  `auto_inject_status`. `InstanceConfig.notes` removido.
- `ui/captcha.js` e seus testes foram removidos: a validação do token agora é
  feita no Rust (`webview::parse_token`).

Validação ao vivo com o MEmu (Android 9, app `com.away.game.br`, Chromium 116):

```
memu-hub.exe auto-inject --vm 1 --file muaway-response.json --seconds 20
{
  "running": true,
  "vm_id": 1,
  "attempts": 1,
  "injections": 1,
  "last_result": "token de 2371 chars injetado em 'MuAwaY' (2 campo(s))",
  "last_error": null
}
```

O worker detectou a página na primeira verificação e injetou o token real sem
intervenção manual. `cargo test` com 30 testes, `npm test` com 16,
`npm run test:ui` passando (inclui alinhamento do switch com a engrenagem,
recusa sem arquivo, start/stop e persistência), `clippy` sem avisos,
`scripts/validate_sites.py` e `scripts/validate_settings_flow.py` sem problemas.

Não validado: a janela Tauri real (o WebView2 não expôs porta de depuração
neste ambiente; a UI foi verificada no Chromium com o mesmo CSP, e o worker foi
verificado pelo binário Rust). Também não foi testado ligar dois switches ao
mesmo tempo com duas VMs reais.

## Resolução do captcha por API (solver)

Antes, o worker só injetava um token vindo de arquivo. Agora ele resolve o
captcha pelos provedores cadastrados, conforme o `full_solve.js` do projeto
`E:\yolo\cap-http`.

Implementado em `src-tauri/src/solver.rs`:

- **NoneCap** (`/lib/nonecap.js`): reimplementado em Rust com `ureq`.
  `POST https://api.nonecap.com/v1/solves?wait=60` com `Bearer <key>`, corpo
  `{type:"hcaptcha",sitekey,url}`, depois `GET /solves/{id}?wait=N` com backoff
  até `status = solved`; o token vem em `solve.token`. Erros 401/402/403/422 e
  o motivo final são propagados.
- **CaptchaSonic** (`full_solve.js --solver sonic`): executado como subprocesso
  com `CAPSONIC_KEY` no ambiente; o JSON impresso após `FINAL:` é lido por
  `parse_sonic_output`. Não foi reescrito em Rust porque depende do protocolo
  hCaptcha + PoW (`hsw`) em Chrome headless.
- `solve_with_priority` tenta os provedores na ordem de `settings.priority`,
  devolvendo o primeiro token; se todos falharem, o erro lista cada motivo.
- O token só é pedido quando a página do captcha já está aberta, para não gastar
  crédito de API sem necessidade. O mesmo token não é reinjetado na mesma página.

### Sitekey: existem dois

O usuário apontou que o jogo usa mais de um sitekey. Confirmado no ambiente:

- `06bbf0e3-4583-44df-ad7e-61c028c50e38` — hardcoded no `full_solve.js`;
- `ef8c43cb-a59d-461f-a710-239b13569c4d` — presente na URL
  `android.html?sitekey=...`, que é o que o widget realmente usa hoje.

Correção: `url_param()` extrai `sitekey` da URL da página aberta (com
percent-decoding) e esse valor tem prioridade. A constante `webview::SITEKEY`
foi atualizada para `ef8c43cb-...` e serve apenas como fallback. Dois testes
cobrem a preferência e o fallback.

Validação ao vivo com o MEmu (app real, página do captcha aberta), usando a
`NONECAP_KEY` do `.env` do `cap-http`:

```
memu-hub.exe auto-inject --vm 1 --seconds 150
{
  "running": true,
  "vm_id": 1,
  "token_file": "solvers:1",
  "attempts": 1,
  "injections": 1,
  "last_result": "token de 2376 chars injetado em 'MuAwaY' (2 campo(s))",
  "last_error": null
}
```

A resolução pela NoneCap e a injeção via `AndroidBridge` aconteceram no mesmo
ciclo, sem token manual. Repetido após a troca do sitekey padrão, com o mesmo
resultado (2376 chars).

- `cargo test`: 42 testes; `clippy` sem avisos.
- `npm test`: 16; `npm run test:ui`: passa (API-key do catálogo → switch liga).
- `scripts/validate_sites.py` e `scripts/validate_settings_flow.py`: sem problemas.

Não validado: CaptchaSonic executado a partir do binário Rust (o caminho
NoneCap foi validado ao vivo; o Sonic depende de Chrome headless e não foi
disparado aqui). Também não foi testado esgotamento de créditos nem falha de
rede durante a resolução.

## Etapas, contadores e log de tentativas

Implementado:

- `AutoStatus` ganhou `stage`/`stage_label` (enum `Stage`: Watching, Detecting,
  Solving, Injecting, Injected, Failed), `passes`, `failures`,
  `last_elapsed_ms` e `history` (últimas 20 tentativas).
- `try_inject` marca as etapas, mede o tempo da resolução e registra cada
  tentativa (pass/fail) em `AttemptRecord`.
- Log append-only em `%LOCALAPPDATA%\MEmuHub\captcha-attempts.jsonl` com
  `at`, `vm`, `provider`, `elapsed_ms`, `passed`, `token_length`, `detail`.
- Card do emulador: `Etapa · ✓ pass · ✕ fail · último tempo`, com tooltip das
  últimas tentativas.

Execução real (MEmu, página do captcha aberta, NoneCap):

```
{
  "stage": "injected", "stage_label": "Token injetado",
  "attempts": 1, "passes": 1, "failures": 0, "injections": 1,
  "last_elapsed_ms": 21278,
  "last_result": "token de 2355 chars injetado em 'MuAwaY' (2 campo(s), 21278 ms)"
}
```

Linha correspondente no log:

```json
{"at":"2026-10-24T07:09:55Z","vm":1,"provider":"nonecap","elapsed_ms":21278,"passed":true,"token_length":2355,"detail":"pass: true"}
```

## CaptchaSonic reimplementado em Rust

O que foi feito (novos `src/sonic.rs` e `src/hsw.rs`):

- protocolo hCaptcha em Rust: `checksiteconfig`, formulário msgpack de 11
  campos, `getcaptcha`, `checkcaptcha` e montagem de `answers`;
- `hsw` (PoW + criptografia) executado num Chrome/Edge headless controlado por
  CDP, com `awaitPromise`; **sem Node e sem puppeteer**;
- mapeamento `request_type` → `questionType` e respostas para
  classificação/click/drag;
- challenge version lida de `js.hcaptcha.com` com fallback;
- fallback para o `full_solve.js` quando o nativo não cobre o caso.

Causas raiz encontradas durante a validação (todas verificadas byte a byte
contra o payload do script oficial):

1. **`415 Unsupported Media Type`** — o segundo item do envelope msgpack deve
   ser **ext type 18 (Uint8Array)**; `bin16` é recusado. Corrigido com encoder
   próprio (`wrap_getcaptcha`).
2. **`Utf8String::to_string()` inclui aspas** — as chaves JSON saíam como
   `"\"pass\""`. Corrigido usando `as_str()`.
3. **`415` no cliente HTTP** — os POSTs migraram de `ureq` para `reqwest`,
   com runtime próprio (o modo CLI não tem runtime ativo).
4. **`403` nas imagens** do CDN — faltavam `Referer` e `Sec-Fetch-*`; também
   foi adicionada a tentativa com extensão (`.webm`/`.png`).
5. **PoW retornando `{}`** — `Runtime.evaluate` precisa de
   `awaitPromise: true` para o `hsw` (que é assíncrono).

Resultado ao vivo: o pipeline nativo **passou do `getcaptcha`** (sem 415) e
baixou as mídias. O desafio apresentado no momento do teste era de **vídeo**
(`ERROR_VIDEO_PAYLOAD_REQUIRED`), que exige ffmpeg + análise de frames; nesse
caso o app usa o `full_solve.js`. O caminho Sonic completo foi validado pelo
script via app (token real de 2122 chars em ~27 s) e o NoneCap foi validado
100% nativo.

## NoCaptchaAI (porte em Rust)

Documentação consultada: <https://docs.nocaptchaai.com/docs> (`llms.txt` e
`llms-full.txt`). API: `https://api.nocaptchaai.com`, autenticação por
`clientKey` no corpo, fluxo assíncrono `createTask` → `getTaskResult`.

Implementado em `solver.rs`:

- `SolverKind::Nocaptchaai` (`nocaptchaai`/`nocaptcha` no catálogo);
- `solve_nocaptchaai`: `HCaptchaTaskProxyless` com `websiteURL`/`websiteKey`,
  poll com backoff até `status: ready`, token de `solution.token`;
- classificação dos códigos oficiais (`nocaptcha_retryable`,
  `nocaptcha_error_name`) e respeito a `retryAfterSec`;
- leitura dos dois formatos de erro observados: `{code,msg}` (documentado) e
  `{error,errorId}` (conta sem plano);
- CLI: `solve-test --provider nocaptchaai` e `NOCAPTCHAAI_KEY` em `auto-inject`.

Validação com a chave fornecida (`...149454`, saldo US$ 0,00464, sem plano):

```
POST /createTask  HCaptchaTaskProxyless  -> {"errorId":0,"status":"idle","taskId":"..."}
POST /getTaskResult (poll 8x)            -> {"errorId":0,"status":"idle", ...}
POST /createTask (tentativa seguinte)    -> {"error":"You do not have a paid plan.", "errorId":1}
```

Comparação com uma tarefa documentada (`AntiTurnstileTask`) usando a mesma
chave: `status: "processing"` — a API responde, mas hCaptcha não é liberado
para essa conta. Conclusão: o cliente está correto e testado, porém **não foi
possível obter um token real de hCaptcha** com a chave disponível.

Testes: 8 casos novos (mapeamento do provedor, retry/códigos, formatos de erro,
chave vazia). `cargo test`: 59; `clippy` limpo.

## Surfsky (avaliado, não implementado)

Documentação analisada: <https://docs.surfsky.io/category/use-cases>,
`/captcha-solving`, `/llms.txt` e `/llms-full.txt` (inclui o OpenAPI).

Conclusão: **não dá para portar como NoneCap/NoCaptchaAI**, por três motivos
verificados:

1. **Não é uma API de token.** Surfsky é um navegador Chrome na nuvem com
   solvers embutidos, comandados por `Captcha.solve` / `Captcha.autoSolve` num
   CDP de página. O resultado é consumido na própria sessão do browser; não há
   endpoint que devolva `generated_pass_UUID` para injetar no WebView do MEmu.
2. **hCaptcha não está entre os tipos suportados.** A lista é Turnstile,
   reCAPTCHA, GeeTest v4, AWS WAF, BLS, PerimeterX, DataDome, FunCaptcha, click
   e image-to-text. Busca nos docs completos e no OpenAPI: `hcaptcha` = 0
   ocorrências.
3. **A base URL é por conta.** O OpenAPI declara `servers: [{url: "{baseUrl}",
   variables: {baseUrl: {default: "https://YOUR_API_HOST"}}}]` e a doc diz que
   host errado retorna `403 namespace_not_allowed`. Testes com o token
   fornecido em `app.surfsky.io` (e variações `/api`, `/v1`, `/api/v1`)
   retornaram **404**; `api.surfsky.io` nem resolve em DNS.

O token informado não pôde ser validado porque a URL da API da conta não foi
fornecida (ela consta apenas no dashboard). Mesmo com ela, o item 2 impede o
uso para o hCaptcha do MuAwaY.

Para evitar falha silenciosa, foi adicionado `solver::unsupported_sites`: ao
ativar o switch só com chaves de sites sem solver (`surfsky`, `2captcha`), o
erro lista os sites e informa os suportados. Coberto por dois testes.

## CapSolver (porte em Rust)

Documentação: <https://docs.capsolver.com/en/guide/api-createtask/>,
`/api-gettaskresult`, `/api-getbalance`, `/api-support/`, `/api-tasktype/`.
O repositório indicado (`javapuppteernodejs/hcaptcha-solver`) foi verificado e
tem **apenas um README** (5 KB, nenhum código) — é um artigo sobre hCaptcha, não
uma implementação.

Implementado em `solver.rs` (`SolverKind::Capsolver`):

- `capsolver_balance`: valida a chave via `getBalance` sem criar tarefa;
- `solve_capsolver`: `createTask` com `HCaptchaClassification` +
  `websiteURL`/`websiteKey`, `getTaskResult` com backoff, extração do token
  (`token`, `gRecaptchaResponse`, `captchaToken`, `text`);
- `capsolver_retryable` (RATE_LIMIT, SERVICE_OVERLOAD, NO_SLOT_AVAILABLE,
  TASK_TIMEOUT) e `capsolver_unsupported` (reconhece "not support",
  "don't support", "under maintenance", `ERROR_TYPE_NOT_SUPPORTED`);
- CLI: `solve-test --provider capsolver --balance` e `CAPSOLVER_KEY` em
  `auto-inject`.

### Resultado real com a chave fornecida

```
solve-test --provider capsolver --balance
-> saldo: US$ 5.9984

POST /createTask ImageToTextTask (controle)
-> 200 {"errorId":0,"status":"ready","solution":{"text":"g7x4k2",...}}

POST /createTask HCaptchaTaskProxyless
-> 400 ERROR_INVALID_TASK_DATA "We don't support this service."

POST /createTask HCaptchaEnterpriseTask (+proxy)
-> 400 "This service is not supported: HCaptchaEnterpriseTask"

POST /createTask HCaptchaClassification
-> 400 "HCaptchaClassification is under maintenance"

POST /getToken HCaptchaTaskProxyless
-> 400 ERROR_TYPE_NOT_SUPPORTED "this type does not support getToken"

solve-test --provider capsolver --sitekey ef8c43cb-...
-> erro: CapSolver não resolve hCaptcha nesta conta:
   HCaptchaClassification is under maintenance. Use NoneCap, NoCaptchaAI ou CaptchaSonic.
```

Conclusão: **a CapSolver não entrega token de hCaptcha** nesta conta nem em
nenhuma variante de nome. O cliente está implementado, testado e integrado; a
limitação é do provedor, não do código. Nenhum crédito foi consumido nas
tentativas de hCaptcha (todas falharam na validação, antes de cobrar); apenas o
`ImageToTextTask` de controle gerou custo mínimo.

Testes: 6 novos (mapeamento do provedor, classificação de erros/retry, detecção
de "não suportado" com as mensagens reais, formato de erro, extração de token,
chave vazia). `cargo test`: 67; `clippy` limpo.

## 2Captcha (porte em Rust)

Documentação: <https://2captcha.com/api-docs> (v2) e
<https://2captcha.com/2captcha-api> (v1). O SDK oficial
(`2captcha/2captcha-python`) foi inspecionado: o método `hcaptcha()` chama
`solve(..., method='hcaptcha')`, isto é, **API v1** (`in.php`/`res.php`). A v2
não tem página de hCaptcha na navegação.

Implementado em `solver.rs` (`SolverKind::Twocaptcha`):

- `solve_twocaptcha`: `POST /in.php` com `method=hcaptcha`, `sitekey`,
  `pageurl`, `json=1`; poll em `GET /res.php?action=get&id=...` a cada 5 s
  (recomendação da doc); token lido de `request`;
- `twocaptcha_balance`: `action=getbalance` (não passa pela validação de IP);
- `twocaptcha_retryable` (NO_SLOT_AVAILABLE, SERVER_ERROR, BAD_GATEWAY) e
  `twocaptcha_error_hint`, que traduz erros em ação: chave, saldo, IP liberado,
  sitekey/URL, captcha insolúvel;
- CLI: `solve-test --provider 2captcha --balance` e `TWOCAPTCHA_KEY` em
  `auto-inject`.

### Resultado real com a chave fornecida

```
solve-test --provider 2captcha --balance
-> saldo: US$ 14.93831

GET /res.php?action=getbalance&json=1
-> {"status":1,"request":"14.93831"}

POST /in.php (method=hcaptcha)
-> {"status":0,"request":"ERROR_IP_NOT_ALLOWED",
    "error_text":"The request is sent from the IP that is not on the list of your allowed IPs."}

solve-test --provider 2captcha --sitekey ef8c43cb-...
-> 2Captcha in.php: ERROR_IP_NOT_ALLOWED
   A conta tem whitelist de IP: libere este IP no painel do 2Captcha
   (aba de configurações da API).
```

Chave válida, saldo presente, serviço suporta hCaptcha — mas a conta tem
**whitelist de IP** e esta máquina não está nela. É configuração de conta, não
limitação do código: liberando o IP no painel, o mesmo binário resolve.

Testes: 5 novos (mapeamento do provedor, retry, hints de erro, parsing de
`status`/`request`, chave vazia) + ajuste do teste de "sites sem solver", que
antes usava `2captcha` como exemplo e agora usa `surfsky`/`captchakings`.
`cargo test`: 71; `clippy` limpo.

## AntiCaptcha (portado e validado end-to-end)

O `send_key.py` original usa `anticaptchaofficial.hCaptchaProxyless`, que envia
`HCaptchaTaskProxyless` para `https://api.anti-captcha.com/createTask`. O cliente
nativo reproduz o mesmo fluxo em Rust, sem o SDK Python.

Implementado em `solver.rs` (`SolverKind::Anticaptcha`, id do catálogo
`anti-captcha`):

- `solve_anticaptcha`: `createTask` → poll `getTaskResult` até `status: ready`,
  token de `solution.gRecaptchaResponse`, com backoff de 5 s até 10 s;
- `anticaptcha_balance`: `getBalance` (valida a chave sem criar tarefa);
- `anticaptcha_retryable` (NO_SLOT_AVAILABLE, SERVER_ERROR) e
  `anticaptcha_error_hint` (chave, saldo, IP, tipo não suportado, sitekey/URL,
  captcha insolúvel);
- CLI: `solve-test --provider anticaptcha [--balance]` e `ANTICAPTCHA_KEY` no
  `auto-inject`.

### Resultado real

```
POST /getBalance
-> {"errorId":0,"balance":0.05,"captchaCredits":50}

POST /createTask HCaptchaTaskProxyless
-> {"errorId":0,"taskId":762517488}

POST /getTaskResult (poll a cada 5 s)
-> status=processing ... (13 vezes)
-> status=ready  errorId=0
   solution.gRecaptchaResponse = "P1_eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...."
   solution.userAgent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64..."
```

Pelo binário Rust:

```
solve-test --provider anticaptcha --balance
-> saldo: US$ 0.04700

solve-test --provider anticaptcha --sitekey ef8c43cb-...
-> provider anticaptcha, token_length 2482, prefixo P1_eyJhbGciO,
   expiration 120, elapsed 68160 ms, exit 0
```

É o **segundo provedor validado end-to-end** (depois do NoneCap) e o único que
entrega token `P1_...` de hCaptcha sem Chrome e sem exigir liberação de IP. O
token foi consumido do saldo real da conta de teste.

Testes: 5 novos (mapeamento do provedor, retry, hints, formato de erro,
extração de `gRecaptchaResponse`, chave vazia). `cargo test`: 76; `clippy`
limpo; `validate_sites.py`: 5 sites, 0 problemas.

### Observação sobre o `send_key.py`

O script é para o **WebView2 do Windows** e injeta via
`window.chrome.webview.postMessage("hcaptcha=" + token)`. No Android/WebView do
MEmu o canal é outro: `AndroidBridge.handleCallback(token)`. Por isso o
`send_key.py` não é reutilizável no app Rust — apenas o fluxo de resolução
(anti-captcha → 2captcha como fallback) foi portado.

## Correção: switch preso em "Aguardando captcha"

Sintoma: ao ativar o switch, o card ficava em
`Aguardando captcha · ✓ 0 · ✕ 0` e nenhuma tarefa era enviada.

Duas causas, ambas confirmadas no ambiente:

1. **A UI enviava `enableDebug: false` fixo.** Sem socket DevTools — o caso do
   processo 2755, reiniciado sem Frida — o worker não conseguia consultar
   `/json/list`, nunca detectava a página e nunca chamava o solver.
   Corrigido: o switch e o restart de workers enviam `enableDebug: true`.
2. **O worker silenciava a falha.** No ramo sem socket e sem permissão de
   depuração havia `return Ok(None)`, indistinguível de "não há captcha aberto".
   Agora devolve erro explícito, exibido no card:
   `WebView sem socket DevTools: marque 'Habilitar depuração via Frida' ...`

O card também passou a mostrar `last_error` na linha de status (antes só no
tooltip), então a causa fica visível sem passar o mouse.

Validação ao vivo a partir de estado sem socket (PID 2755):

```
auto-inject --vm 1 --enable-debug --seconds 45
-> stage: solving, attempts: 1     (Frida habilitou e a página foi encontrada)

auto-inject --vm 1 --enable-debug --seconds 150
-> stage: injected
   last_result: "token de 2168 chars injetado em 'MuAwaY' (2 campo(s), 132351 ms)"
   attempts: 1, passes: 1, failures: 0, injections: 1
   history: [{provider: "anticaptcha", elapsed_ms: 132351, passed: true,
              token_length: 2168, detail: "pass: true"}]
   exit 0
```

Habilitou a depuração, detectou o WebView, resolveu pela AntiCaptcha e injetou o
token. `npm run test:ui` ganhou asserção de que o switch envia
`enableDebug: true`. `cargo test`: 76; `clippy` limpo.

## Correção: resolvia captcha com a janela fechada

Sintoma: depois de fechar a janela do WebView, o worker continuava resolvendo.

Investigação no ambiente (PID 2755, app em primeiro plano):

- o target **continua** em `/json/list` depois de fechar a janela;
- `document.visibilityState` continua `"visible"` e `document.hidden` é `false`;
- `performance.timeOrigin` **não muda** nem no `location.reload()`;
- o que muda é o **campo de resposta**: ele permanece preenchido com o token
  injetado (`textarea len: 2198`).

Ou seja, `visibilityState` e `timeOrigin` não servem como sinal; o campo
preenchido serve.

Implementado em `cdp.rs`:

- `PageState { time_origin, answered, widget }` e `PAGE_STATE_JS`, que lê
  `timeOrigin`, se há token no campo e se o widget está no DOM;
- `page_state(port, target, timeout)` consulta via CDP **antes** de chamar o
  solver;
- `should_solve(&state)` → `!state.answered`. Simples de propósito: uma captcha
  nova na mesma página (que mantém o `timeOrigin`) volta com o campo vazio e
  precisa ser resolvida; usar `timeOrigin` como bloqueio quebraria esse caso.

`try_inject` agora retorna `Ok(None)` sem gastar crédito quando o campo já está
respondido.

Validação ao vivo (AntiCaptcha, mesma conta):

```
# campo vazio (captcha nova) -> resolve
auto-inject --vm 1 --enable-debug --seconds 25
-> stage: solving, attempts: 1, injections: 0

# campo preenchido (janela fechada, token de 2198 chars) -> NÃO resolve
auto-inject --vm 1 --enable-debug --seconds 25
-> stage: watching, attempts: 0, passes: 0, injections: 0, exit 1
```

O segundo caso é exatamente o relatado: a página ainda está em `/json/list`,
mas nenhuma tentativa é feita e nenhum crédito é consumido.

Testes: 4 casos em `cdp` (campo respondido, campo vazio, campo ausente e
"captcha nova na mesma página"), incluindo a regressão de `timeOrigin` não
bloquear uma captcha nova. `cargo test`: 80; `clippy` limpo.

## Pré-resolução preditiva (segundo switch)

Pedido: um switch abaixo do de injeção que resolva o captcha antes da janela
abrir, deixando o token esperando; o cálculo deve usar o tempo médio de injeção
junto ao tempo de resolução, considerando que o token expira em 2 minutos; o
switch só ativa com tempo médio definido.

Implementado em `auto_inject.rs`:

- `Timing`: janela móvel de 20 amostras, guardando duração das resoluções e
  instantes de detecção. Expõe `solve_mean`, `interval_mean`, `margin`,
  `ready`, `lead_seconds` e `should_pre_solve`;
- `margin`: desvio-padrão amostral das resoluções, **limitado a 50% da média**
  (mínimo 1 s). O teto evita que um outlier de 68 s (visto no ambiente)
  antecipe a resolução em mais de um minuto;
- `PreparedToken` com TTL de 120 s e margem de segurança de 10 s
  (`is_valid`, `seconds_left`);
- `pre_solve()`: só roda quando a página ainda **não** está aberta, pede o token
  ao provedor e guarda; não injeta;
- `try_inject` consome o token preparado quando a página aparece, sem chamar a
  API; token vencido é descartado e a resolução acontece normalmente;
- switch em `AtomicBool` (`set_pre_solve`), permitindo ligar/desligar com o
  worker rodando;
- `set_pre_solve` recusa ativar sem `interval_mean_s` — a UI também deixa o
  switch `disabled`, mas a regra vale no backend.

Cálculo com os tempos reais medidos neste ambiente:

```
resoluções: 9.1, 9.8, 13.2, 17.5, 21.8, 68.6 s
média ......... 23.3 s
desvio ........ 22.7 s
margem ........ 11.7 s (teto 50% da média)
antecipação ... 35.0 s
intervalo ..... 368 s (dados reais do log)
disparo ....... 333 s após a última captcha
token pronto .. ~35 s de espera, ~85 s de validade restante
```

Com resolução mais longa que o intervalo, `trigger` é 0 e a pré-resolução
começa imediatamente.

Testes (10 novos, todos sem rede): `ready` exige amostras dos dois lados;
médias; margem limitada; disparo perto da chegada prevista; disparo imediato
quando a resolução excede o intervalo; janela limitada a 20 amostras;
detecções duplicadas em <1 s ignoradas; TTL do token preparado; e o caso de
`timeOrigin` não bloquear captcha nova (mantido da correção anterior).

`cargo test`: 90; `clippy` limpo; `npm run test:ui` ganhou asserções de que o
segundo switch fica **abaixo** do primeiro, começa **desabilitado** sem tempo
médio, habilita quando o status traz as médias, chama `set_pre_solve` e persiste
o estado.

Não validado ao vivo: um ciclo completo de pré-resolução com espera real de
minutos entre captchas. A conta de teste tem 47 créditos; a validação do
caminho de resolução e injeção já foi feita, e a lógica de tempo está coberta por
testes determinísticos.

## Layout: dois switches na mesma div, com rótulos

Alteração pedida: colocar os dois switches na mesma div, um abaixo do outro, cada
um com uma label acima indicando a função.

Implementado em `ui/app.js` + `ui/style.css`:

```
.card.row
├── .device            (ícone do telefone)
├── .grow              (nome/apelido + PID)
├── .switch-stack      <-- mesma div para os dois switches
│   ├── .switch-field  <span.switch-label>Injetar token</span> + .switch(.auto-toggle)
│   └── .switch-field  <span.switch-label>Pré-resolver</span>  + .switch(.pre-toggle)
└── .icon-btn.vm       (engrenagem)
```

- `.switch-stack`: `flex-direction: column`, `gap: 6px`, `flex-shrink: 0`;
- `.switch-label`: 9px, caixa alta, cor discreta, `white-space: nowrap`;
- switches ligeiramente menores (34×19) para caberem com os rótulos;
- o estilo antigo do switch inline (`.switch-inline`) foi removido do markup e
  neutralizado no CSS.

O switch de pré-resolução continua `disabled` até haver tempo médio, e a dica do
`title` explica o motivo.

Verificação (`npm run test:ui`) com asserções novas:

- existe **uma** `.switch-stack` contendo exatamente 1 `.auto-toggle` e 1
  `.pre-toggle`;
- existem 2 `.switch-label`, o primeiro contendo "Injetar" e o segundo
  "resolver";
- o `y` do switch de pré-resolução é **maior** que o do switch de injeção
  (abaixo);
- cada rótulo tem `top` menor que o `top` do seu switch (acima);
- sem overflow horizontal em 350 px e sem rótulo cortado
  (`scrollWidth > clientWidth`).

`npm test`: 16; `npm run test:ui`: passa; build Rust sem erros;
`validate_sites.py`: 5 sites, 0 problemas.

## NoCaptchaAI: teste com chave de teste

Chave testada: `<chave NoCaptchaAI>` (conta de teste).

### A chave funciona — mas não para hCaptcha token

```
GET /balance?apiKey=...
-> {"balance":0,"hasActivePlan":true,"status":"ok",
    "plan":{"active":1,"type":"daily","planType":"daily",
            "dailyLimit":3000,"dailyLimitUsed":0,
            "start":"2026-10-14","end":"2026-11-13"}}

POST /createTask  {"type":"ImageToTextTask","images":[base64]}
-> 200 {"errorId":0,"status":"ready","solution":{"text":["G7X4K2"]}}

POST /solve  {"method":"ocr","images":{"0":base64}}
-> 200 {"status":"solved","solution":{"0":"G7X4K2"}}
```

A chave autentica e resolve OCR corretamente (o texto da imagem de teste bate).
O plano diário está ativo até 13/11/2026 com 0 de 3000 usados.

### hCaptcha token não está disponível

```
POST /createTask  {"type":"HCaptchaTaskProxyless", sitekey, url}
-> 402 {"error":"You do not have a paid plan...","errorId":1,"reTryAfter":600}

POST /solve  {"clientKey":..., "type":"hcaptcha", sitekey, url}
-> 400 {"code":23,"msg":"ERROR_TASK_NOT_SUPPORTED",
        "supportedTypes":["PopularCaptchaImage","ReCaptchaV2Image","GeetestImage",
                          "AwsWafImage","OcrImage","AntiTurnstileTask",
                          "AntiTurnstileTaskProxyLess","AntiCloudflareTask"]}
```

A própria API publica a lista de tipos suportados, e **não há hCaptcha token**.
O enum de `type` no OpenAPI do `createTask` confirma:

```
AceCaptchaTask, BLSCaptchaTask, ReCaptchaV2TaskProxyLess,
CloudflareTurnstileTaskProxyLess, TurnstileTaskProxyLess, ImageToTextTask,
AWSWAFTask, GeeTestTaskProxyLess, MTCaptchaTask, ProsopoTask, TikTokTask,
CaptchaFoxTask, ReCaptchaV3TaskProxyLess, ClassificationTask
```

Nenhum `HCaptchaTask*`. A busca por "hcaptcha" no documento inteiro do OpenAPI
retorna **0 ocorrências**; a palavra aparece apenas na FAQ, e na descrição do
`/solve` como "hCaptcha (grid, bbox, drag)" — isto é, **classificação de
imagem**, não emissão de token.

### Sobre o `/solve` citado

A página `/docs/api/solver/solveCaptcha` descreve o endpoint síncrono e menciona
hCaptcha em `method: "hcap"` com `images`, mas:

- é classificação (retorna índices/coordenadas), não token `P1_...`;
- na prática a API respondeu `ERROR_TASK_NOT_SUPPORTED` mesmo com o payload do
  exemplo da doc, e `hcap` não está em `supportedTypes`.

O app precisa de **token** (`generated_pass_UUID`), então esse endpoint não
serve. Mesmo se a classificação funcionasse, seria preciso montar o desafio e o
PoW do hCaptcha manualmente — o que o CaptchaSonic nativo já faz melhor.

### Token tasks dão 402 apesar do plano ativo

`AntiTurnstileTaskProxyLess` (documentado) também retorna
`You do not have a paid plan`, embora `/balance` mostre `hasActivePlan: true`.
Há inconsistência entre o plano reportado e o que libera token tasks; é do lado
do provedor. O app agora reporta isso de forma explícita:

```
nocaptchaai: 1 KEY_DOES_NOT_EXIST: You do not have a paid plan... (não repetir)
(se o painel mostra plano ativo, este tipo de tarefa não está disponível:
 hCaptcha não consta em /solve supportedTypes)
```

### Código

- `solver::nocaptchaai_balance`: valida a chave via `GET /balance?apiKey=`,
  distingue "sem plano ativo" e "limite diário atingido";
- `solve-test --provider nocaptchaai --balance` agora funciona:
  `saldo: US$ 0.00000` (plano diário ativo, sem saldo em conta);
- mensagem de erro de `createTask` orienta o caso "plano ativo mas tipo
  recusado";
- 2 testes novos (`nocaptchaai_balance_requires_key`,
  `explains_no_paid_plan_even_with_active_plan`).

`cargo test`: 92; `clippy` limpo.

Conclusão: a chave é válida e o OCR funciona, mas **NoCaptchaAI não emite token
de hCaptcha** — nem por `createTask`, nem por `/solve`. Para o MuAwaY continuam
válidos NoneCap, AntiCaptcha e CaptchaSonic.

## AZCaptcha e NextCaptcha (testados com chaves reais)

### AZCaptcha — suporte a hCaptcha em manutenção

Chave de teste, saldo US$ 0,10. Implementado `SolverKind::Azcaptcha`
(API v1 compatível com 2Captcha) + `azcaptcha_balance`.

```
GET /res.php?action=getbalance        -> {"status":1,"request":0.1}
in.php method=userrecaptcha           -> {"status":1,"request":"109561973"}   OK
createTask RecaptchaV2TaskProxyless   -> {"errorId":0,"taskId":109561970}     OK

in.php method=hcaptcha                -> {"status":0,"request":"ERROR_MAINTENANCE"}
createTask HCaptchaTaskProxyless      -> {"errorId":1,"errorCode":"ERROR_MAINTENANCE"}
createTask HCaptchaTask (+proxy)      -> ERROR_MAINTENANCE
createTask AntiHCaptchaTaskProxyless  -> ERROR_MAINTENANCE
```

Reexecutei após alguns segundos para descartar erro transitório: mesmo
resultado. O cliente está pronto e reporta:
`AZCaptcha in.php: ERROR_MAINTENANCE A AZCaptcha está com o serviço de hCaptcha em manutenção`.

### NextCaptcha — investigação completa (12 rodadas, 40+ payloads)

Chave de teste, saldo US$ 0,10.

**O serviço funciona** para os tipos documentados:

```
getBalance                 -> {"balance":0.1,"errorId":0}
ReCaptchaV2TaskProxyLess   -> {"errorId":0,"taskId":660946748,"status":"pending"}
TurnstileTaskProxyLess     -> criado
TipoInventadoTask          -> ERROR_TASK_NOT_SUPPORTED
```

**hCaptcha existe no produto, mas não tem documentação publicada.** O bundle de
i18n do site contém as chaves:

```
hcaptchaTitle            = "hCaptcha - Addresses the hCaptcha API"
hcaptchaProxyLessTitle   = "hCaptcha - Resolving the hCaptcha API without going through a proxy server"
hcaptchaEnterpriseTitle  = "hCaptchaEnterprise - Addresses the hCaptchaEnterprise API"
hcaptchaDesc / hcaptchaKeywords / hcaptchaEnterpriseKeywords
```

Mas **nenhuma página de doc** existe: `/apidocs/hcaptcha`,
`/apidocs/hcaptcha-proxyless`, `/apidocs/hcaptcha-enterprise` e variantes
retornam **404**. A navegação lista 17 páginas, todas de reCAPTCHA e Turnstile.

**Na API, o tipo é aceito mas sempre falha por parâmetros:**

```
hCaptchaTaskProxyLess  -> 400 ERROR_BAD_PARAMETERS ... (5)
hCaptchaTask           -> 400 ERROR_BAD_PARAMETERS ... (5)
hCaptchaEnterpriseTask -> 400 ERROR_BAD_PARAMETERS ... (5)
```

O detalhe decisivo: **o `(5)` é o código genérico de "campo obrigatório
ausente"**. Prova com o reCAPTCHA, que funciona:

```
ReCaptchaV2TaskProxyLess (completo)  -> 200 taskId
ReCaptchaV2TaskProxyLess (sem key)   -> 400 ERROR_BAD_PARAMETERS ... (5)   <- mesmo erro
ReCaptchaV2TaskProxyLess (sem url)   -> 400 ERROR_BAD_PARAMETERS ... (5)   <- mesmo erro
```

Ou seja, o erro do hCaptcha não é "tipo não suportado" — é "falta um campo que
a documentação não descreve". Foi o que motivou testar 40+ payloads:

- nomes de campo: `force`/`class`/`invisible`/`isInvisible`/`rqdata`/`quota`/
  `domain`/`host`/`url`/`sitekey`/`cookies`/`userAgent`/`pageAction`/`sid`/
  `enterprise`/`proxyType=direct` → todos `(5)` (exceto `cookies`, que muda
  para `(4)`, indicando outro ramo de validação);
- grafias do tipo: `HCaptchaTaskProxyless`, `hCaptchaTaskProxyLess`,
  `hCaptchaTaskProxyless`, `hcaptchataskproxyless`, `HcaptchaTaskProxyLess` →
  todas `(5)`;
- sitekeys oficiais de demonstração do hCaptcha
  (`10000000-ffff-ffff-ffff-000000000001` com `dashboard.hcaptcha.com/signup` e
  `accounts.hcaptcha.com/demo`) → `(5)`;
- `hCaptchaEnterpriseTask` com proxy completo + `userAgent` → `(5)`;
- host `api-v2.nextcaptcha.com` (usado pelo `getToken`): `createTask` e
  `getBalance` retornam **405 METHOD_NOT_SUPPORTED** — só `getToken` existe lá;
- `getToken` com os mesmos payloads → `1|... (5)`.

Também tentei obter o OpenAPI (`/openapi.json`, `/swagger.json`, `/api-docs`) e
analisei os 17 bundles JS: nenhum descreve os campos do hCaptcha.

**Conclusão:** o NextCaptcha anuncia hCaptcha nos textos de marketing, mas o
tipo não está utilizável — falta um parâmetro não documentado. **Não implementei
cliente**: com o contrato indisponível, qualquer implementação seria adivinhação
e falharia sempre com o mesmo `(5)`.

Se você conseguir a página de doc do hCaptcha (ela pode aparecer no dashboard
após login) ou o suporte informar os campos, a implementação é rápida: o fluxo
`createTask`/`getTaskResult` é idêntico ao já usado para AntiCaptcha/CapSolver.

### Bug real corrigido (saldo numérico)### Bug real corrigido (saldo numérico)

Ao testar AZCaptcha, `solve-test --balance` falhou com `saldo inesperado:` —
`twocaptcha_reply` só aceitava `request` como string, mas o saldo vem como
**número** (`{"status":1,"request":0.1}`). Corrigido para aceitar string, número
e booleano; isso também conserta o saldo do 2Captcha (mesmo formato).

- teste novo: `reads_numeric_request_as_balance`.

`cargo test`: 97; `clippy` limpo.

## Onyx (portado e validado end-to-end)

Base: `https://onyxsolver.io`. `PopularCaptchaTask` é o hCaptcha e **exige
proxy**; `TurnstileTaskProxyless` e `CloudflareChallengeTask` são os outros
tipos.

Chave de teste:

```
POST /api/getBalance
-> {"errorId":0,"balance":0,"solves":10,
    "allowed_types":["PopularCaptchaTask","TurnstileTaskProxyless","CloudflareChallengeTask"],
    "allowed_kinds":["popular","turnstile","challenge"],
    "maintenance":{"popular":false,"turnstile":false,"challenge":false}}

POST /api/createTask TurnstileTaskProxyless (sem proxy)
-> {"errorId":0,"taskId":"4FD0C323-..."}                      OK

POST /api/createTask PopularCaptchaTask (sem proxy)
-> ERROR_INVALID_TASK "PopularCaptchaTask requires a proxy"     validação ok

POST /api/createTask PopularCaptchaTask + proxy real
-> {"errorId":0,"taskId":"058E6C69-9D71-4D2F-842A-BAEEE367D979"}

POST /api/getTaskResult (poll 4 s, ~12 tentativas)
-> status=processing ...
-> status=ready
   solution.gRecaptchaResponse = "P1_eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.haJw..."
   solution.userAgent = "Mozilla/5.0 ..."
```

Pelo binário Rust:

```
solve-test --provider onyx --balance
-> saldo: US$ 0.00000

solve-test --provider onyx --proxy '<user>:<pass>@<host>:<port>'
-> provider "onyx", token_length 2483, prefixo P1_eyJhbGciO,
   expiration 120, elapsed 100141 ms, exit 0
```

**Terceiro provedor validado end-to-end** (com NoneCap e AntiCaptcha).

### WAF e formato do proxy (descoberta)

A API está atrás do DiamWall. O padrão literal `user:pass@127.0.0.1:8080` é
bloqueado:

```
POST /api/createTask  proxy="http://user:pass@127.0.0.1:8080"
-> 403 <html> Blocked | DiamWall
```

O mesmo proxy sem esquema e sem endereço local é aceito:

```
proxy="127.0.0.1:8080"                  -> 200 taskId
proxy="user:pass@127.0.0.1:8080"        -> 200 taskId
proxy="socks5://user:pass@127.0.0.1:8080" -> 200 taskId
proxy="user:pass@proxy.example.com:8080"  -> 200 taskId
```

Ou seja, o gatilho é a combinação `http://` + credenciais + `127.0.0.1` na
mesma string. O cliente envia o proxy **sem esquema** (`user:pass@host:port`),
que é o formato que a doc pede e que passa pelo WAF.

Também há proxy real disponível por variáveis/Configurações: o worker usa
`settings.solver_proxy` e o CLI aceita `--proxy` ou `ONYX_PROXY`.

Implementado em `solver.rs` (`SolverKind::Onyx`):

- `solve_onyx`: `createTask` (`PopularCaptchaTask`) + poll `getTaskResult`
  (4 s), token de `solution.gRecaptchaResponse`;
- `onyx_balance`: `getBalance`, valida acesso a `popular` e manutenção;
- `onyx_error_hint`: traduz "requires a proxy", "failed to connect via the
  supplied proxy", `ERROR_INVALID_TASK`, chave e saldo;
- sem proxy, recusa **antes** de chamar a API;
- campo **Proxy** em Configurações e `--proxy`/`ONYX_PROXY` no CLI;
- site `Onyx` adicionado ao `ui/sites.json`.

Testes: 4 novos (mapeamento do provedor, hints de proxy, exigência de
proxy/chave, default sem proxy). `cargo test`: 101; `clippy` limpo;
`npm run test:ui` passa; `validate_sites.py`: 6 sites, 0 problemas.

Não validado: renovação do proxy (os proxies fornecidos são de teste e podem
expirar), `CloudflareChallengeTask` (exige proxy e devolve cookies, fluxo
diferente) e uso contínuo por horas.

## Ajustes de UX (out/2026)

Quatro mudanças pedidas, todas implementadas e verificadas:

### 1. Proxy foi para o formulário da API-key

Antes era um campo global em Configurações. Agora é propriedade da credencial
(`ApiKey.proxy`), perguntado no cadastro do Onyx:

```
keyForm → needsProxy = siteId === 'onyx'
        → campo "Proxy (obrigatório para Onyx)" com validação obrigatória
        → salvo em { site_id, label, secret, proxy }
```

O `SolverCredential` ganhou `proxy`, e `solve_with_priority` passa a proxy da
credencial para o Onyx (antes usava a global). Cada chave pode ter seu proxy.

### 2. "Arquivo do token" removido

- campo retirado das Configurações;
- `TokenSource::File` eliminado; só existe `TokenSource::Solvers`;
- `read_token` e `Settings.token_file` removidos;
- CLI `auto-inject --file` removido (usa apenas variáveis de ambiente).

### 3. Tempo médio manual (libera a pré-resolução)

Novo bloco **Configurações → Tempo médio (opcional)**:

- checkbox `use_manual_timing`;
- `manual_interval_s` (captcha a cada N s) e `manual_solve_s` (resolução N s).

No `Timing`, `solve_mean`/`interval_mean` caem no valor manual enquanto não houver
amostras reais, e `ready()` aceita manual **ou** 2+ amostras. O cálculo real
sempre prevalece: ao acumular amostras, os manuais deixam de ser usados. Validação
recusa ativar o checkbox com os campos zerados.

### 4. Bug: "← Emuladores" não voltava

`instance()` renderizava `back()` mas nunca ligava `$('#back').onclick`. Corrigido
com `$('#back').onclick=home`.

### Verificação

```
cargo test        -> 55 passed (4 novos de tempo manual, incluindo
                     "real_samples_override_manual_values" e
                     "zero_or_missing_manual_keeps_timing_disabled")
cargo clippy      -> sem avisos
npm test          -> 16
npm run test:ui   -> passa, com asserções novas:
                     · #solver-proxy não existe em Configurações;
                     · ← Emuladores volta para a lista;
                     · proxy pedido no formulário da chave
validate_sites.py -> 4 sites, 0 problemas

solve-test --provider anticaptcha
-> token 2484 chars, P1_eyJhbGciO, 52 s, exit 0
```

## Não validado

- Injeção em múltiplos emuladores reais simultâneos (o fluxo `inject_captcha_all`
  e o monitor de páginas foram testados com backend simulado, não com duas VMs
  ligadas).
- Injeção completa via Frida em um ciclo único: a habilitação foi validada
  (`debug_enabled: true`, socket criado) e a injeção real foi validada com o
  socket já ativo; não houve um captcha aberto no mesmo ciclo da habilitação
  para injetar com o token real.
- Outros aplicativos, outras versões de Android/Chromium.
- Instalador NSIS e execução da janela Tauri em modo release (o binário debug
  foi compilado; a interface foi testada no Chromium via Playwright).
- Autenticação real de produção (apenas o modo `demo-auth`).
