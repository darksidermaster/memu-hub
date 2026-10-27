#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod adb;
mod auto_inject;
mod cdp;
mod http;
mod memu;
mod solver;
mod vault;
mod webview;
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, HashSet}, sync::Mutex};
use tauri::State;
type Result<T> = std::result::Result<T, String>;
type Workers = std::collections::HashMap<u32, auto_inject::AutoInjector>;
#[derive(Clone, Serialize, Deserialize)]
struct Site { id: String, name: String, icon: String }
#[derive(Clone, Serialize, Deserialize)]
struct ApiKey {
    id: String,
    site_id: String,
    label: String,
    secret: String,
    /// Proxy da credencial (Onyx exige). Vazio para os demais.
    #[serde(default)]
    proxy: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct InstanceConfig {
    alias: String,
    site_id: String,
    /// Lembra o estado do switch da lista; a thread em si é controlada pelo backend.
    #[serde(default)]
    auto_inject: bool,
    /// Pré-resolução: resolve antes da janela abrir, com base no tempo médio.
    #[serde(default)]
    pre_solve: bool,
}
#[derive(Clone, Serialize, Deserialize)]
struct Settings {
    memuc_path: String,
    #[serde(default)]
    adb_path: String,
    #[serde(default = "default_package")]
    package_name: String,
    demo_emulators: bool,
    /// Timeout de uma resolução, em segundos.
    #[serde(default = "default_solver_timeout")]
    solver_timeout: u64,
    /// Intervalo médio entre captchas informado manualmente (segundos).
    /// Libera a pré-resolução sem esperar o cálculo automático; o cálculo real
    /// assume assim que houver amostras.
    #[serde(default)]
    manual_interval_s: u32,
    /// Tempo médio de resolução informado manualmente (segundos).
    #[serde(default)]
    manual_solve_s: u32,
    /// Libera a pré-resolução usando os valores manuais, sem esperar o
    /// cálculo automático. O cálculo real substitui os manuais com o tempo.
    #[serde(default)]
    use_manual_timing: bool,
    sites: Vec<Site>, keys: Vec<ApiKey>,
    priority: Vec<String>, instances: BTreeMap<u32, InstanceConfig>,
}
fn default_package() -> String { webview::DEFAULT_PACKAGE.to_string() }
fn default_solver_timeout() -> u64 { 180 }
impl Default for Settings {
    fn default() -> Self {
        Self { memuc_path: String::new(), adb_path: String::new(),
            package_name: default_package(), demo_emulators: false,
            solver_timeout: default_solver_timeout(),
            manual_interval_s: 0, manual_solve_s: 0, use_manual_timing: false,
            sites: vec![Site { id:"site-a".into(), name:"Site A (exemplo)".into(), icon:"A".into() }, Site { id:"site-b".into(), name:"Site B (exemplo)".into(), icon:"B".into() }],
            keys: vec![], priority: vec![], instances: BTreeMap::new() }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Vault { key: String, settings: Settings }
#[derive(Default)]
struct Session { vault: Vault, authenticated: bool, revision: u64, load_error: Option<String> }
#[derive(Serialize)]
struct AuthStatus { authenticated: bool, demo: bool }
fn lock<'a>(s: &'a State<'_, Mutex<Session>>) -> Result<std::sync::MutexGuard<'a, Session>> { s.lock().map_err(|_| "Sessão indisponível.".into()) }
fn persist(v: &Vault) -> Result<()> { vault::write(&serde_json::to_vec(v).map_err(|_| "Configuração inválida.")?) }
fn authorized(s: &Session) -> Result<()> {
    if let Some(e) = &s.load_error { return Err(e.clone()); }
    if s.authenticated { Ok(()) } else { Err("Autentique-se para continuar.".into()) }
}
async fn validate(key: &str) -> Result<bool> {
    if key.is_empty() || key.len() > 512 { return Ok(false); }
    #[cfg(feature="demo-auth")]
    { Ok(key == "DEMO-2026") }
    #[cfg(not(feature="demo-auth"))]
    {
        let endpoint = option_env!("MEMU_HUB_AUTH_URL").ok_or("Endpoint de autenticação não definido na compilação.")?;
        let url = reqwest::Url::parse(endpoint).map_err(|_| "Endpoint inválido.")?;
        if url.scheme() != "https" && !(url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1") | Some("localhost"))) { return Err("Use HTTPS ou servidor local.".into()); }
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).redirect(reqwest::redirect::Policy::none()).build().map_err(|_| "Falha HTTP.")?;
        let response = client.post(url).json(&serde_json::json!({"key":key})).send().await.map_err(|_| "Não foi possível validar a chave. Tente novamente.")?;
        if matches!(response.status().as_u16(), 401 | 403) { return Ok(false); }
        if !response.status().is_success() { return Err("Servidor de autenticação indisponível.".into()); }
        #[derive(Deserialize)] struct Reply { valid: bool }
        Ok(response.json::<Reply>().await.map_err(|_| "Resposta de autenticação inválida.")?.valid)
    }
}
#[tauri::command]
async fn authenticate(key: Option<String>, state: State<'_, Mutex<Session>>) -> Result<AuthStatus> {
    let (candidate, revision) = {
        let s = lock(&state)?;
        if let Some(e) = &s.load_error { return Err(e.clone()); }
        (key.unwrap_or_else(|| s.vault.key.clone()).trim().to_string(), s.revision)
    };
    let valid = validate(&candidate).await?;
    let mut s = lock(&state)?;
    if revision != s.revision { return Err("A sessão mudou. Tente novamente.".into()); }
    s.authenticated = false;
    if valid {
        let mut next = s.vault.clone(); next.key = candidate;
        persist(&next)?; s.vault = next; s.authenticated = true;
    }
    s.revision += 1;
    Ok(AuthStatus { authenticated: valid, demo: cfg!(feature="demo-auth") })
}
#[tauri::command]
fn is_demo() -> bool { cfg!(feature="demo-auth") }
#[tauri::command]
fn logout(state: State<'_, Mutex<Session>>) -> Result<()> {
    let mut s = lock(&state)?;
    if let Some(e) = &s.load_error { return Err(e.clone()); }
    s.authenticated = false; s.revision += 1;
    let mut next = s.vault.clone(); next.key.clear(); persist(&next)?; s.vault = next; Ok(())
}
#[tauri::command]
fn get_settings(state: State<'_, Mutex<Session>>) -> Result<Settings> {
    let s = lock(&state)?; authorized(&s)?; Ok(s.vault.settings.clone())
}
/// Ícone aceito: texto curto (até 2 caracteres) ou caminho relativo dentro de
/// `ui/assets/`. Espelha a validação de `ui/catalog.js`.
fn check_icon(icon: &str) -> bool {
    let icon = icon.trim();
    if icon.is_empty() || icon.len() > 512 {
        return false;
    }
    if icon.chars().count() <= 2 && !icon.contains('/') && !icon.contains('\\') {
        return true;
    }
    let normalized = icon.replace('\\', "/");
    let mut path = normalized.as_str();
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    if !path.starts_with("assets/") || path.contains("..") || path.contains("//") {
        return false;
    }
    path.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '/' || c == '_' || c == '-')
}

fn check_settings(v: &Settings) -> Result<()> {
    if v.sites.len()>50 || v.keys.len()>200 || v.instances.len()>500 || v.memuc_path.len()>1024 || v.adb_path.len()>1024 || v.package_name.len()>200 { return Err("Limite de configuração excedido.".into()); }
    let mut sites = HashSet::new();
    for site in &v.sites {
        if site.id.is_empty() || site.id.len()>100 {
            return Err("Site com identificador inválido.".into());
        }
        if !sites.insert(&site.id) {
            return Err(format!("O site '{}' está duplicado no catálogo.", site.name));
        }
        if site.name.trim().is_empty() || site.name.len()>100 {
            return Err("Site sem nome válido.".into());
        }
        if !check_icon(&site.icon) {
            return Err(format!(
                "O ícone do site '{}' é inválido. Use um caminho em assets/ (ex.: assets/logos/site.svg) ou até 2 caracteres.",
                site.name
            ));
        }
    }
    let mut ids = HashSet::new();
    for k in &v.keys {
        if !sites.contains(&k.site_id) || k.id.is_empty() || k.id.len()>100 || !ids.insert(&k.id) || k.secret.trim().is_empty() || k.secret.len()>4096 || k.label.len()>100 { return Err("API-key inválida.".into()); }
    }
    let active: HashSet<_> = v.keys.iter().map(|k| &k.site_id).collect();
    let priority: HashSet<_> = v.priority.iter().collect();
    if priority.len()!=v.priority.len() || priority!=active { return Err("Prioridade inconsistente.".into()); }
    for i in v.instances.values() {
        if i.alias.len()>100 || (!i.site_id.is_empty() && !active.contains(&i.site_id)) {
            return Err("Configuração do emulador inválida.".into());
        }
    }
    if v.memuc_path.len() > 1024 {
        return Err("Caminho de arquivo muito longo.".into());
    }
    // 0 significa "não definido"; acima disso aceita 5..=3600.
    if v.manual_interval_s != 0 && !(5..=3600).contains(&v.manual_interval_s) {
        return Err("O intervalo médio deve ficar entre 5 e 3600 segundos.".into());
    }
    if v.manual_solve_s != 0 && !(5..=3600).contains(&v.manual_solve_s) {
        return Err("O tempo de resolução deve ficar entre 5 e 3600 segundos.".into());
    }
    if v.use_manual_timing && (v.manual_interval_s == 0 || v.manual_solve_s == 0) {
        return Err(
            "Para usar o tempo médio definido, informe o intervalo e o tempo de resolução."
                .into(),
        );
    }
    if !(30..=1800).contains(&v.solver_timeout) {
        return Err("O timeout do solver deve ficar entre 30 e 1800 segundos.".into());
    }
    if v.package_name.trim().is_empty() || !v.package_name.chars().all(|c| c.is_ascii_alphanumeric() || c=='.' || c=='_') { return Err("Nome de pacote inválido.".into()); }
    Ok(())
}
#[tauri::command]
fn save_settings(settings: Settings, state: State<'_, Mutex<Session>>) -> Result<()> {
    check_settings(&settings)?;
    let mut s = lock(&state)?; authorized(&s)?;
    let mut next = s.vault.clone(); next.settings = settings; persist(&next)?; s.vault = next; Ok(())
}
fn current_settings(state: &State<'_, Mutex<Session>>) -> Result<Settings> {
    let s = lock(state)?; authorized(&s)?; Ok(s.vault.settings.clone())
}
async fn running_vms(settings: &Settings) -> Result<Vec<memu::RunningVm>> {
    if settings.demo_emulators {
        return Ok(vec![
            memu::RunningVm { id: 0, name: "MEmu • exemplo".into(), pid: 12480 },
            memu::RunningVm { id: 1, name: "MEmu 2 • exemplo".into(), pid: 16832 },
        ]);
    }
    let output = memu::run(memu::locate(&settings.memuc_path)?, &["listvms", "--running"]).await?;
    let list = memu::parse_running(&output);
    if list.is_empty() && !output.trim().is_empty() {
        return Err("O MEmu retornou uma resposta não reconhecida; confira a versão e se há instâncias ativas.".into());
    }
    Ok(list)
}

#[tauri::command]
async fn list_emulators(state: State<'_, Mutex<Session>>) -> Result<Vec<memu::Emulator>> {
    let settings = current_settings(&state)?;
    let list = running_vms(&settings).await?;
    Ok(list
        .into_iter()
        .map(|vm| memu::Emulator { id: vm.id, name: vm.name, pid: vm.pid })
        .collect())
}

/// Status de cada emulador: app, PID, socket DevTools e páginas abertas.
#[tauri::command]
async fn list_sessions(
    enable_debug: Option<bool>,
    state: State<'_, Mutex<Session>>,
) -> Result<Vec<webview::SessionReport>> {
    let settings = current_settings(&state)?;
    let enable_debug = enable_debug.unwrap_or(false);
    if settings.demo_emulators {
        return Ok(vec![
            webview::SessionReport {
                vm_id: Some(0), vm_name: "MEmu • exemplo".into(), serial: "127.0.0.1:21503".into(),
                model: "Exemplo".into(), app_running: false, app_pid: None, socket: None,
                debug_enabled: false, pages: vec![],
                error: Some("Instância simulada: nenhum comando foi executado.".into()),
            },
            webview::SessionReport {
                vm_id: Some(1), vm_name: "MEmu 2 • exemplo".into(), serial: "127.0.0.1:21513".into(),
                model: "Exemplo".into(), app_running: false, app_pid: None, socket: None,
                debug_enabled: false, pages: vec![],
                error: Some("Instância simulada: nenhum comando foi executado.".into()),
            },
        ]);
    }
    let vms = running_vms(&settings).await?;
    webview::sessions(
        &settings.adb_path,
        &vms,
        &settings.package_name,
        webview::CAPTCHA_HOST,
        enable_debug,
        false,
    )
    .await
}

/// Decide de onde o worker tira o token: provedores cadastrados, na ordem de
/// prioridade. O proxy vem de cada credencial (Onyx exige; os demais ignoram).
fn token_source_for(settings: &Settings) -> Result<auto_inject::TokenSource> {
    let keys = settings
        .keys
        .iter()
        .map(|key| {
            (
                key.site_id.clone(),
                key.label.clone(),
                key.secret.clone(),
                key.proxy.clone(),
            )
        })
        .collect::<Vec<_>>();
    let credentials = solver::ordered_credentials(&keys, &settings.priority);
    if credentials.is_empty() {
        let unsupported = solver::unsupported_sites(&keys, &settings.priority);
        if !unsupported.is_empty() {
            let names = unsupported
                .iter()
                .map(|(site, _)| site.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Nenhum solver implementado para: {names}. Sites suportados: NoneCap, 2Captcha, Anti-Captcha e Onyx."
            ));
        }
        return Err(
            "Cadastre uma API-key de um site suportado antes de ativar.".into(),
        );
    }
    Ok(auto_inject::TokenSource::Solvers {
        config: solver::SolverConfig {
            timeout: std::time::Duration::from_secs(settings.solver_timeout.max(30)),
            poll_wait: 60,
            proxy: String::new(),
        },
        credentials,
    })
}

/// Liga a injeção automática para um emulador: uma thread verifica a cada
/// segundo se a página do captcha está pronta e injeta o token do arquivo.
#[tauri::command]
fn start_auto_inject(
    vm_id: u32,
    enable_debug: Option<bool>,
    pre_solve: Option<bool>,
    state: State<'_, Mutex<Session>>,
    workers: State<'_, Mutex<Workers>>,
) -> Result<auto_inject::AutoStatus> {
    let settings = current_settings(&state)?;
    if settings.demo_emulators {
        return Err("Instância simulada: nenhuma injeção é executada.".into());
    }
    let token_source = token_source_for(&settings)?;
    let mut map = workers.lock().map_err(|_| "Estado das verificações indisponível.".to_string())?;
    if let Some(existing) = map.get(&vm_id) {
        if existing.status().running {
            return Err("A injeção automática já está ativa neste emulador.".into());
        }
    }
    let worker = auto_inject::AutoInjector::start(
        vm_id,
        token_source,
        settings.adb_path.clone(),
        settings.package_name.clone(),
        enable_debug.unwrap_or(false),
        pre_solve.unwrap_or(false),
        settings
            .use_manual_timing
            .then_some(settings.manual_interval_s as f64),
        settings.use_manual_timing.then_some(settings.manual_solve_s as f64),
    )?;
    let status = worker.status();
    map.insert(vm_id, worker);
    Ok(status)
}

/// Desliga a injeção automática de um emulador.
#[tauri::command]
fn stop_auto_inject(
    vm_id: u32,
    workers: State<'_, Mutex<Workers>>,
) -> Result<auto_inject::AutoStatus> {
    let mut map = workers.lock().map_err(|_| "Estado das verificações indisponível.".to_string())?;
    match map.remove(&vm_id) {
        Some(mut worker) => {
            let mut status = worker.status();
            worker.stop();
            status.running = false;
            Ok(status)
        }
        None => Err("A injeção automática não está ativa neste emulador.".into()),
    }
}

/// Estado atual (contadores, último resultado e último erro).
/// Liga/desliga a pré-resolução de uma instância já em execução.
///
/// Só ativa quando o worker já acumulou tempo médio suficiente (o worker expõe
/// `interval_mean_s`/`solve_mean_s` no status). Sem isso, o botão fica
/// desabilitado na interface — esta checagem é a garantia do backend.
#[tauri::command]
fn set_pre_solve(
    vm_id: u32,
    enabled: bool,
    state: State<'_, Mutex<Session>>,
    workers: State<'_, Mutex<Workers>>,
) -> Result<auto_inject::AutoStatus> {
    let settings = current_settings(&state)?;
    let token_source = token_source_for(&settings)?;
    let map = workers.lock().map_err(|_| "Estado das verificações indisponível.".to_string())?;
    let Some(worker) = map.get(&vm_id) else {
        return Err("Ative a injeção automática antes de usar a pré-resolução.".into());
    };
    let status = worker.status();
    worker.set_pre_solve(enabled)?;
    let _ = token_source;
    let mut updated = status;
    updated.pre_solve = enabled;
    Ok(updated)
}

#[tauri::command]
fn auto_inject_status(
    vm_id: u32,
    workers: State<'_, Mutex<Workers>>,
) -> Result<auto_inject::AutoStatus> {
    let map = workers.lock().map_err(|_| "Estado das verificações indisponível.".to_string())?;
    Ok(map.get(&vm_id).map(|worker| worker.status()).unwrap_or_default())
}

#[tauri::command]
async fn open_android_settings(id: u32, state: State<'_, Mutex<Session>>) -> Result<()> {
    let settings = current_settings(&state)?;
    if settings.demo_emulators { return Err("Instância simulada: nenhum comando foi executado.".into()); }
    let path = memu::locate(&settings.memuc_path)?;
    let list = memu::parse_list(&memu::run(path.clone(), &["listvms", "--running"]).await?);
    if !list.iter().any(|vm| vm.id==id) { return Err("Esse emulador não está mais ativo.".into()); }
    let result = memu::run(path, &["startapp", "-i", &id.to_string(), "com.android.settings/.Settings"]).await?;
    if result.to_ascii_lowercase().contains("error") { return Err("Não foi possível abrir as configurações Android.".into()); }
    Ok(())
}
/// Modo de linha de comando (QA/automação).
///
/// `cargo run -- sessions` lista os emuladores e as páginas do WebView.
/// `cargo run -- inject --serial 127.0.0.1:21513 --file resposta.json`
/// executa exatamente o mesmo caminho Rust usado pela interface.
fn run_cli() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return None;
    }
    let usage = "uso: memu-hub [sessions | auto-inject (--vm <id> | --serial <serial>) --file <json|token> [--enable-debug] [--seconds N] | validate-settings]";
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("falha ao iniciar runtime: {error}");
            return Some(1);
        }
    };
    let code = match args[0].as_str() {
        // QA: exercita a mesma validação usada por save_settings, sem tocar no
        // cofre. Lê um Settings em JSON do stdin e devolve exit 0/1.
        "validate-settings" => {
            use std::io::Read;

            let mut buffer = String::new();
            if let Err(error) = std::io::stdin().read_to_string(&mut buffer) {
                eprintln!("falha ao ler stdin: {error}");
                return Some(2);
            }
            match serde_json::from_str::<Settings>(&buffer) {
                Ok(value) => match check_settings(&value) {
                    Ok(()) => {
                        println!("válido");
                        0
                    }
                    Err(error) => {
                        eprintln!("{error}");
                        1
                    }
                },
                Err(error) => {
                    eprintln!("JSON inválido: {error}");
                    2
                }
            }
        }
        "sessions" => runtime.block_on(async {
            let vms = memu::parse_running(
                &memu::run(
                    match memu::locate("") {
                        Ok(path) => path,
                        Err(error) => {
                            eprintln!("{error}");
                            return 1;
                        }
                    },
                    &["listvms", "--running"],
                )
                .await
                .unwrap_or_default(),
            );
            match webview::sessions(
                "", &vms, webview::DEFAULT_PACKAGE, webview::CAPTCHA_HOST,
                std::env::var("MEMU_HUB_ENABLE_DEBUG").is_ok(), false,
            ).await {
                Ok(reports) => {
                    println!("{}", serde_json::to_string_pretty(&reports).unwrap_or_default());
                    0
                }
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }),
        // QA: executa o mesmo worker da interface por N segundos e imprime o
        // estado final. Não abre a janela.
        // QA: roda o solver sem emulador e imprime o token. Útil para validar
        // o pipeline CaptchaSonic nativo (Chrome via CDP, sem Node).
        "solve-test" => {
            let mut provider = "nonecap".to_string();
            let mut key = String::new();
            let mut sitekey = webview::SITEKEY.to_string();
            let mut proxy = std::env::var("ONYX_PROXY").unwrap_or_default();
            let mut index = 1;
            while index < args.len() {
                match args[index].as_str() {
                    "--provider" => {
                        index += 1;
                        provider = args.get(index).cloned().unwrap_or_default();
                    }
                    "--key" => {
                        index += 1;
                        key = args.get(index).cloned().unwrap_or_default();
                    }
                    "--sitekey" => {
                        index += 1;
                        sitekey = args.get(index).cloned().unwrap_or_default();
                    }
                    "--proxy" => {
                        index += 1;
                        proxy = args.get(index).cloned().unwrap_or_default();
                    }
                    "--balance" => {}
                    other => {
                        eprintln!("argumento desconhecido: {other}");
                        return Some(2);
                    }
                }
                index += 1;
            }
            if key.is_empty() {
                key = std::env::var(if provider.eq_ignore_ascii_case("nonecap") {
                    "NONECAP_KEY"
                } else if provider.eq_ignore_ascii_case("2captcha")
                    || provider.eq_ignore_ascii_case("twocaptcha")
                {
                    "TWOCAPTCHA_KEY"
                } else if provider.eq_ignore_ascii_case("anticaptcha")
                    || provider.eq_ignore_ascii_case("anti-captcha")
                {
                    "ANTICAPTCHA_KEY"
                } else {
                    "ONYX_KEY"
                })
                .unwrap_or_default();
            }
            if key.is_empty() {
                eprintln!("informe --key ou defina a variável de ambiente");
                return Some(2);
            }
            if args.iter().any(|a| a == "--balance") {
                let result = if provider.eq_ignore_ascii_case("nonecap") {
                    // A NoneCap não tem endpoint de saldo; valida a chave ao usar.
                    return Some(2);
                } else if provider.eq_ignore_ascii_case("2captcha")
                    || provider.eq_ignore_ascii_case("twocaptcha")
                {
                    solver::twocaptcha_balance(&key)
                } else if provider.eq_ignore_ascii_case("anticaptcha")
                    || provider.eq_ignore_ascii_case("anti-captcha")
                {
                    solver::anticaptcha_balance(&key)
                } else if provider.eq_ignore_ascii_case("onyx")
                    || provider.eq_ignore_ascii_case("onyxsolver")
                {
                    solver::onyx_balance(&key)
                } else {
                    return Some(2);
                };
                return Some(match result {
                    Ok(balance) => {
                        println!("saldo: US$ {balance:.5}");
                        0
                    }
                    Err(error) => {
                        eprintln!("{error}");
                        1
                    }
                });
            }
            let credentials = vec![solver::SolverCredential {
                kind: solver::SolverKind::from_site_id(&provider)
                    .unwrap_or(solver::SolverKind::Nonecap),
                site_id: provider.clone(),
                label: "qa".into(),
                secret: key,
                proxy: proxy.clone(),
            }];
            let config = solver::SolverConfig {
                timeout: std::time::Duration::from_secs(240),
                poll_wait: 60,
                proxy,
            };
            let started = std::time::Instant::now();
            match solver::solve_with_priority(
                &config, &credentials, &sitekey, webview::SITE_URL,
            ) {
                Ok(result) => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "provider": result.provider,
                            "token_length": result.token.len(),
                            "token_prefix": result.token.chars().take(12).collect::<String>(),
                            "expiration": result.expiration,
                            "elapsed_ms": started.elapsed().as_millis(),
                        }))
                        .unwrap_or_default()
                    );
                    0
                }
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }
        "auto-inject" => {
            let mut vm_id: Option<u32> = None;
            let mut serial = String::new();
            let mut enable_debug = false;
            let mut seconds: u64 = 15;
            let mut index = 1;
            while index < args.len() {
                match args[index].as_str() {
                    "--serial" => {
                        index += 1;
                        serial = args.get(index).cloned().unwrap_or_default();
                    }
                    "--vm" => {
                        index += 1;
                        vm_id = args.get(index).and_then(|value| value.parse().ok());
                    }
                    "--enable-debug" => enable_debug = true,
                    "--seconds" => {
                        index += 1;
                        seconds = args
                            .get(index)
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(15);
                    }
                    other => {
                        eprintln!("argumento desconhecido: {other}");
                        eprintln!("{usage}");
                        return Some(2);
                    }
                }
                index += 1;
            }
            let id = match (vm_id, serial.is_empty()) {
                (Some(id), _) => id,
                (None, false) => match webview::infer_vm_id(&serial) {
                    Some(id) => id,
                    None => {
                        eprintln!("use --vm <id> para este serial");
                        return Some(2);
                    }
                },
                (None, true) => {
                    eprintln!("informe --vm <id> ou --serial");
                    return Some(2);
                }
            };
            let token_source = auto_inject::TokenSource::Solvers {
                config: solver::SolverConfig {
                    timeout: std::time::Duration::from_secs(180),
                    poll_wait: 60,
                    proxy: String::new(),
                },
                credentials: solver::ordered_credentials(
                    &[
                        (
                            "nonecap".to_string(),
                            "env".to_string(),
                            std::env::var("NONECAP_KEY").unwrap_or_default(),
                            String::new(),
                        ),
                        (
                            "2captcha".to_string(),
                            "env".to_string(),
                            std::env::var("TWOCAPTCHA_KEY").unwrap_or_default(),
                            String::new(),
                        ),
                        (
                            "anticaptcha".to_string(),
                            "env".to_string(),
                            std::env::var("ANTICAPTCHA_KEY").unwrap_or_default(),
                            String::new(),
                        ),
                        (
                            "onyx".to_string(),
                            "env".to_string(),
                            std::env::var("ONYX_KEY").unwrap_or_default(),
                            std::env::var("ONYX_PROXY").unwrap_or_default(),
                        ),
                    ],
                    &[
                        "nonecap".to_string(),
                        "2captcha".to_string(),
                        "anticaptcha".to_string(),
                        "onyx".to_string(),
                    ],
                )
                .into_iter()
                .filter(|credential| !credential.secret.trim().is_empty())
                .collect(),
            };
            let mut worker = match auto_inject::AutoInjector::start(
                id,
                token_source,
                String::new(),
                webview::DEFAULT_PACKAGE.to_string(),
                enable_debug,
                false,
                None,
                None,
            ) {
                Ok(worker) => worker,
                Err(error) => {
                    eprintln!("{error}");
                    return Some(1);
                }
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
            while std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(250));
                let status = worker.status();
                if status.injections > 0 {
                    break;
                }
            }
            let status = worker.status();
            worker.stop();
            println!("{}", serde_json::to_string_pretty(&status).unwrap_or_default());
            if status.injections > 0 { 0 } else { 1 }
        }
        "--help" | "-h" | "help" => {
            println!("{usage}");
            0
        }
        other => {
            eprintln!("comando desconhecido: {other}");
            eprintln!("{usage}");
            2
        }
    };
    Some(code)
}

fn main() {
    if let Some(code) = run_cli() {
        std::process::exit(code);
    }
    let session = match vault::read() {
        Ok(Some(bytes)) => match serde_json::from_slice(&bytes) {
            Ok(vault) => Session { vault, ..Default::default() },
            Err(_) => Session { load_error: Some("Dados salvos inválidos. Consulte a recuperação no README.".into()), ..Default::default() },
        },
        Ok(None) => Session::default(),
        Err(e) => Session { load_error: Some(e), ..Default::default() },
    };
    tauri::Builder::default()
        .manage(Mutex::new(session))
        .manage(Mutex::new(Workers::new()))
        .invoke_handler(tauri::generate_handler![
            authenticate, is_demo, logout, get_settings, save_settings,
            list_emulators, list_sessions, start_auto_inject, stop_auto_inject,
            auto_inject_status, set_pre_solve, open_android_settings
        ])
        .run(tauri::generate_context!()).expect("Falha ao iniciar MEmu Hub");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(id: &str, icon: &str) -> Site {
        Site { id: id.into(), name: format!("Site {id}"), icon: icon.into() }
    }

    fn key(id: &str, site_id: &str) -> ApiKey {
        ApiKey { id: id.into(), site_id: site_id.into(), label: "Conta".into(), secret: "s".into(), proxy: String::new() }
    }

    fn settings_with(sites: Vec<Site>, keys: Vec<ApiKey>) -> Settings {
        let mut priority: Vec<String> = keys.iter().map(|k| k.site_id.clone()).collect();
        priority.dedup();
        Settings { sites, keys, priority, ..Settings::default() }
    }

    #[test]
    fn accepts_catalog_icon_path() {
        // Regressão: o ícone do catálogo é um caminho e era recusado por "até 2 chars".
        assert!(check_icon("assets/logos/capsonic.svg"));
        assert!(check_icon("assets/logos/nonecap.webp"));
        assert!(check_icon(".\\assets\\logos\\capsonic.svg"));
        assert!(check_icon("./assets/logos/x.png"));
        let value = settings_with(vec![site("capsonic", "assets/logos/capsonic.svg")], vec![]);
        assert_eq!(check_settings(&value), Ok(()));
    }

    #[test]
    fn accepts_short_text_icon() {
        assert!(check_icon("C"));
        assert!(check_icon("AB"));
        let value = settings_with(vec![site("a", "A")], vec![]);
        assert_eq!(check_settings(&value), Ok(()));
    }

    #[test]
    fn rejects_unsafe_icons() {
        for bad in ["", "   ", "ABC", "..\\evil.png", "C:\\x.png", "/etc/x", "assets/../../x", "assets//x", "assets/lo go.png", "https://x/y.png"] {
            assert!(!check_icon(bad), "deveria rejeitar {bad:?}");
        }
    }

    #[test]
    fn accepts_two_keys_for_the_same_site() {
        // Regressão do relato: adicionar a segunda chave não pode falhar.
        let value = settings_with(
            vec![site("capsonic", "assets/logos/capsonic.svg")],
            vec![key("k1", "capsonic"), key("k2", "capsonic")],
        );
        assert_eq!(check_settings(&value), Ok(()));
    }

    #[test]
    fn rejects_duplicate_site_ids() {
        let value = settings_with(
            vec![site("a", "A"), site("a", "B")],
            vec![],
        );
        assert!(check_settings(&value).is_err());
    }

    #[test]
    fn rejects_key_for_unknown_site() {
        let value = settings_with(vec![site("a", "A")], vec![key("k1", "missing")]);
        assert!(check_settings(&value).is_err());
    }

    #[test]
    fn rejects_inconsistent_priority() {
        let mut value = settings_with(
            vec![site("a", "A")],
            vec![key("k1", "a")],
        );
        value.priority = vec![];
        assert!(check_settings(&value).is_err());
    }

    #[test]
    fn accepts_empty_catalog_after_saving_site_with_key() {
        // Estado inicial do app: nenhum site, nenhuma chave.
        let value = Settings::default();
        assert_eq!(check_settings(&value), Ok(()));
    }
}

#[cfg(test)]
mod message_tests {
    use super::*;

    #[test]
    fn icon_error_names_the_site() {
        let value = Settings {
            sites: vec![Site { id: "x".into(), name: "CapSonic".into(), icon: "ABC".into() }],
            ..Settings::default()
        };
        let error = check_settings(&value).unwrap_err();
        assert!(error.contains("CapSonic"), "{error}");
        assert!(error.contains("assets/"), "{error}");
    }

    #[test]
    fn duplicate_error_names_the_site() {
        let value = Settings {
            sites: vec![
                Site { id: "a".into(), name: "Repetido".into(), icon: "A".into() },
                Site { id: "a".into(), name: "Repetido".into(), icon: "B".into() },
            ],
            ..Settings::default()
        };
        let error = check_settings(&value).unwrap_err();
        assert!(error.contains("Repetido"), "{error}");
    }
}
