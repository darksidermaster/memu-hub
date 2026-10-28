# MEmu Hub

Aplicativo Windows em **Rust + Tauri 2** para controlar emuladores MEmu e
automatizar o captcha hCaptcha de um WebView Android **já aberto** — sem
reinstalar o APK, sem reiniciar o aplicativo e sem Python, Selenium ou Playwright.

A janela tem 350 × 450 unidades lógicas, sem bordas, com interface HTML/CSS/JS
local (sem CDN, sem fontes remotas). O backend fala **ADB**, **Chrome DevTools
Protocol** e as APIs dos provedores de captcha, tudo nativamente em Rust.

> **Escopo.** Projeto para ambiente local de testes autorizado. Ele não modifica
> o APK, não contorna autenticação, pagamentos ou proteções anti-depuração e não
> lê cookies, tokens de sessão ou credenciais.

---

## Índice

- [O problema que ele resolve](#o-problema-que-ele-resolve)
- [Como funciona](#como-funciona)
- [Recursos](#recursos)
- [Provedores de captcha](#provedores-de-captcha)
- [Requisitos](#requisitos)
- [Instalação](#instalação)
- [Uso](#uso)
- [Interface](#interface)
- [Pré-resolução (token antecipado)](#pré-resolução-token-antecipado)
- [Configuração](#configuração)
- [Linha de comando](#linha-de-comando)
- [Segurança e dados](#segurança-e-dados)
- [Estrutura do projeto](#estrutura-do-projeto)
- [Testes](#testes)
- [O que foi validado](#o-que-foi-validado)
- [Limitações](#limitações)
- [Solução de problemas](#solução-de-problemas)
- [Referências](#referências)

---

## O problema que ele resolve

O MuAwaY (`com.away.game.br`) roda em MEmu e, em certos momentos, abre uma
WebView em `https://captcha.muaway.net/` que exige um hCaptcha. Controlar essa
janela é difícil por três motivos:

1. **O WebView não expõe CDP por padrão.** Não existe `webview_devtools_remote_<pid>`
   em `/proc/net/unix` até a depuração ser habilitada no processo.
2. **O canal de entrega do token é específico do Android.** A página chama
   `AndroidBridge.handleCallback(token)`; o `window.chrome.webview.postMessage`
   do WebView2 (Windows) não existe aqui.
3. **Fechar a janela não destrói a página.** O target continua em `/json/list`,
   `document.visibilityState` segue `"visible"` e `timeOrigin` não muda — então
   é preciso outro sinal para saber que o captcha já foi respondido.

O MEmu Hub resolve os três: habilita a depuração quando necessário, injeta pelo
bridge correto e usa o **campo de resposta preenchido** como sinal de "já
resolvido", evitando gastar crédito à toa.

## Como funciona

Para cada emulador, sem PID fixo:

```
memuc listvms --running
   └─ índice da VM → serial ADB (127.0.0.1:21503 + id × 10)

adb -s <serial> shell pidof com.away.game.br          → PID atual
su -c 'cat /proc/net/unix' | grep devtools            → webview_devtools_remote_<pid>
   └─ se ausente + --enable-debug: frida-server + setWebContentsDebuggingEnabled(true)
      na UI thread (o PID é relido: o app pode reiniciar)

adb forward tcp:<porta livre> localabstract:<socket>
GET /json/list                                        → targets
   └─ aceita somente type=page cuja URL contenha captcha.muaway.net

GET estado da página (CDP)                            → campo de resposta vazio?
   ├─ não → nada a fazer (janela fechada)
   └─ sim → token do provedor → Runtime.evaluate:
              AndroidBridge.handleCallback(token)
            + preenche h-captcha-response / g-recaptcha-response
            + relê os campos para confirmar
```

Cada injeção é confirmada por três evidências (`bridge_called`, campos
preenchidos, `readback_ok`). O app **nunca** declara sucesso sem confirmação.

## Recursos

**Controle de emuladores**
- Descobre todas as instâncias MEmu (sonda as portas 21503–21563 e cruza com
  `memuc listvms`), inclusive múltiplas ao mesmo tempo.
- Detecta a instalação do MEmu pelo Registro (`InstallLocation`), então funciona
  mesmo com o emulador em outro disco.
- Abre as configurações do Android na VM selecionada.

**Inspeção do WebView**
- Prova que o processo usa WebView lendo `/proc/<pid>/maps` e procurando
  bibliotecas Chromium.
- Distingue sockets CDP reais de `WebViewZygoteInit` (que não são endpoints).
- Valida `/json/version` e `/json/list` antes de usar o forward.

**Automação**
- Habilitação de depuração via Frida quando não há socket, com o `frida-server`
  iniciado automaticamente e o PID relido após a instrumentação.
- Injeção via `AndroidBridge.handleCallback`, com preenchimento opcional dos
  campos do widget e verificação por readback.
- Monitor por thread (1 s) que injeta assim que a página fica pronta.
- Contadores `pass`/`fail`, etapa atual e log append-only das tentativas.

**Operação**
- Vault DPAPI (CurrentUser) para a licença e as API-keys no Registro.
- CSP restritivo, comandos autenticados, HTML escapado.
- Modo CLI para QA e automação.

## Provedores de captcha

Todos são **nativos em Rust** — HTTP direto, sem Node, sem Chrome headless e sem
subprocesso. O token é o `P1_...` do hCaptcha.

| Provedor | Endpoint | Como o token chega | Estado |
| --- | --- | --- | --- |
| **NoneCap** | `api.nonecap.com/v1` | `POST /solves` (Bearer), poll até `solved`, token em `solve.token` | validado end-to-end |
| **2Captcha** | `2captcha.com` | API v1 `in.php`/`res.php` com `method=hcaptcha` | validado end-to-end |
| **Anti-Captcha** | `api.anti-captcha.com` | `createTask` (`HCaptchaTaskProxyless`) → `getTaskResult` | validado end-to-end |
| **NopeCHA** | `api.nopecha.com` | `POST /v1/token/hcaptcha` + poll `GET /v1/token/hcaptcha?id=` | validado end-to-end (**20 créditos/token**) |
| **Onyx** | `onyxsolver.io` | `POST /api/createTask` (`PopularCaptchaTask`, **exige proxy**) → `/api/getTaskResult` | validado end-to-end |

**Custo do NopeCHA:** `20 créditos` por token de hCaptcha, cobrados no
submit (antes de o token existir). A página de preços fala em "1 crédito por
captcha", mas essa regra é da *Recognition API* (imagens); a *Token API* — a que
este app usa — não publica o preço e cobra por token. Na prática, um plano de
2000 créditos rende ~100 tokens de hCaptcha por ciclo de 23 h.

O provedor é escolhido pela **ordem de prioridade** configurada. Se um falha, o
próximo é tentado e o erro final lista o motivo de cada um.

Detalhes que exigiram trabalho e estão implementados:

- **Onyx** exige proxy por credencial (`user:pass@host:port`, sem esquema) e sua
  API fica atrás de um WAF que bloqueia o padrão `http://user:pass@127.0.0.1`;
  o cliente envia no formato aceito.
- **2Captcha** usa `in.php` (v1), que é o caminho do SDK oficial para hCaptcha.
  A conta precisa ter o IP liberado no painel, senão o `in.php` responde
  `ERROR_IP_NOT_ALLOWED`.
- **NoneCap** devolve o token direto, sem etapa de imagens.
- **AntiCaptcha** entrega `solution.gRecaptchaResponse` em ~30–100 s.

Provedores avaliados e **removidos** (não entregam token de hCaptcha ou exigiam
runtime externo) ficam documentados em [`VALIDATION.md`](VALIDATION.md):
CapSolver, NoCaptchaAI, AZCaptcha, NextCaptcha, Surfsky, CaptchaKings e
CaptchaSonic.

## Requisitos

- **Windows 10/11** (o vault usa DPAPI e o MEmu é Windows-only).
- **Rust** estável com toolchain MSVC e **VS Build Tools** com "Desenvolvimento
  para desktop com C++" + Windows SDK.
- **Node.js** LTS (só para o CLI do Tauri).
- **Microsoft Edge WebView2 Runtime**.
- **MEmu** com Android 9 (testado), depuração USB e **root** habilitados.
- **ADB** (platform-tools): detectado em `ANDROID_HOME`, `ANDROID_SDK_ROOT`,
  `%LOCALAPPDATA%\Android\Sdk`, instalação do MEmu ou `PATH`.
- **Frida**: o aplicativo prepara sozinho. Na primeira abertura, antes de pedir
  a chave, ele detecta `adb`/`frida`, instala `frida-tools` com o pip quando
  necessário, baixa o `frida-server` na mesma versão do CLI e, se houver um
  emulador aberto, envia e inicia como root. A série 17.x removeu o runtime
  `Java` no servidor deste ambiente, por isso a versão fixa é 16.7.19.

### Preparação do ambiente

A tela **Preparação** abre antes do login e mostra o estado de `ADB`, `Frida` e
`frida-server`, com as etapas executadas. Ela é idempotente: quando tudo já está
pronto, responde em milissegundos. Falhar numa etapa não impede o login — só
deixa a depuração via Frida pendente, e **Verificar novamente** refaz a
preparação.

O `frida` CLI é procurado no `PATH`, em `Scripts` ao lado do Python e nas
instalações padrão em `%LOCALAPPDATA%\Programs\Python`. O `frida-server` fica em
`%LOCALAPPDATA%\MEmuHub\tools\frida-server\`, gravado primeiro em arquivo
temporário e renomeado só quando o download termina.

O comando equivalente fora da interface:

```powershell
memu-hub.exe setup               # instala o que faltar
memu-hub.exe setup --no-install  # só verifica o ambiente
```

## Instalação

```powershell
git clone <url-do-repositorio>
cd memu-hub
npm install
npm run build          # executável em src-tauri/target/release/
# ou
npm run build:installer  # instalador NSIS
python -m pip install "frida==16.7.19" frida-tools==13.7.1   # opcional
```

O executável fica em `src-tauri/target/release/memu-hub.exe`. O PC de destino
precisa do WebView2 Runtime.

### Autenticação pelo LicenseHub

As chaves são as **mesmas do Autoloot**: o servidor `license-server` faz a
ativação por chave + HWID e emite tokens. O app usa o mesmo cálculo de HWID do
Autoloot (`SHA-256("MachineGuid|serial-do-volume|MuAwaY-Autoloot")`), então uma
chave já ativada pela DLL abre o MEmu Hub nesta máquina sem nova ativação, e
vice-versa. Um HWID diferente faz o servidor responder `HWID_MISMATCH`.

O servidor controla **para quais apps** cada chave vale: `memuhub`, `autoloot`
ou ambos. Uma chave restrita ao outro app responde
`APP_NOT_ALLOWED` e a tela mostra a mensagem do servidor.

Fluxo:

1. `POST /api/activate` com `{ key, hwid, app }` na primeira vez — vincula o
   dispositivo e define o vencimento.
2. `POST /api/refresh` com `{ refresh_token, hwid, app }` nas aberturas
   seguintes — o refresh token é rotativo e é regravado a cada uso.
3. Se o refresh token for recusado, a chave salva é usada como fallback; se
   também for recusada, a mensagem real do servidor aparece na tela.

O tráfego vai em **envelope autenticado** (ECDH P-256 + AES-256-GCM, com
timestamp e nonce), então o conteúdo não pode ser lido nem forjado por quem
termina o TLS no meio do caminho. Ver `src-tauri/src/envelope.rs`.

O servidor padrão é `https://rachabuzina.online:8443`. Para outro ambiente:

```powershell
$env:MEMU_HUB_LICENSE_SERVER = 'https://licencas.seudominio.com'
npm run build
```

O override por variável de ambiente existe para desenvolvimento; o build de
distribuição usa `--features production-server`, que ignora a variável.

A chave e os tokens ficam no cofre DPAPI do usuário (Registro
`HKCU\Software\MEmuHub\Vault`), nunca em texto claro. "Sair e esquecer chave de
acesso" apaga a chave, o refresh token e o access token.
Compilação de teste, com a chave local habilitada:

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --features demo-auth
```

> **Nota deste ambiente.** O `rustc` estoura a pilha ao linkar o binário com
> debuginfo completo. Por isso `Cargo.toml` define `debug = 0` para o crate do
> app. Se o build paralelo falhar, use `$env:CARGO_BUILD_JOBS=1`.

### Frida (opcional)

Para habilitar a depuração de um WebView que ainda não expõe socket:

```powershell
# inicia o frida-server no emulador (se já estiver instalado)
python -c "import frida; print(frida.__version__)"
adb -s 127.0.0.1:21513 shell "su -c 'nohup /data/local/tmp/frida-server >/dev/null 2>&1 &'"
```

O app faz isso automaticamente ao ligar o switch de injeção, desde que o binário
esteja em `/data/local/tmp/frida-server`.

## Uso

1. Abra o MEmu com o MEmu Hub fechado ou aberto — tanto faz.
2. Execute `memu-hub.exe` (ou `npm run dev` em desenvolvimento).
3. Informe sua chave de autenticação.
   O build padrão valida a chave em `MEMU_HUB_AUTH_URL` (definido na
   compilação, HTTPS ou localhost). Para testes locais existe a feature
   opcional `demo-auth`, que aceita uma chave fixa — ela **não** vem ligada por
   padrão e não deve ser usada em produção.
4. A lista mostra os emuladores ativos com PID.
5. Cadastre uma API-key em **Configurações → + Cadastrar site**.
6. Ligue o switch **Injetar token** no card do emulador.

O switch inicia a thread que verifica o WebView a cada segundo. Quando o jogo
abrir o captcha, o token é obtido e injetado automaticamente.

## Interface

```
┌─────────────────────────────────────────┐
│  SEUS EMULADORES                    ⟳   │
│  Instâncias MEmu em execução            │
│                                         │
│  ┌───────────────────────────────────┐  │
│  │ [MEmu] MEmu1                       │  │
│  │    Ativo · PID 3217 · #0          │  │
│  │    Token injetado · ✓3 · ✕1 · 2s  │  │
│  │              INJETAR  [ ●]        │  │
│  │              PRÉ-RES. [ ○]  ⚙     │  │
│  └───────────────────────────────────┘  │
└─────────────────────────────────────────┘
```

- **Injetar token:** liga a thread de injeção automática.
- **Pré-resolver:** resolve antes da janela abrir (ver seção seguinte). Fica
  desabilitado até existir tempo médio — calculado ou definido manualmente.
- **Etapa e contadores:** `Aguardando captcha → Página detectada → Resolvendo
  pela API → Injetando token → Token injetado`, com `✓ pass`, `✕ fail` e a
  duração da última resolução. O tooltip lista as últimas tentativas.
- **Engrenagem:** configuração da instância e acesso às configurações Android.

## Pré-resolução (token antecipado)

O segundo switch resolve o captcha **antes** de a janela abrir, deixando o token
pronto para injeção imediata. O disparo é calculado com o histórico real:

| Medida | Como é obtida |
| --- | --- |
| `solve_mean` | duração média das resoluções |
| `interval_mean` | intervalo médio entre captchas detectados |
| margem | desvio-padrão das resoluções, **limitado a 50% da média** |
| disparo | `interval_mean − (solve_mean + margem)` após a última captcha |

O teto de 50% existe porque resoluções de hCaptcha variam muito na prática
(observadas de 9 s a 68 s); sem ele, um outlier anteciparia a resolução em mais
de um minuto e o token venceria antes do uso.

**Validade:** 120 s (TTL do hCaptcha) com margem de segurança de 10 s. Um token
mais antigo que isso é descartado em vez de injetado vencido. Se a janela abrir
antes do previsto, o token é usado na hora; se não abrir, ele expira e o fluxo
normal assume.

**Tempo médio manual:** em **Configurações → Tempo médio** há um checkbox com
dois campos (captcha a cada N s / resolução leva N s). Ligado, ele libera a
pré-resolução imediatamente, sem esperar o histórico. O cálculo real
**substitui** os valores manuais assim que houver amostras, então a estimativa
melhora sozinha.

## Configuração

`config.json` não é usado: as preferências ficam no Registro, cifradas com DPAPI.

| Campo | Para que serve |
| --- | --- |
| `memuc_path` | caminho do `memuc.exe` (vazio = detecção automática) |
| `adb_path` | caminho do `adb.exe` |
| `package_name` | pacote alvo (padrão `com.away.game.br`) |
| `solver_timeout` | timeout da resolução (30–1800 s) |
| `manual_interval_s` / `manual_solve_s` | tempo médio manual |
| `use_manual_timing` | libera a pré-resolução com os valores manuais |
| `sites` / `keys` / `priority` | catálogo, credenciais e ordem de tentativa |
| `instances` | preferências por índice de VM (apelido, site, switches) |

As credenciais guardam também o **proxy** quando o provedor exige (Onyx), porque
cada conta pode usar um proxy diferente.

### Catálogo de sites

As opções de cadastro vêm de [`ui/sites.json`](ui/sites.json):

```json
{
  "NoneCap":      { "icon": ".\\assets\\logos\\nonecap.webp" },
  "2Captcha":     { "icon": ".\\assets\\logos\\2captcha.svg" },
  "Anti-Captcha": { "icon": ".\\assets\\logos\\Anti-Captcha.ico" },
  "Onyx":         { "icon": "ON" }
}
```

O `icon` é um caminho dentro de `ui/assets/` (barras invertidas e `./` são
normalizados; caminho absoluto, `..` e URL são rejeitados) ou um texto de até 2
caracteres. O id interno vem do nome (`NoneCap` → `nonecap`).

Validação do arquivo, sem abrir o app:

```powershell
python scripts/validate_sites.py
```

## Linha de comando

O mesmo binário expõe os caminhos usados pela interface — útil para QA:

```powershell
memu-hub.exe sessions
# lista emuladores, PID, socket e páginas abertas (JSON)

memu-hub.exe solve-test --provider anticaptcha --key <chave> --balance
# saldo da conta, sem criar tarefa

memu-hub.exe solve-test --provider onyx --key <chave> --proxy <proxy>
# resolve um hCaptcha e imprime provedor, tamanho do token e tempo

memu-hub.exe auto-inject --vm 1 --enable-debug --seconds 120
# executa o mesmo worker da interface (chaves via NONECAP_KEY,
# TWOCAPTCHA_KEY, ANTICAPTCHA_KEY, ONYX_KEY)

memu-hub.exe validate-settings < settings.json
# roda o mesmo check_settings do app
```

Códigos de saída: `0` sucesso, `1` falha do provedor, `2` uso incorreto.

## Segurança e dados

- **DPAPI (CurrentUser)** protege licença e API-keys em repouso, no valor
  `Vault` de `HKCU\Software\MEmuHub`. Isso não impede acesso por programas que
  já rodem com a mesma conta do Windows.
- Nenhuma chave vai para localStorage, logs, argumentos de linha de comando ou
  arquivos de texto. O frontend recebe as configurações durante a sessão; a
  licença armazenada não é enviada a ele.
- Comandos Tauri exigem autenticação no backend.
- CSP restringe scripts, estilos, imagens e IPC; `connect-src 'self'` é
  necessário para carregar o catálogo.
- Nomes fornecidos pelo usuário são escapados antes de virar HTML.
- `adb`, `su`, `memuc` e `frida` rodam sem shell intermediário, com serial
  explícito e timeout; o forward criado é sempre removido.
- O log de tentativas não contém o token: só provedor, tempo, `pass`/`fail` e
  tamanho.

### Recuperação do vault

Se o Registro ficar ilegível, o app **não** sobrescreve o valor. Exporte a chave
antes de remover manualmente apenas `Vault` em `HKCU\Software\MEmuHub` —
removê-lo apaga todas as preferências e credenciais. Dados DPAPI não são
portáveis entre usuários ou máquinas.

## Estrutura do projeto

```
memu-hub/
├── ui/                       frontend (sem build step)
│   ├── index.html
│   ├── app.js                telas, switches, handlers
│   ├── catalog.js            parsing/validação de sites.json
│   ├── model.js              prioridade e escape
│   ├── style.css
│   ├── sites.json            catálogo de provedores
│   └── assets/logos/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs           comandos Tauri, Settings, sessão e auth
│   │   ├── adb.rs            wrapper ADB (devices, su, forward, /proc)
│   │   ├── memu.rs           memuc (listvms, startapp, descoberta)
│   │   ├── webview.rs        sessão, PID, socket, sitekey
│   │   ├── cdp.rs            HTTP /json/list + WebSocket CDP + estado da página
│   │   ├── http.rs           cliente JSON compartilhado
│   │   ├── solver.rs         NoneCap, 2Captcha, AntiCaptcha, Onyx
│   │   ├── auto_inject.rs    worker, estágios, Timing, pré-resolução, log
│   │   └── vault.rs          DPAPI + Registro
│   ├── Cargo.toml
│   └── tauri.conf.json
├── tests/                    node:test + Playwright
├── scripts/                  validadores em Python
├── README.md
└── VALIDATION.md             registro do que foi testado e como
```

## Testes

```powershell
npm test                 # lógica JS (prioridade, escape, catálogo)
npm run test:ui          # fluxo completo da UI no Chromium (backend simulado)
cargo test               # backend Rust
cargo clippy --all-targets
python scripts/validate_sites.py
python scripts/validate_settings_flow.py   # exige cargo build antes
```

- **55 testes Rust**: parser do MEmu, envelope CDP, estado da página
  (`should_solve`), `check_settings` (duas chaves no mesmo site, ícone, caminhos
  inseguros), worker (estágios, deduplicação, TTL) e os quatro provedores
  (mapeamento de id, recusa dos removidos, prioridade, erros, exigência de
  chave/proxy).
- **16 testes JS** e **teste de UI** com Playwright: login, listagem, switches
  na mesma `div` com rótulos, bloqueio da pré-resolução sem tempo médio,
  persistência, catálogo com ícones, prioridade, escape e layout em 350 px. O
  teste roda sob o **mesmo CSP** do Tauri, então uma violação falha a suíte.
- **`validate_settings_flow.py`** executa o `check_settings` real via binário,
  fechando a lacuna do backend simulado do teste de UI.

## O que foi validado

Registro completo em [`VALIDATION.md`](VALIDATION.md). Resumo do ambiente de
teste (MEmu, Android 9, x86_64, root, Chromium 116):

| Item | Resultado |
| --- | --- |
| Descoberta do emulador | encontrou a VM, serial e PID reais |
| Socket DevTools antes do Frida | ausente (só `WebViewZygoteInit`) |
| Habilitação via Frida | criou `@webview_devtools_remote_<pid>` sem reiniciar o app |
| Injeção via `AndroidBridge` | token real, `readback_ok`, página fecha após aceitar |
| NoneCap / AntiCaptcha / Onyx | token `P1_...` real, 14 s / 37 s / 39 s |
| 2Captcha | token `P1_...` real injetado no WebView, 27,6 s |
| Reinício do app | PID mudou, socket recriado, reconexão automática |
| Janela do captcha fechada | 0 tentativas, nenhum crédito gasto |

## Limitações

- **Windows-only** (DPAPI + MEmu).
- **Um app-alvo por vez** por emulador (`package_name`), embora várias VMs
  possam rodar em paralelo.
- **Onyx exige proxy.** Sem proxy o provedor recusa; o app avisa antes de tentar.
- A habilitação via Frida depende de `frida-server` compatível; sem ele, use um
  WebView que já exponha socket.
- **Desafios de vídeo** do hCaptcha não são cobertos por nenhum provedor nativo
  deste projeto.
- A pré-resolução só é exata com histórico; os valores manuais são um ponto de
  partida.

## Solução de problemas

| Sintoma | Causa provável | Ação |
| --- | --- | --- |
| `MEmu não encontrado` | `memuc.exe` fora dos locais comuns | informe o caminho nas Configurações |
| `WebView sem socket DevTools` | depuração desligada | ligue o switch (habilita Frida) ou reinicie o app com a flag |
| `Token injetado` mas captcha volta | token rejeitado/expirado | verifique a API-key e o saldo; o log mostra `pass`/`fail` |
| `A pré-resolução exige um tempo médio` | sem histórico e sem valores manuais | deixe rodar algumas captchas ou configure o tempo médio |
| `Onix: a conta não tem acesso` | plano sem `popular` | verifique o dashboard da Onyx |
| `ERROR_IP_NOT_ALLOWED` (2Captcha) | whitelist de IP | libere o IP no painel |
| Build falha com stack overflow | paralelismo do `rustc` | `$env:CARGO_BUILD_JOBS=1` |

## Referências

- Tauri 2 — <https://v2.tauri.app/reference/config/>
- Chrome DevTools Protocol — <https://chromedevtools.github.io/devtools-protocol/>
- MEmu / MEMUC — <https://www.memuplay.com/blog/memucommand-reference-manual.html>
- Frida — <https://frida.re/docs/>
- NoneCap — <https://docs.nocaptchaai.com/docs>
- 2Captcha — <https://2captcha.com/api-docs>
- Anti-Captcha — <https://anti-captcha.com/apidoc>
- Onyx — <https://onyxsolver.io>

## Licença

Uso restrito a testes locais autorizados. Não há licença de distribuição
implícita; entre em contato antes de reutilizar.
