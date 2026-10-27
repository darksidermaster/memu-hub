//! Obtenção do token do captcha pelos provedores configurados.
//!
//! Somente implementações **nativas em Rust** (sem Node, sem Chrome, sem
//! subprocesso). Todos falam HTTP diretamente e devolvem o token `P1_...` do
//! hCaptcha:
//!
//! * **NoneCap** (`https://api.nonecap.com/v1`) — `POST /solves` com Bearer key,
//!   poll até `status = solved`; token em `solve.token`. Validado end-to-end.
//! * **2Captcha** (`https://2captcha.com`) — API v1 (`in.php`/`res.php`), o
//!   caminho do SDK oficial para hCaptcha (`method=hcaptcha`).
//! * **AntiCaptcha** (`https://api.anti-captcha.com`) — `createTask` com
//!   `HCaptchaTaskProxyless` + `getTaskResult`. Validado end-to-end.
//! * **Onyx** (`https://onyxsolver.io`) — `PopularCaptchaTask` (exige proxy) +
//!   `/api/getTaskResult`. Validado end-to-end.
//!
//! O app nunca resolve o captcha localmente: orquestra os provedores que o
//! usuário cadastrou, na ordem de prioridade.

use crate::http;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Provedor usado para obter o token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SolverKind {
    /// NoneCap (`https://api.nonecap.com/v1`).
    Nonecap,
    /// 2Captcha (v1: `in.php` + `res.php`, `method=hcaptcha`).
    Twocaptcha,
    /// AntiCaptcha (`https://api.anti-captcha.com`).
    Anticaptcha,
    /// Onyx (`https://onyxsolver.io`, `PopularCaptchaTask`).
    Onyx,
}

impl SolverKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SolverKind::Nonecap => "nonecap",
            SolverKind::Twocaptcha => "2captcha",
            SolverKind::Anticaptcha => "anticaptcha",
            SolverKind::Onyx => "onyx",
        }
    }

    /// Converte o id do catálogo (`ui/sites.json` → `NoneCap` / `2Captcha` ...).
    pub fn from_site_id(id: &str) -> Option<Self> {
        match id.trim().to_ascii_lowercase().as_str() {
            "nonecap" => Some(SolverKind::Nonecap),
            "2captcha" | "twocaptcha" => Some(SolverKind::Twocaptcha),
            "anticaptcha" | "anti-captcha" => Some(SolverKind::Anticaptcha),
            "onyx" | "onyxsolver" => Some(SolverKind::Onyx),
            _ => None,
        }
    }
}

/// Credencial de um provedor.
#[derive(Clone, Debug)]
pub struct SolverCredential {
    pub kind: SolverKind,
    pub site_id: String,
    pub label: String,
    pub secret: String,
    /// Proxy desta credencial (Onyx exige). Formato `user:pass@host:port`.
    pub proxy: String,
}

/// Resposta pronta para injeção (mesmo formato do `full_solve.js`).
#[derive(Clone, Debug, Serialize)]
pub struct SolverResult {
    pub token: String,
    pub expiration: Option<f64>,
    pub provider: String,
}

/// Configuração de rede/comando do solver.
#[derive(Clone, Debug)]
pub struct SolverConfig {
    /// Timeout total de uma resolução.
    pub timeout: Duration,
    /// `wait` da NoneCap (segundos por chamada).
    pub poll_wait: u64,
    /// Proxy para provedores que exigem (Onyx Popular/hCaptcha).
    /// Formato `user:pass@host:port` (sem esquema, para não acionar o WAF).
    pub proxy: String,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(180),
            poll_wait: 60,
            proxy: String::new(),
        }
    }
}

// -- NoneCap (HTTP puro) ----------------------------------------------------

#[derive(Debug, Deserialize)]
struct NoneCapSolve {
    #[serde(default)]
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    error: Option<NoneCapError>,
}

#[derive(Debug, Deserialize)]
struct NoneCapError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

fn nonecap_request(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    key: &str,
    body: Option<serde_json::Value>,
) -> Result<(u16, serde_json::Value), String> {
    let request = match method {
        "POST" => agent.post(url),
        _ => agent.get(url),
    }
    .set("Authorization", &format!("Bearer {key}"))
    .set("Content-Type", "application/json");
    let response = match body {
        Some(payload) => request.send_json(payload),
        None => request.call(),
    };
    match response {
        Ok(response) => {
            let status = response.status();
            let text = response
                .into_string()
                .map_err(|error| format!("resposta da NoneCap ilegível: {error}"))?;
            let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            Ok((status, json))
        }
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            Ok((status, json))
        }
        Err(error) => Err(format!("falha de rede na NoneCap: {error}")),
    }
}

/// Resolve via NoneCap, replicando `solveHcaptcha` do `lib/nonecap.js`.
pub fn solve_nonecap(
    config: &SolverConfig,
    credential: &SolverCredential,
    sitekey: &str,
    site_url: &str,
) -> Result<SolverResult, String> {
    if credential.secret.trim().is_empty() {
        return Err("API-key da NoneCap vazia.".into());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout(Duration::from_secs(config.poll_wait + 30))
        .build();
    let base = "https://api.nonecap.com/v1";
    let deadline = Instant::now() + config.timeout;

    let (status, json) = nonecap_request(
        &agent,
        "POST",
        &format!("{base}/solves?wait={}", config.poll_wait),
        &credential.secret,
        Some(serde_json::json!({
            "type": "hcaptcha",
            "sitekey": sitekey,
            "url": site_url,
        })),
    )?;
    if matches!(status, 401 | 402 | 403 | 422) {
        let code = json
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(|code| code.as_str())
            .unwrap_or("");
        let message = json
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(|message| message.as_str())
            .unwrap_or("erro");
        return Err(format!("NoneCap {status} {code}: {message}"));
    }
    let mut solve: NoneCapSolve = serde_json::from_value(json)
        .map_err(|error| format!("resposta da NoneCap inesperada: {error}"))?;

    let mut wait = 15u64;
    while !matches!(solve.status.as_str(), "solved" | "failed" | "cancelled" | "expired")
        && Instant::now() < deadline
    {
        if solve.id.is_empty() {
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now()).as_secs();
        if remaining == 0 {
            break;
        }
        let call_wait = wait.min(remaining).max(1);
        let (_, json) = nonecap_request(
            &agent,
            "GET",
            &format!("{base}/solves/{}?wait={call_wait}", solve.id),
            &credential.secret,
            None,
        )?;
        solve = serde_json::from_value(json)
            .map_err(|error| format!("resposta da NoneCap inesperada: {error}"))?;
        wait = ((wait as f64 * 1.5) as u64).clamp(10, 90);
    }

    if solve.status == "solved" {
        if let Some(token) = solve.token.filter(|token| !token.trim().is_empty()) {
            return Ok(SolverResult {
                token: token.trim().to_string(),
                expiration: Some(120.0),
                provider: SolverKind::Nonecap.as_str().to_string(),
            });
        }
    }
    let detail = solve
        .error
        .map(|error| {
            if error.code.is_empty() {
                error.message
            } else {
                format!("{}: {}", error.code, error.message)
            }
        })
        .unwrap_or_else(|| format!("não resolvido (status: {})", solve.status));
    Err(format!("NoneCap: {detail}"))
}

// -- Onyx (HTTP puro) -------------------------------------------------------
//
// Base: https://onyxsolver.io
//   POST /api/createTask    { clientKey, task }  -> { errorId, taskId }
//   POST /api/getTaskResult { clientKey, taskId } -> { status, solution }
//   POST /api/getBalance    { clientKey }        -> { balance, solves, allowed_types, maintenance }
//
// `PopularCaptchaTask` é o tipo de hCaptcha. **Exige proxy** no formato
// `user:pass@host:port` — sem ele: `ERROR_INVALID_TASK "requires a proxy"`.
// Já `TurnstileTaskProxyless` não usa proxy.
//
// Validado em out/2026 com token `P1_...` real em ~48 s:
//   {"errorId":0,"status":"ready",
//    "solution":{"gRecaptchaResponse":"P1_eyJ...","userAgent":"Mozilla/5.0 ..."}}
//
// Cuidado com o WAF (DiamWall) na frente da API: o padrão literal
// `user:pass@127.0.0.1:porta` é bloqueado com 403. Por isso o proxy é enviado
// como `user:pass@host:port` (sem esquema) e endereços locais não são usados.

const ONYX_BASE: &str = "https://onyxsolver.io";

/// Traduz os erros da Onyx em ação.
pub fn onyx_error_hint(code: &str, description: &str) -> String {
    let text = description.to_ascii_lowercase();
    if text.contains("requires a proxy") || text.contains("proxy") && text.contains("valid") {
        return "PopularCaptchaTask (hCaptcha) exige proxy: configure um proxy em user:pass@host:port."
            .into();
    }
    if text.contains("failed to connect via the supplied proxy") {
        return "A Onyx não conseguiu conectar no proxy informado; verifique host, porta e credenciais."
            .into();
    }
    match code {
        "ERROR_INVALID_TASK" => "Tarefa inválida: confira type, websiteURL, websiteKey e proxy.".into(),
        "ERROR_KEY_DOES_NOT_EXIST" | "" if text.contains("key") => {
            "Verifique a API-key no painel da Onyx.".into()
        }
        "ERROR_ZERO_BALANCE" => "A conta da Onyx está sem saldo.".into(),
        _ => String::new(),
    }
}

fn onyx_post(path: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    let url = format!("{ONYX_BASE}{path}");
    let (status, value) = http::post_json(&url, body)
        .map_err(|error| format!("Onyx {path}: {error}"))?;
    if status == 403 {
        return Err(
            "Onyx: requisição bloqueada pelo WAF (403 DiamWall). Evite proxies com endereço local ou padrões que o WAF recuse."
                .into(),
        );
    }
    if status >= 400 && value.is_null() {
        return Err(format!("Onyx {path}: HTTP {status}"));
    }
    Ok(value)
}

/// Consulta saldo, solves e o que a conta pode usar.
pub fn onyx_balance(api_key: &str) -> Result<f64, String> {
    if api_key.trim().is_empty() {
        return Err("API-key da Onyx vazia.".into());
    }
    let value = onyx_post(
        "/api/getBalance",
        serde_json::json!({ "clientKey": api_key.trim() }),
    )?;
    let error_id = value
        .get("errorId")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    if error_id != 0 {
        let description = value
            .get("errorDescription")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("erro");
        return Err(format!("Onyx getBalance: {description}"));
    }
    let balance = value
        .get("balance")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let kinds = value
        .get("allowed_kinds")
        .map(|kinds| kinds.to_string())
        .unwrap_or_default();
    let popular = value
        .pointer("/maintenance/popular")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if popular {
        return Err("Onyx: o serviço de Popular Captcha (hCaptcha) está em manutenção.".into());
    }
    if !kinds.contains("popular") {
        return Err(format!(
            "Onyx: a conta não tem acesso a PopularCaptchaTask (hCaptcha). Tipos: {kinds}"
        ));
    }
    Ok(balance)
}

/// Resolve hCaptcha pela Onyx (`PopularCaptchaTask`, requer proxy).
///
/// O proxy vem de `SolverConfig.proxy`. Sem ele o provedor recusa, então a
/// mensagem orienta a configurar.
pub fn solve_onyx(
    config: &SolverConfig,
    credential: &SolverCredential,
    sitekey: &str,
    site_url: &str,
) -> Result<SolverResult, String> {
    if credential.secret.trim().is_empty() {
        return Err("API-key da Onyx vazia.".into());
    }
    let proxy = config.proxy.trim();
    if proxy.is_empty() {
        return Err(
            "Onyx PopularCaptchaTask (hCaptcha) exige proxy. Configure o proxy (user:pass@host:port) nas Configurações."
                .into(),
        );
    }
    let key = credential.secret.trim();

    let created = onyx_post(
        "/api/createTask",
        serde_json::json!({
            "clientKey": key,
            "task": {
                "type": "PopularCaptchaTask",
                "websiteURL": site_url,
                "websiteKey": sitekey,
                "proxy": proxy,
            }
        }),
    )?;
    let error_id = created
        .get("errorId")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let task_id = created
        .get("taskId")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty());
    let task_id = match (error_id, task_id) {
        (0, Some(task_id)) => task_id.to_string(),
        _ => {
            let code = created
                .get("errorCode")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let description = created
                .get("errorDescription")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("erro");
            let hint = onyx_error_hint(code, description);
            return Err(format!("Onyx createTask: {code} {description} {hint}")
                .trim()
                .to_string());
        }
    };

    let deadline = Instant::now() + config.timeout;
    loop {
        if Instant::now() >= deadline {
            return Err(format!(
                "Onyx não resolveu em {}s (task {task_id})",
                config.timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_secs(4));
        let result = onyx_post(
            "/api/getTaskResult",
            serde_json::json!({ "clientKey": key, "taskId": task_id }),
        )?;
        let error_id = result
            .get("errorId")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        if error_id != 0 {
            let code = result
                .get("errorCode")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let description = result
                .get("errorDescription")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("erro");
            let hint = onyx_error_hint(code, description);
            return Err(format!("Onyx getTaskResult: {code} {description} {hint}")
                .trim()
                .to_string());
        }
        match result
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
        {
            "ready" => {
                let token = result
                    .pointer("/solution/gRecaptchaResponse")
                    .and_then(serde_json::Value::as_str)
                    .filter(|token| !token.trim().is_empty())
                    .ok_or("Onyx ready sem solution.gRecaptchaResponse")?;
                return Ok(SolverResult {
                    token: token.trim().to_string(),
                    expiration: Some(120.0),
                    provider: SolverKind::Onyx.as_str().to_string(),
                });
            }
            "failed" => return Err("Onyx: tarefa falhou".into()),
            _ => {}
        }
    }
}




//
// Documentação: https://anti-captcha.com/apidoc
//   POST https://api.anti-captcha.com/createTask    { clientKey, task }
//   POST https://api.anti-captcha.com/getTaskResult { clientKey, taskId }
//   POST https://api.anti-captcha.com/getBalance    { clientKey }
//
// A documentação do SDK oficial (anticaptchaofficial) usa
// `HCaptchaTaskProxyless` com `websiteURL`/`websiteKey`, e o resultado vem em
// `solution.gRecaptchaResponse` (o token `P1_...` do hCaptcha), junto com o
// `userAgent` usado na resolução.
//
// Validado com a conta de teste: saldo US$ 0,05 (50 créditos), tarefa criada e
// token real recebido em ~100 s.

const ANTICAPTCHA_BASE: &str = "https://api.anti-captcha.com";

/// Erros transitórios da AntiCaptcha (vale repetir o poll).
pub fn anticaptcha_retryable(code: &str) -> bool {
    matches!(
        code,
        "ERROR_NO_SLOT_AVAILABLE" | "ERROR_SERVER_ERROR" | "ERROR_INTERNAL_SERVER_ERROR"
    )
}

/// Traduz o erro em ação para o usuário.
pub fn anticaptcha_error_hint(code: &str) -> &'static str {
    match code {
        "ERROR_KEY_DOES_NOT_EXIST" | "ERROR_WRONG_USER_KEY" => {
            "Verifique a API-key no painel da AntiCaptcha."
        }
        "ERROR_ZERO_BALANCE" => "A conta da AntiCaptcha está sem saldo.",
        "ERROR_IP_NOT_ALLOWED" | "ERROR_IP_BLOCKED" => {
            "A conta tem restrição de IP: libere este IP no painel da AntiCaptcha."
        }
        "ERROR_TASK_NOT_SUPPORTED" => "A conta não tem acesso a tarefas de hCaptcha.",
        "ERROR_CAPTCHA_UNSOLVABLE" => "Os workers não conseguiram resolver este captcha.",
        "ERROR_WRONG_WEBSITE_KEY" | "ERROR_WRONG_WEBSITE_URL" => {
            "sitekey ou URL da página foram recusados."
        }
        _ => "",
    }
}

fn anticaptcha_post(path: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    let url = format!("{ANTICAPTCHA_BASE}{path}");
    let (status, value) = http::post_json(&url, body)
        .map_err(|error| format!("AntiCaptcha {path}: {error}"))?;
    if status >= 400 && value.is_null() {
        return Err(format!("AntiCaptcha {path}: HTTP {status}"));
    }
    Ok(value)
}

fn anticaptcha_error(value: &serde_json::Value) -> Option<(String, String, i64)> {
    let id = value
        .get("errorId")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    if id == 0 {
        return None;
    }
    Some((
        value
            .get("errorCode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        value
            .get("errorDescription")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        id,
    ))
}

/// Consulta saldo e créditos (valida a chave sem criar tarefa).
pub fn anticaptcha_balance(api_key: &str) -> Result<f64, String> {
    if api_key.trim().is_empty() {
        return Err("API-key da AntiCaptcha vazia.".into());
    }
    let value = anticaptcha_post(
        "/getBalance",
        serde_json::json!({ "clientKey": api_key.trim() }),
    )?;
    if let Some((code, description, _)) = anticaptcha_error(&value) {
        let hint = anticaptcha_error_hint(&code);
        return Err(format!("AntiCaptcha saldo: {code} {description} {hint}")
            .trim()
            .to_string());
    }
    value
        .get("balance")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| "AntiCaptcha getBalance sem 'balance'".to_string())
}

/// Resolve hCaptcha pela AntiCaptcha.
pub fn solve_anticaptcha(
    config: &SolverConfig,
    credential: &SolverCredential,
    sitekey: &str,
    site_url: &str,
) -> Result<SolverResult, String> {
    if credential.secret.trim().is_empty() {
        return Err("API-key da AntiCaptcha vazia.".into());
    }
    let key = credential.secret.trim();

    // 1. Cria a tarefa.
    let created = anticaptcha_post(
        "/createTask",
        serde_json::json!({
            "clientKey": key,
            "task": {
                "type": "HCaptchaTaskProxyless",
                "websiteURL": site_url,
                "websiteKey": sitekey,
            }
        }),
    )?;
    if let Some((code, description, _)) = anticaptcha_error(&created) {
        let hint = anticaptcha_error_hint(&code);
        return Err(format!("AntiCaptcha createTask: {code} {description} {hint}")
            .trim()
            .to_string());
    }
    let task_id = created
        .get("taskId")
        .map(|value| {
            value
                .as_i64()
                .map(|number| number.to_string())
                .or_else(|| value.as_str().map(str::to_string))
                .unwrap_or_default()
        })
        .filter(|value| !value.is_empty())
        .ok_or("AntiCaptcha createTask sem taskId")?;

    // 2. Poll até o token (a resolução costuma levar ~30-100 s).
    let deadline = Instant::now() + config.timeout;
    let mut wait = Duration::from_secs(5);
    loop {
        if Instant::now() >= deadline {
            return Err(format!(
                "AntiCaptcha não resolveu em {}s (task {task_id})",
                config.timeout.as_secs()
            ));
        }
        std::thread::sleep(wait);
        let result = anticaptcha_post(
            "/getTaskResult",
            serde_json::json!({ "clientKey": key, "taskId": task_id }),
        )?;
        if let Some((code, description, _)) = anticaptcha_error(&result) {
            if anticaptcha_retryable(&code) {
                wait = (wait * 2).min(Duration::from_secs(10));
                continue;
            }
            let hint = anticaptcha_error_hint(&code);
            return Err(format!("AntiCaptcha getTaskResult: {code} {description} {hint}")
                .trim()
                .to_string());
        }
        match result
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
        {
            "ready" => {
                let token = result
                    .pointer("/solution/gRecaptchaResponse")
                    .and_then(serde_json::Value::as_str)
                    .filter(|token| !token.trim().is_empty())
                    .ok_or("AntiCaptcha ready sem solution.gRecaptchaResponse")?;
                return Ok(SolverResult {
                    token: token.trim().to_string(),
                    expiration: Some(120.0),
                    provider: SolverKind::Anticaptcha.as_str().to_string(),
                });
            }
            "failed" => return Err("AntiCaptcha: tarefa falhou".into()),
            // "processing": continua.
            _ => wait = (wait * 2).min(Duration::from_secs(10)),
        }
    }
}

// -- 2Captcha (API v1) ------------------------------------------------------
//
// O SDK oficial resolve hCaptcha por `method=hcaptcha` no `in.php` (v1), e não
// pelos tipos da v2. Fluxo:
//   POST https://2captcha.com/in.php   key, method=hcaptcha, sitekey, pageurl, json=1
//        -> {"status":1,"request":"<id>"}  |  {"status":0,"request":"ERROR_..."}
//   GET  https://2captcha.com/res.php  key, action=get, id, json=1
//        -> {"status":1,"request":"<token>"} | {"status":0,"request":"CAPCHA_NOT_READY"}
//   GET  https://2captcha.com/res.php  key, action=getbalance
//
// Estado com a conta de teste: saldo responde (US$ 14,93831), mas `in.php`
// devolve ERROR_IP_NOT_ALLOWED porque a conta tem whitelist de IP e esta máquina
// não está nela. Basta liberar o IP no painel para o fluxo funcionar; nada no
// cliente depende disso.

const TWOCAPTCHA_BASE: &str = "https://2captcha.com";

/// Erros do 2Captcha que valem nova tentativa.
pub fn twocaptcha_retryable(code: &str) -> bool {
    matches!(
        code,
        "ERROR_NO_SLOT_AVAILABLE"
            | "ERROR_SERVER_ERROR"
            | "ERROR_INTERNAL_SERVER_ERROR"
            | "ERROR_BAD_GATEWAY"
    )
}

/// Erros que exigem ação do usuário (chave, saldo, IP liberado).
pub fn twocaptcha_error_hint(code: &str) -> &'static str {
    match code {
        "ERROR_WRONG_USER_KEY" | "ERROR_KEY_DOES_NOT_EXIST" => {
            "Verifique a API-key no painel do 2Captcha."
        }
        "ERROR_ZERO_BALANCE" => "A conta está sem saldo.",
        "ERROR_IP_NOT_ALLOWED" => {
            "A conta tem whitelist de IP: libere este IP no painel do 2Captcha (aba de configurações da API)."
        }
        "ERROR_NO_SLOT_AVAILABLE" => "Sem workers disponíveis no momento; tente de novo.",
        "ERROR_WRONG_GOOGLEKEY" | "ERROR_WRONG_PAGEURL" => {
            "sitekey ou URL da página foram recusados."
        }
        "ERROR_CAPTCHA_UNSOLVABLE" => "Os workers não conseguiram resolver este captcha.",
        _ => "",
    }
}

fn twocaptcha_get(url: &str) -> Result<serde_json::Value, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("runtime HTTP: {error}"))?;
    runtime.block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| format!("cliente HTTP: {error}"))?;
        let response = client
            .get(url)
            .header("User-Agent", "memu-hub/1.0")
            .send()
            .await
            .map_err(|error| format!("GET {url} falhou: {error}"))?;
        let text = response
            .text()
            .await
            .map_err(|error| format!("resposta ilegível: {error}"))?;
        serde_json::from_str(&text)
            .map_err(|_| format!("resposta não-JSON do 2Captcha: {}", text.chars().take(120).collect::<String>()))
    })
}

/// Lê `status`/`request` do formato v1.
///
/// `request` pode ser string (id, token, `ERROR_*`) ou número (saldo, como
/// `{"status":1,"request":0.1}`), por isso os dois casos são tratados.
fn twocaptcha_reply(value: &serde_json::Value) -> (bool, String) {
    let ok = value.get("status").and_then(serde_json::Value::as_i64) == Some(1);
    let request = match value.get("request") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Number(number)) => number.to_string(),
        Some(serde_json::Value::Bool(flag)) => flag.to_string(),
        _ => String::new(),
    };
    (ok, request)
}

/// Consulta o saldo (não passa pela validação de IP da criação de tarefa).
pub fn twocaptcha_balance(api_key: &str) -> Result<f64, String> {
    if api_key.trim().is_empty() {
        return Err("API-key do 2Captcha vazia.".into());
    }
    let url = format!(
        "{TWOCAPTCHA_BASE}/res.php?key={}&action=getbalance&json=1",
        api_key.trim()
    );
    let value = twocaptcha_get(&url)?;
    let (ok, request) = twocaptcha_reply(&value);
    if !ok {
        let hint = twocaptcha_error_hint(&request);
        return Err(format!("2Captcha saldo: {request} {hint}").trim().to_string());
    }
    request
        .parse::<f64>()
        .map_err(|_| format!("saldo inesperado: {request}"))
}

/// Resolve hCaptcha pelo 2Captcha (v1, `method=hcaptcha`).
pub fn solve_twocaptcha(
    config: &SolverConfig,
    credential: &SolverCredential,
    sitekey: &str,
    site_url: &str,
) -> Result<SolverResult, String> {
    if credential.secret.trim().is_empty() {
        return Err("API-key do 2Captcha vazia.".into());
    }
    let key = credential.secret.trim();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("runtime HTTP: {error}"))?;

    // 1. Cria a tarefa.
    let created = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| format!("cliente HTTP: {error}"))?;
        let response = client
            .post(format!("{TWOCAPTCHA_BASE}/in.php"))
            .header("User-Agent", "memu-hub/1.0")
            .form(&[
                ("key", key),
                ("method", "hcaptcha"),
                ("sitekey", sitekey),
                ("pageurl", site_url),
                ("json", "1"),
            ])
            .send()
            .await
            .map_err(|error| format!("POST in.php falhou: {error}"))?;
        let text = response
            .text()
            .await
            .map_err(|error| format!("resposta de in.php ilegível: {error}"))?;
        serde_json::from_str::<serde_json::Value>(&text).map_err(|_| {
            format!(
                "in.php não retornou JSON: {}",
                text.chars().take(120).collect::<String>()
            )
        })
    })?;
    let (ok, task_id) = twocaptcha_reply(&created);
    if !ok {
        let hint = twocaptcha_error_hint(&task_id);
        return Err(format!("2Captcha in.php: {task_id} {hint}").trim().to_string());
    }

    // 2. Poll até o token (5 s, igual à recomendação da doc).
    let deadline = Instant::now() + config.timeout;
    loop {
        if Instant::now() >= deadline {
            return Err(format!(
                "2Captcha não resolveu em {}s (id {task_id})",
                config.timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_secs(5));
        let url = format!(
            "{TWOCAPTCHA_BASE}/res.php?key={key}&action=get&id={task_id}&json=1"
        );
        let value = twocaptcha_get(&url)?;
        let (ok, request) = twocaptcha_reply(&value);
        if ok {
            if request.trim().is_empty() {
                return Err("2Captcha retornou token vazio".into());
            }
            return Ok(SolverResult {
                token: request.trim().to_string(),
                expiration: Some(120.0),
                provider: SolverKind::Twocaptcha.as_str().to_string(),
            });
        }
        if request == "CAPCHA_NOT_READY" {
            continue;
        }
        if twocaptcha_retryable(&request) {
            continue;
        }
        let hint = twocaptcha_error_hint(&request);
        return Err(format!("2Captcha res.php: {request} {hint}").trim().to_string());
    }
}

// -- orquestração -----------------------------------------------------------

/// Tenta os provedores na ordem informada, devolvendo o primeiro token.
pub fn solve_with_priority(
    config: &SolverConfig,
    credentials: &[SolverCredential],
    sitekey: &str,
    site_url: &str,
) -> Result<SolverResult, String> {
    if credentials.is_empty() {
        return Err("Nenhuma API-key cadastrada para resolver o captcha.".into());
    }
    let mut errors = Vec::new();
    for credential in credentials {
        let attempt = match credential.kind {
            SolverKind::Nonecap => solve_nonecap(config, credential, sitekey, site_url),
            SolverKind::Twocaptcha => {
                solve_twocaptcha(config, credential, sitekey, site_url)
            }
            SolverKind::Anticaptcha => {
                solve_anticaptcha(config, credential, sitekey, site_url)
            }
            SolverKind::Onyx => solve_onyx(
                &SolverConfig {
                    proxy: credential.proxy.clone(),
                    ..config.clone()
                },
                credential,
                sitekey,
                site_url,
            ),
        };
        match attempt {
            Ok(result) => return Ok(result),
            Err(error) => errors.push(format!(
                "{} ({}){}: {error}",
                credential.kind.as_str(),
                credential.site_id,
                if credential.label.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", credential.label)
                }
            )),
        }
    }
    Err(format!(
        "Nenhum provedor devolveu token. {}",
        errors.join(" | ")
    ))
}

/// Sites que têm API-key cadastrada mas nenhum solver implementado.
///
/// Ex.: 2Captcha e Surfsky estão no catálogo por serem serviços reais, mas não
/// têm cliente neste app. Sem este aviso o usuário cadastraria a chave, ligaria
/// o switch e nada aconteceria.
pub fn unsupported_sites(
    keys: &[(String, String, String, String)],
    priority: &[String],
) -> Vec<(String, String)> {
    let mut unsupported = Vec::new();
    for site_id in priority {
        if SolverKind::from_site_id(site_id).is_some() {
            continue;
        }
        if let Some((_, label, _, _)) = keys
            .iter()
            .find(|(key_site, _, _, _)| key_site == site_id)
        {
            unsupported.push((site_id.clone(), label.clone()));
        }
    }
    unsupported
}

/// Ordena as credenciais conforme a prioridade de sites do app.
pub fn ordered_credentials(
    keys: &[(String, String, String, String)], // (site_id, label, secret, proxy)
    priority: &[String],
) -> Vec<SolverCredential> {
    let mut ordered = Vec::new();
    for site_id in priority {
        for (key_site, label, secret, proxy) in keys {
            if key_site == site_id {
                if let Some(kind) = SolverKind::from_site_id(site_id) {
                    ordered.push(SolverCredential {
                        kind,
                        site_id: site_id.clone(),
                        label: label.clone(),
                        secret: secret.clone(),
                        proxy: proxy.clone(),
                    });
                }
            }
        }
    }
    ordered
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_only_native_providers() {
        assert_eq!(SolverKind::from_site_id("NoneCap"), Some(SolverKind::Nonecap));
        assert_eq!(SolverKind::from_site_id("2captcha"), Some(SolverKind::Twocaptcha));
        assert_eq!(SolverKind::from_site_id("Twocaptcha"), Some(SolverKind::Twocaptcha));
        assert_eq!(
            SolverKind::from_site_id("Anti-Captcha"),
            Some(SolverKind::Anticaptcha)
        );
        assert_eq!(SolverKind::from_site_id("onyx"), Some(SolverKind::Onyx));
    }

    #[test]
    fn rejects_removed_providers() {
        for removed in ["CapSonic", "sonic", "NoCaptchaAI", "CapSolver", "AZCaptcha"] {
            assert_eq!(
                SolverKind::from_site_id(removed),
                None,
                "{removed} não deveria existir"
            );
        }
    }

    #[test]
    fn exposes_provider_ids() {
        assert_eq!(SolverKind::Nonecap.as_str(), "nonecap");
        assert_eq!(SolverKind::Twocaptcha.as_str(), "2captcha");
        assert_eq!(SolverKind::Anticaptcha.as_str(), "anticaptcha");
        assert_eq!(SolverKind::Onyx.as_str(), "onyx");
    }

    #[test]
    fn orders_credentials_by_priority() {
        let keys = vec![
            ("onyx".to_string(), "A".to_string(), "k1".to_string(), String::new()),
            ("nonecap".to_string(), "B".to_string(), "k2".to_string(), String::new()),
        ];
        let ordered = ordered_credentials(&keys, &["nonecap".to_string(), "onyx".to_string()]);
        assert_eq!(ordered.len(), 2);
        assert_eq!(ordered[0].kind, SolverKind::Nonecap);
        assert_eq!(ordered[1].kind, SolverKind::Onyx);
    }

    #[test]
    fn flags_sites_without_solver() {
        let keys = vec![
            ("2captcha".to_string(), "2Captcha".to_string(), "k1".to_string(), String::new()),
            ("surfsky".to_string(), "Surfsky".to_string(), "k2".to_string(), String::new()),
            ("capsonic".to_string(), "CapSonic".to_string(), "k3".to_string(), String::new()),
        ];
        let priority = vec![
            "2captcha".to_string(),
            "surfsky".to_string(),
            "capsonic".to_string(),
        ];
        let unsupported = unsupported_sites(&keys, &priority);
        assert_eq!(unsupported.len(), 2);
        assert_eq!(unsupported[0].0, "surfsky");
        assert_eq!(unsupported[1].0, "capsonic");
    }

    #[test]
    fn supported_sites_are_not_flagged() {
        let keys = vec![
            ("nonecap".to_string(), "a".to_string(), "k".to_string(), String::new()),
            ("2captcha".to_string(), "b".to_string(), "k".to_string(), String::new()),
            ("anticaptcha".to_string(), "c".to_string(), "k".to_string(), String::new()),
            ("onyx".to_string(), "d".to_string(), "k".to_string(), String::new()),
        ];
        let priority = vec![
            "nonecap".to_string(),
            "2captcha".to_string(),
            "anticaptcha".to_string(),
            "onyx".to_string(),
        ];
        assert!(unsupported_sites(&keys, &priority).is_empty());
    }

    #[test]
    fn empty_credentials_report_clear_error() {
        let error = solve_with_priority(&SolverConfig::default(), &[], "k", "u").unwrap_err();
        assert!(error.contains("Nenhuma API-key"), "{error}");
    }

    #[test]
    fn reads_numeric_request_as_balance() {
        let (ok, request) = twocaptcha_reply(&serde_json::json!({"status":1,"request":0.1}));
        assert!(ok);
        assert_eq!(request, "0.1");
    }

    #[test]
    fn classifies_2captcha_errors() {
        assert!(twocaptcha_retryable("ERROR_NO_SLOT_AVAILABLE"));
        assert!(!twocaptcha_retryable("ERROR_IP_NOT_ALLOWED"));
        assert!(twocaptcha_error_hint("ERROR_IP_NOT_ALLOWED").contains("whitelist"));
        assert!(twocaptcha_error_hint("ERROR_ZERO_BALANCE").contains("saldo"));
    }

    #[test]
    fn classifies_anticaptcha_errors() {
        assert!(anticaptcha_retryable("ERROR_NO_SLOT_AVAILABLE"));
        assert!(!anticaptcha_retryable("ERROR_ZERO_BALANCE"));
        assert!(anticaptcha_error_hint("ERROR_KEY_DOES_NOT_EXIST").contains("API-key"));
        assert!(anticaptcha_error_hint("ERROR_ZERO_BALANCE").contains("sem saldo"));
    }

    #[test]
    fn onyx_explains_proxy_requirement() {
        let hint = onyx_error_hint(
            "ERROR_INVALID_TASK",
            "PopularCaptchaTask requires a proxy (format: user:pass@host:port).",
        );
        assert!(hint.contains("proxy"), "{hint}");
        let hint = onyx_error_hint(
            "",
            "Failed to connect via the supplied proxy. Verify format user:pass@host:port",
        );
        assert!(hint.contains("não conseguiu conectar"), "{hint}");
    }

    #[test]
    fn onyx_requires_proxy_and_key() {
        let credential = SolverCredential {
            kind: SolverKind::Onyx,
            site_id: "onyx".into(),
            label: String::new(),
            secret: "onyx_x".into(),
            proxy: String::new(),
        };
        let error = solve_onyx(
            &SolverConfig::default(),
            &credential,
            "sitekey",
            "https://captcha.muaway.net/",
        )
        .unwrap_err();
        assert!(error.contains("exige proxy"), "{error}");

        let empty = SolverCredential {
            kind: SolverKind::Onyx,
            site_id: "onyx".into(),
            label: String::new(),
            secret: "  ".into(),
            proxy: String::new(),
        };
        let error = solve_onyx(
            &SolverConfig::default(),
            &empty,
            "sitekey",
            "https://captcha.muaway.net/",
        )
        .unwrap_err();
        assert!(error.contains("vazia"), "{error}");
    }

    #[test]
    fn default_config_has_no_proxy() {
        let config = SolverConfig::default();
        assert!(config.proxy.is_empty());
        assert_eq!(config.poll_wait, 60);
    }

    #[test]
    fn providers_require_keys() {
        for (kind, name) in [
            (SolverKind::Nonecap, "NoneCap"),
            (SolverKind::Twocaptcha, "2Captcha"),
            (SolverKind::Anticaptcha, "AntiCaptcha"),
        ] {
            let credential = SolverCredential {
                kind,
                site_id: kind.as_str().into(),
                label: String::new(),
                secret: "  ".into(),
                proxy: String::new(),
            };
            let error = solve_with_priority(
                &SolverConfig::default(),
                &[credential],
                "sitekey",
                "https://captcha.muaway.net/",
            )
            .unwrap_err();
            assert!(
                error.contains("vazia") || error.contains(name),
                "{name}: {error}"
            );
        }
    }
}
