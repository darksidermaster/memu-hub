//! Injeção automática do token no WebView do captcha.
//!
//! O app **não resolve captcha**. Esta thread apenas observa o emulador a cada
//! segundo; quando a página de `captcha.muaway.net` aparece e aceita o token
//! (o bridge `AndroidBridge` já existe), o token — lido de um arquivo JSON ou
//! de texto — é injetado. O arquivo é relido em cada tentativa, então basta
//! salvar um token novo por cima para a próxima captcha usá-lo.
//!
//! O worker é único por instância, com `AtomicBool` para parar. Nada aqui
//! reinicia o aplicativo Android, altera o APK ou lê cookies/credenciais.

use crate::adb;
use crate::cdp;
use crate::solver;
use crate::webview;
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, SystemTime},
};

/// Intervalo entre verificações.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Validade do token do hCaptcha. Cada token expira em 2 minutos, então o
/// pré-resolvido precisa ser usado (ou descartado) dentro dessa janela.
pub const TOKEN_TTL: Duration = Duration::from_secs(120);

/// Margem de segurança antes do TTL: um token com mais idade que isto é
/// descartado para não ser injetado já vencido.
pub const TOKEN_TTL_SAFETY: Duration = Duration::from_secs(10);

/// Etapa atual do ciclo de resolução, exibida no card do emulador.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Observando se há captcha aberto.
    Watching,
    /// Página detectada, verificando o bridge.
    Detecting,
    /// Pedindo o token ao provedor (API).
    Solving,
    /// Token recebido; injetando no WebView.
    Injecting,
    /// Token entregue e confirmado.
    Injected,
    /// Resolvendo antes do captcha aparecer (pré-resolução).
    PreSolving,
    /// Token pré-resolvido pronto, aguardando a janela de injeção.
    Prepared,
    /// Falhou (pass: false ou erro).
    Failed,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Watching => "Aguardando captcha",
            Stage::Detecting => "Página detectada",
            Stage::Solving => "Resolvendo pela API",
            Stage::Injecting => "Injetando token",
            Stage::Injected => "Token injetado",
            Stage::PreSolving => "Pré-resolvendo",
            Stage::Prepared => "Token pronto",
            Stage::Failed => "Falhou",
        }
    }
}

/// Uma tentativa de resolução registrada no log.
#[derive(Clone, Debug, Serialize)]
pub struct AttemptRecord {
    pub at: String,
    pub provider: String,
    pub elapsed_ms: u128,
    pub passed: bool,
    pub token_length: usize,
    pub detail: String,
}

/// Estado observável pela interface.
#[derive(Clone, Debug, Default, Serialize)]
pub struct AutoStatus {
    pub running: bool,
    pub vm_id: Option<u32>,
    pub token_file: String,
    /// Etapa atual do ciclo.
    pub stage: Option<Stage>,
    /// Rótulo legível da etapa.
    pub stage_label: String,
    /// Tentativas de resolução (chamadas ao provedor).
    pub attempts: u64,
    /// Respostas com `pass: true`.
    pub passes: u64,
    /// Respostas com `pass: false` ou erro.
    pub failures: u64,
    /// Injeções confirmadas no WebView.
    pub injections: u64,
    /// Duração da última resolução, em ms.
    pub last_elapsed_ms: Option<u128>,
    pub last_result: Option<String>,
    pub last_error: Option<String>,
    /// Últimas tentativas (mais recentes primeiro), limitadas a 20.
    pub history: Vec<AttemptRecord>,
    /// Pré-resolução ativa nesta instância.
    pub pre_solve: bool,
    /// Existe um token pré-resolvido válido aguardando injeção.
    pub prepared: bool,
    /// Segundos restantes até o token pré-resolvido expirar.
    pub prepared_age_s: Option<f64>,
    /// Intervalo médio entre captchas (segundos), quando já há amostras.
    pub interval_mean_s: Option<f64>,
    /// Tempo médio de resolução (segundos).
    pub solve_mean_s: Option<f64>,
    /// Quantas amostras de intervalo embasam a média.
    pub timing_samples: usize,
}

impl AutoStatus {
    fn reset(&mut self, vm_id: u32, token_file: &str) {
        self.running = true;
        self.vm_id = Some(vm_id);
        self.token_file = token_file.to_string();
        self.attempts = 0;
        self.passes = 0;
        self.failures = 0;
        self.injections = 0;
        self.last_elapsed_ms = None;
        self.last_result = None;
        self.last_error = None;
        self.history.clear();
        self.pre_solve = false;
        self.prepared = false;
        self.prepared_age_s = None;
        self.interval_mean_s = None;
        self.solve_mean_s = None;
        self.timing_samples = 0;
        self.set_stage(Stage::Watching);
    }

    fn set_stage(&mut self, stage: Stage) {
        self.stage = Some(stage);
        self.stage_label = stage.label().to_string();
    }

    fn record(&mut self, record: AttemptRecord) {
        if record.passed {
            self.passes = self.passes.saturating_add(1);
        } else {
            self.failures = self.failures.saturating_add(1);
        }
        self.last_elapsed_ms = Some(record.elapsed_ms);
        self.history.insert(0, record);
        self.history.truncate(20);
    }
}

/// Caminho do log de tentativas (`%LOCALAPPDATA%\MEmuHub\captcha-attempts.jsonl`).
pub fn attempts_log_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("MEmuHub").join("captcha-attempts.jsonl")
}

/// Acrescenta uma tentativa ao log em disco. Falhas de escrita não interrompem
/// o worker: o estado em memória continua válido.
fn append_attempt_log(record: &AttemptRecord, vm_id: u32) {
    let path = attempts_log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let line = serde_json::json!({
        "at": record.at,
        "vm": vm_id,
        "provider": record.provider,
        "elapsed_ms": record.elapsed_ms,
        "passed": record.passed,
        "token_length": record.token_length,
        "detail": record.detail,
    });
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(file, "{line}");
    }
}

/// Timestamp ISO-8601 em UTC, sem dependências externas.
fn now_iso8601() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let days = secs / 86_400;
    let mut remainder = secs % 86_400;
    let hours = remainder / 3_600;
    remainder %= 3_600;
    let minutes = remainder / 60;
    let seconds = remainder % 60;
    // Conversão civil de Howard Hinnant (epoch 1970-01-01).
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!(
        "{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z"
    )
}

/// Estatísticas de tempo usadas pela pré-resolução.
///
/// Guarda as durações de resolução e os instantes em que cada captcha foi
/// **detectado**. Com isso calcula:
///
/// * `solve_mean`: quanto tempo o provedor leva para devolver um token;
/// * `interval_mean`: intervalo médio entre captchas consecutivos.
///
/// O momento de começar a pré-resolver é
/// `proxima_chegada - solve_mean - margem`. A margem absorve a variação:
/// usa-se o desvio-padrão amostral das resoluções, limitado a 50% da média,
/// porque resoluções de hCaptcha variam bastante (10 s a 130 s na prática).
#[derive(Clone, Debug, Default)]
pub struct Timing {
    solves: Vec<f64>,
    detections: Vec<f64>,
    last_detection: Option<std::time::Instant>,
    /// Intervalo médio informado manualmente (Configurações). Usado somente
    /// enquanto não houver amostras reais; o cálculo real sempre prevalece.
    manual_interval: Option<f64>,
    /// Tempo de resolução informado manualmente. Mesma regra do intervalo.
    manual_solve: Option<f64>,
}

/// Quantas amostras no máximo guardar (janela móvel).
const TIMING_WINDOW: usize = 20;
/// Amostras mínimas para habilitar a pré-resolução.
pub const TIMING_MIN_SAMPLES: usize = 2;

impl Timing {
    /// Cria o `Timing` já com as médias manuais (quando configuradas).
    pub fn with_manual(interval: Option<f64>, solve: Option<f64>) -> Self {
        Self {
            manual_interval: interval.filter(|value| *value > 0.0),
            manual_solve: solve.filter(|value| *value > 0.0),
            ..Self::default()
        }
    }

    fn record_solve(&mut self, seconds: f64) {
        if seconds <= 0.0 {
            return;
        }
        self.solves.push(seconds);
        if self.solves.len() > TIMING_WINDOW {
            self.solves.remove(0);
        }
    }

    /// Registra a detecção de uma captcha e, se havia uma anterior, o intervalo.
    fn record_detection(&mut self, now: std::time::Instant) {
        if let Some(previous) = self.last_detection {
            let gap = now.saturating_duration_since(previous).as_secs_f64();
            // Ignora intervalos irreais (mesma página vista duas vezes).
            if gap >= 1.0 {
                self.detections.push(gap);
                if self.detections.len() > TIMING_WINDOW {
                    self.detections.remove(0);
                }
            }
        }
        self.last_detection = Some(now);
    }

    fn mean(values: &[f64]) -> Option<f64> {
        if values.is_empty() {
            return None;
        }
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }

    /// Desvio-padrão amostral (n-1). Precisa de 2+ amostras.
    fn stddev(values: &[f64]) -> Option<f64> {
        if values.len() < 2 {
            return None;
        }
        let mean = Self::mean(values)?;
        let variance = values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / (values.len() - 1) as f64;
        Some(variance.sqrt())
    }

    /// Média real de resolução; cai no valor manual enquanto não houver amostra.
    pub fn solve_mean(&self) -> Option<f64> {
        Self::mean(&self.solves).or(self.manual_solve)
    }

    /// Média real de intervalo; cai no valor manual enquanto não houver amostra.
    pub fn interval_mean(&self) -> Option<f64> {
        Self::mean(&self.detections).or(self.manual_interval)
    }

    /// Margem de segurança: desvio-padrão das resoluções, limitado a 50% da
    /// média para não antecipar demais quando há um outlier.
    pub fn margin(&self) -> f64 {
        let Some(mean) = self.solve_mean() else {
            return 0.0;
        };
        let spread = Self::stddev(&self.solves).unwrap_or(mean * 0.25);
        spread.min(mean * 0.5).max(1.0)
    }

    /// Pré-resolução está apta quando há média de resolução e de intervalo.
    /// `true` quando dá para pré-resolver: amostras reais suficientes **ou**
    /// valores manuais configurados.
    pub fn ready(&self) -> bool {
        let real = self.solves.len() >= TIMING_MIN_SAMPLES
            && self.detections.len() >= TIMING_MIN_SAMPLES;
        let manual = self.manual_solve.is_some() && self.manual_interval.is_some();
        (real || manual) && self.solve_mean().is_some() && self.interval_mean().is_some()
    }

    /// Quantos segundos antes da próxima chegada começar a resolver.
    pub fn lead_seconds(&self) -> Option<f64> {
        if !self.ready() {
            return None;
        }
        let solve = self.solve_mean()?;
        // max(0): se a resolução demora mais que o intervalo, começa imediato.
        Some((solve + self.margin()).max(0.0))
    }

    /// Deve começar a resolver agora?
    ///
    /// `since_last` é o tempo desde a última captcha detectada.
    pub fn should_pre_solve(&self, since_last: Duration) -> bool {
        let (Some(interval), Some(lead)) = (self.interval_mean(), self.lead_seconds()) else {
            return false;
        };
        let trigger = (interval - lead).max(0.0);
        since_last.as_secs_f64() >= trigger
    }

    fn sample_count(&self) -> usize {
        self.solves
            .len()
            .min(self.detections.len())
            .max(if self.ready() { 1 } else { 0 })
    }
}

/// Token resolvido antes da janela abrir.
#[derive(Clone, Debug)]
pub struct PreparedToken {
    pub token: webview::Token,
    pub created: std::time::Instant,
}

impl PreparedToken {
    pub fn age(&self) -> Duration {
        self.created.elapsed()
    }

    /// Válido para injeção: dentro do TTL com margem.
    pub fn is_valid(&self) -> bool {
        self.age() + TOKEN_TTL_SAFETY < TOKEN_TTL
    }

    /// Segundos até o token deixar de ser aceitável para injeção.
    pub fn seconds_left(&self) -> f64 {
        (TOKEN_TTL.as_secs_f64() - TOKEN_TTL_SAFETY.as_secs_f64())
            - self.age().as_secs_f64()
    }
}

/// De onde vem o token usado pelo worker automático.
#[derive(Clone, Debug)]
pub enum TokenSource {
    /// Provedores de captcha, tentados na ordem de prioridade.
    Solvers {
        config: solver::SolverConfig,
        credentials: Vec<solver::SolverCredential>,
    },
}

impl TokenSource {
    /// Rótulo do provedor para o log.
    pub fn provider_label(&self) -> String {
        match self {
            TokenSource::Solvers { credentials, .. } => credentials
                .iter()
                .map(|credential| credential.kind.as_str())
                .collect::<Vec<_>>()
                .join("+"),
        }
    }

    /// Obtém um token válido. Para arquivo, recusa se `expiration` venceu em
    /// relação ao mtime; para solvers, chama o primeiro provedor que responder.
    pub fn acquire(&self, sitekey: &str, site_url: &str) -> Result<webview::Token, String> {
        match self {
            TokenSource::Solvers {
                config,
                credentials,
            } => {
                let result =
                    solver::solve_with_priority(config, credentials, sitekey, site_url)?;
                Ok(webview::Token {
                    value: result.token,
                })
            }
        }
    }
}

/// Lê um parâmetro de query de uma URL (`?sitekey=...&lang=`).
pub fn url_param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1.split('#').next()?;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        if key == name && !value.is_empty() {
            return Some(percent_decode(value));
        }
    }
    None
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(high * 16 + low);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn try_inject(
    runtime: &tokio::runtime::Runtime,
    adb: &adb::Adb,
    serial: &str,
    token_source: &TokenSource,
    package: &str,
    enable_debug: bool,
    last_used: &mut Option<String>,
    prepared: &mut Option<PreparedToken>,
    timing: &mut Timing,
    status: &Arc<Mutex<AutoStatus>>,
    vm_id: u32,
) -> Result<Option<String>, String> {
    let pid = runtime
        .block_on(adb::app_pid(adb, serial, package))
        .ok_or_else(|| format!("{package} não está em execução"))?;
    let socket = match runtime.block_on(adb::devtools_socket(adb, serial, pid)) {
        Some(socket) => socket,
        None => {
            if !enable_debug {
                return Err(
                    "WebView sem socket DevTools: marque 'Habilitar depuração via Frida' nas Configurações."
                        .into(),
                );
            }
            match runtime.block_on(webview::ensure_devtools_socket(
                adb, serial, pid, package, true,
            )) {
                Ok((_pid, socket)) => socket,
                Err(error) => return Err(error),
            }
        }
    };

    let (port, spec) = runtime.block_on(webview::open_forward(adb, serial, &socket))?;
    let outcome = (|| -> Result<Option<String>, String> {
        let text = cdp::http_get(port, "/json/list", Duration::from_secs(3))?;
        let targets = cdp::parse_targets(&text)?;
        let pages = cdp::captcha_targets(&targets, webview::CAPTCHA_HOST);
        if pages.is_empty() {
            return Ok(None);
        }
        if let Ok(mut guard) = status.lock() {
            guard.set_stage(Stage::Detecting);
        }

        // Fechar a janela no jogo não remove o target nem muda visibilityState;
        // o campo de resposta preenchido é o sinal de "já resolvido".
        let target = &pages[0];
        let state = cdp::page_state(port, target, Duration::from_secs(3))?;
        if !cdp::should_solve(&state) {
            return Ok(None);
        }

        // Captcha nova: alimenta a média de intervalo entre captchas.
        timing.record_detection(std::time::Instant::now());

        let sitekey = url_param(&pages[0].url, "sitekey")
            .unwrap_or_else(|| webview::SITEKEY.to_string());

        let solving_started = std::time::Instant::now();

        // Usa o token pré-resolvido, se ainda estiver dentro do TTL.
        let prepared_token = match prepared.take() {
            Some(candidate) if candidate.is_valid() => {
                if let Ok(mut guard) = status.lock() {
                    guard.prepared = false;
                    guard.prepared_age_s = None;
                }
                Some(candidate.token)
            }
            Some(expired) => {
                if let Ok(mut guard) = status.lock() {
                    guard.prepared = false;
                    guard.prepared_age_s = None;
                    guard.last_result = Some(format!(
                        "token pré-resolvido descartado após {:.0}s",
                        expired.age().as_secs_f64()
                    ));
                }
                None
            }
            None => None,
        };

        let (token, elapsed_ms) = match prepared_token {
            Some(token) => {
                // Injeção com token pronto não consome crédito nem entra na
                // média de resolução.
                let started = std::time::Instant::now();
                (token, started.elapsed().as_millis())
            }
            None => {
                if let Ok(mut guard) = status.lock() {
                    guard.set_stage(Stage::Solving);
                    guard.attempts = guard.attempts.saturating_add(1);
                }
                let acquired = token_source.acquire(&sitekey, webview::SITE_URL);
                let elapsed_ms = solving_started.elapsed().as_millis();
                match acquired {
                    Ok(token) => {
                        timing.record_solve(elapsed_ms as f64 / 1000.0);
                        let record = AttemptRecord {
                            at: now_iso8601(),
                            provider: token_source.provider_label(),
                            elapsed_ms,
                            passed: true,
                            token_length: token.value.len(),
                            detail: "pass: true".to_string(),
                        };
                        if let Ok(mut guard) = status.lock() {
                            guard.record(record.clone());
                        }
                        append_attempt_log(&record, vm_id);
                        (token, elapsed_ms)
                    }
                    Err(error) => {
                        let record = AttemptRecord {
                            at: now_iso8601(),
                            provider: token_source.provider_label(),
                            elapsed_ms,
                            passed: false,
                            token_length: 0,
                            detail: format!("pass: false — {error}"),
                        };
                        if let Ok(mut guard) = status.lock() {
                            guard.record(record.clone());
                        }
                        append_attempt_log(&record, vm_id);
                        return Err(error);
                    }
                }
            }
        };

        if last_used.as_deref() == Some(token.value.as_str()) {
            return Ok(None);
        }
        if let Ok(mut guard) = status.lock() {
            guard.set_stage(Stage::Injecting);
        }
        for target in pages {
            match cdp::inject(port, &target, &token.value, true, Duration::from_secs(3)) {
                Ok(result) if result.injected => {
                    *last_used = Some(token.value.clone());
                    return Ok(Some(format!(
                        "token de {} chars injetado em '{}' ({} campo(s), {} ms)",
                        token.value.len(),
                        target.title,
                        result.fields_filled.len(),
                        elapsed_ms
                    )));
                }
                Ok(result) if result.bridge_present => {
                    return Err(result.error.unwrap_or_else(|| "o bridge recusou o token".into()))
                }
                _ => continue,
            }
        }
        Ok(None)
    })();
    runtime.block_on(webview::remove_forward(adb, serial, &spec));
    outcome
}

/// Resolve o captcha antes de a janela abrir e guarda o token.
///
/// A decisão de *quando* é tomada pelo chamador ([`Timing::should_pre_solve`]);
/// aqui só se garante que não há outro token pronto e que a página não está
/// aberta (nesse caso o fluxo normal resolve na hora).
#[allow(clippy::too_many_arguments)]
fn pre_solve(
    runtime: &tokio::runtime::Runtime,
    adb: &adb::Adb,
    serial: &str,
    token_source: &TokenSource,
    package: &str,
    enable_debug: bool,
    timing: &mut Timing,
    status: &Arc<Mutex<AutoStatus>>,
    vm_id: u32,
) -> Result<Option<PreparedToken>, String> {
    let pid = runtime
        .block_on(adb::app_pid(adb, serial, package))
        .ok_or_else(|| format!("{package} não está em execução"))?;
    let socket = match runtime.block_on(adb::devtools_socket(adb, serial, pid)) {
        Some(socket) => socket,
        None => {
            if !enable_debug {
                return Err(
                    "WebView sem socket DevTools: marque 'Habilitar depuração via Frida' nas Configurações."
                        .into(),
                );
            }
            match runtime.block_on(webview::ensure_devtools_socket(
                adb, serial, pid, package, true,
            )) {
                Ok((_pid, socket)) => socket,
                Err(error) => return Err(error),
            }
        }
    };

    // Se a janela já está aberta, não adianta pré-resolver: o ciclo normal
    // resolve e injeta. Devolve `None` para o chamador seguir o fluxo.
    let (port, spec) = runtime.block_on(webview::open_forward(adb, serial, &socket))?;
    let open_now = (|| -> Result<bool, String> {
        let text = cdp::http_get(port, "/json/list", Duration::from_secs(3))?;
        let targets = cdp::parse_targets(&text)?;
        let pages = cdp::captcha_targets(&targets, webview::CAPTCHA_HOST);
        Ok(!pages.is_empty())
    })();
    runtime.block_on(webview::remove_forward(adb, serial, &spec));
    let open_now = open_now?;

    let sitekey = webview::SITEKEY.to_string();
    let solving_started = std::time::Instant::now();
    if let Ok(mut guard) = status.lock() {
        guard.set_stage(Stage::PreSolving);
        guard.attempts = guard.attempts.saturating_add(1);
    }
    let acquired = token_source.acquire(&sitekey, webview::SITE_URL);
    let elapsed_ms = solving_started.elapsed().as_millis();

    match acquired {
        Ok(token) => {
            timing.record_solve(elapsed_ms as f64 / 1000.0);
            let record = AttemptRecord {
                at: now_iso8601(),
                provider: token_source.provider_label(),
                elapsed_ms,
                passed: true,
                token_length: token.value.len(),
                detail: "pass: true (pré-resolução)".to_string(),
            };
            if let Ok(mut guard) = status.lock() {
                guard.record(record.clone());
                guard.set_stage(Stage::Prepared);
                guard.last_result = Some(format!(
                    "token pré-resolvido em {:.0}s{}",
                    elapsed_ms as f64 / 1000.0,
                    if open_now { " (janela já aberta)" } else { "" }
                ));
                guard.last_error = None;
            }
            append_attempt_log(&record, vm_id);
            Ok(Some(PreparedToken {
                token,
                created: std::time::Instant::now(),
            }))
        }
        Err(error) => {
            let record = AttemptRecord {
                at: now_iso8601(),
                provider: token_source.provider_label(),
                elapsed_ms,
                passed: false,
                token_length: 0,
                detail: format!("pass: false — {error} (pré-resolução)"),
            };
            if let Ok(mut guard) = status.lock() {
                guard.record(record.clone());
            }
            append_attempt_log(&record, vm_id);
            Err(error)
        }
    }
}

/// Worker controlável: um por instância ativa.
pub struct AutoInjector {
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<AutoStatus>>,
    pre_solve: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl AutoInjector {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        vm_id: u32,
        token_source: TokenSource,
        adb_path: String,
        package: String,
        enable_debug: bool,
        pre_solve_enabled: bool,
        manual_interval: Option<f64>,
        manual_solve: Option<f64>,
    ) -> Result<Self, String> {
        if pre_solve_enabled {
            // Sem histórico de tempo não há como prever quando a janela abre.
            return Err(
                "A pré-resolução precisa de um tempo médio definido: deixe o switch de injeção rodar algumas captchas antes."
                    .into(),
            );
        }
        // Valida a origem agora para não iniciar um worker inútil.
        let label = match &token_source {
            TokenSource::Solvers { credentials, .. } => {
                if credentials.is_empty() {
                    return Err(
                        "Cadastre uma API-key de um site suportado para resolver o captcha.".into(),
                    );
                }
                format!("solvers:{}", credentials.len())
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(AutoStatus::default()));
        {
            let mut guard = status.lock().map_err(|_| "estado indisponível".to_string())?;
            guard.reset(vm_id, &label);
            guard.pre_solve = pre_solve_enabled;
        }
        let thread_stop = Arc::clone(&stop);
        let thread_status = Arc::clone(&status);
        let thread_pre_solve = Arc::new(AtomicBool::new(false));
        let pre_solve_flag = Arc::clone(&thread_pre_solve);
        let pre_solve_for_thread = Arc::clone(&thread_pre_solve);
        let serial = webview::serial_for_vm(vm_id);
        let handle = thread::Builder::new()
            .name(format!("auto-inject-{vm_id}"))
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        if let Ok(mut guard) = thread_status.lock() {
                            guard.running = false;
                            guard.last_error = Some(format!("runtime: {error}"));
                        }
                        return;
                    }
                };
                let adb = match adb::locate_adb(&adb_path) {
                    Ok(path) => adb::Adb::new(path),
                    Err(error) => {
                        if let Ok(mut guard) = thread_status.lock() {
                            guard.running = false;
                            guard.last_error = Some(error);
                        }
                        return;
                    }
                };
                let mut last_message = String::new();
                let mut last_used: Option<String> = None;
                let mut prepared: Option<PreparedToken> = None;
                let mut timing = Timing::with_manual(manual_interval, manual_solve);
                let mut pre_solve_attempted = false;
                while !thread_stop.load(Ordering::Relaxed) {
                    // Pré-resolução: dispara quando o tempo desde a última
                    // captcha se aproxima da chegada prevista menos o tempo de
                    // resolução (com margem). Só tenta uma vez por ciclo de
                    // captcha, e nunca com um token já pronto.
                    if pre_solve_for_thread.load(Ordering::Relaxed)
                        && prepared.is_none()
                        && !pre_solve_attempted
                    {
                        let since_last = timing
                            .last_detection
                            .map(|moment| moment.elapsed())
                            .unwrap_or(Duration::ZERO);
                        if timing.ready() && timing.should_pre_solve(since_last) {
                            pre_solve_attempted = true;
                            match pre_solve(
                                &runtime,
                                &adb,
                                &serial,
                                &token_source,
                                &package,
                                enable_debug,
                                &mut timing,
                                &thread_status,
                                vm_id,
                            ) {
                                Ok(token) => prepared = token,
                                Err(error) => {
                                    if let Ok(mut guard) = thread_status.lock() {
                                        guard.set_stage(Stage::Failed);
                                        guard.last_error = Some(error);
                                    }
                                }
                            }
                            if let Ok(mut guard) = thread_status.lock() {
                                guard.prepared = prepared.is_some();
                                guard.prepared_age_s = prepared
                                    .as_ref()
                                    .map(|token| token.seconds_left().max(0.0));
                                guard.interval_mean_s = timing.interval_mean();
                                guard.solve_mean_s = timing.solve_mean();
                                guard.timing_samples = timing.sample_count();
                            }
                        }
                    }
                    // Token preparado vencido é descartado pelo próprio try_inject.
                    if prepared.is_some() && !prepared.as_ref().is_some_and(|t| t.is_valid()) {
                        prepared = None;
                        pre_solve_attempted = false;
                    }
                    let outcome = try_inject(
                        &runtime,
                        &adb,
                        &serial,
                        &token_source,
                        &package,
                        enable_debug,
                        &mut last_used,
                        &mut prepared,
                        &mut timing,
                        &thread_status,
                        vm_id,
                    );
                    if let Ok(mut guard) = thread_status.lock() {
                        match outcome {
                            Ok(Some(message)) => {
                                if message != last_message {
                                    guard.injections = guard.injections.saturating_add(1);
                                    last_message = message.clone();
                                }
                                guard.set_stage(Stage::Injected);
                                guard.last_result = Some(message);
                                guard.last_error = None;
                                guard.prepared = false;
                                guard.prepared_age_s = None;
                                guard.interval_mean_s = timing.interval_mean();
                                guard.solve_mean_s = timing.solve_mean();
                                guard.timing_samples = timing.sample_count();
                                pre_solve_attempted = false;
                            }
                            Ok(None) => {
                                guard.set_stage(Stage::Watching);
                                guard.last_error = None;
                            }
                            Err(error) => {
                                guard.set_stage(Stage::Failed);
                                guard.last_error = Some(error);
                            }
                        }
                    }
                    // Espera em fatias para desligar rápido.
                    let mut waited = Duration::ZERO;
                    while waited < POLL_INTERVAL && !thread_stop.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(100));
                        waited += Duration::from_millis(100);
                    }
                }
                if let Ok(mut guard) = thread_status.lock() {
                    guard.running = false;
                }
            })
            .map_err(|error| format!("não foi possível iniciar a verificação automática: {error}"))?;
        Ok(Self {
            stop,
            status,
            pre_solve: pre_solve_flag,
            handle: Some(handle),
        })
    }

    /// Liga/desliga a pré-resolução com o worker em execução.
    ///
    /// Exige tempo médio já calculado — sem histórico não há como prever quando
    /// a janela abre, então ativar seria só gastar crédito.
    pub fn set_pre_solve(&self, enabled: bool) -> Result<(), String> {
        if enabled {
            let status = self.status();
            if status.interval_mean_s.is_none() || status.solve_mean_s.is_none() {
                return Err(
                    "A pré-resolução exige um tempo médio: informe os valores nas Configurações ou deixe a injeção rodar algumas captchas."
                        .into(),
                );
            }
        }
        self.pre_solve.store(enabled, Ordering::Relaxed);
        if let Ok(mut guard) = self.status.lock() {
            guard.pre_solve = enabled;
            if !enabled {
                guard.prepared = false;
                guard.prepared_age_s = None;
            }
        }
        Ok(())
    }

    pub fn status(&self) -> AutoStatus {
        self.status.lock().map(|guard| guard.clone()).unwrap_or_default()
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        if let Ok(mut guard) = self.status.lock() {
            guard.running = false;
        }
    }
}

impl Drop for AutoInjector {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing_with(solves: &[f64], gaps: &[f64]) -> Timing {
        let mut timing = Timing::default();
        for value in solves {
            timing.record_solve(*value);
        }
        let base = std::time::Instant::now();
        timing.last_detection = Some(base);
        timing.detections = gaps.to_vec();
        timing
    }

    #[test]
    fn timing_not_ready_without_samples() {
        let timing = Timing::default();
        assert!(!timing.ready());
        assert_eq!(timing.lead_seconds(), None);
        assert!(!timing.should_pre_solve(Duration::from_secs(3600)));
    }

    #[test]
    fn timing_not_ready_with_only_one_side() {
        // Só tem tempos de resolução, sem intervalos.
        let timing = timing_with(&[10.0, 12.0], &[]);
        assert!(!timing.ready());
        // Só tem intervalos, sem resolução.
        let only_gaps = Timing {
            detections: vec![60.0, 60.0],
            ..Timing::default()
        };
        assert!(!only_gaps.ready());
    }

    #[test]
    fn timing_computes_means() {
        let timing = timing_with(&[10.0, 20.0], &[60.0, 80.0]);
        assert!(timing.ready());
        assert_eq!(timing.solve_mean(), Some(15.0));
        assert_eq!(timing.interval_mean(), Some(70.0));
    }

    #[test]
    fn lead_is_solve_plus_bounded_margin() {
        // Resoluções estáveis: margem pequena, mas nunca abaixo de 1 s.
        let timing = timing_with(&[10.0, 10.0, 10.0], &[60.0, 60.0]);
        let lead = timing.lead_seconds().unwrap();
        assert!(lead >= 11.0, "lead={lead}");
        assert!(lead <= 15.0, "lead={lead}");
    }

    #[test]
    fn margin_caps_outlier_influence() {
        // Um outlier grande não pode antecipar demais: margem <= 50% da média.
        let timing = timing_with(&[10.0, 10.0, 200.0], &[60.0, 60.0]);
        let mean = timing.solve_mean().unwrap();
        let margin = timing.margin();
        assert!(margin <= mean * 0.5 + 1e-6, "margin={margin} mean={mean}");
    }

    #[test]
    fn pre_solve_triggers_near_expected_arrival() {
        // Intervalo 60 s, resolução ~10 s: deve começar por volta de 60-lead.
        let timing = timing_with(&[10.0, 10.0], &[60.0, 60.0]);
        let lead = timing.lead_seconds().unwrap();
        let trigger = 60.0 - lead;
        assert!(!timing.should_pre_solve(Duration::from_secs_f64(trigger - 5.0)));
        assert!(timing.should_pre_solve(Duration::from_secs_f64(trigger + 1.0)));
    }

    #[test]
    fn pre_solve_starts_immediately_when_solve_exceeds_interval() {
        // Resolução mais longa que o intervalo: começa já.
        let timing = timing_with(&[90.0, 95.0], &[30.0, 32.0]);
        assert!(timing.should_pre_solve(Duration::ZERO));
    }

    #[test]
    fn timing_window_is_bounded() {
        let mut timing = Timing::default();
        for index in 0..40 {
            timing.record_solve(index as f64 + 1.0);
        }
        assert!(timing.solves.len() <= TIMING_WINDOW);
    }

    #[test]
    fn ignores_duplicate_detections() {
        let mut timing = Timing::default();
        let now = std::time::Instant::now();
        timing.record_detection(now);
        // Mesma página vista de novo em menos de 1 s não conta como intervalo.
        timing.record_detection(now);
        assert!(timing.detections.is_empty());
    }

    #[test]
    fn prepared_token_respects_ttl() {
        let token = webview::Token { value: "P1_x".into() };
        let fresh = PreparedToken { token: token.clone(), created: std::time::Instant::now() };
        assert!(fresh.is_valid());
        // Simula 115 s de idade: fora do TTL com margem.
        let old = PreparedToken {
            token,
            created: std::time::Instant::now() - Duration::from_secs(115),
        };
        assert!(!old.is_valid());
    }

    #[test]
    fn extracts_sitekey_from_url() {
        let url = "https://captcha.muaway.net/android.html?sitekey=ef8c43cb-a59d-461f-a710-239b13569c4d&lang=";
        assert_eq!(
            url_param(url, "sitekey").as_deref(),
            Some("ef8c43cb-a59d-461f-a710-239b13569c4d")
        );
        assert_eq!(url_param(url, "lang"), None);
        assert_eq!(url_param(url, "missing"), None);
        assert_eq!(url_param("https://x/y", "sitekey"), None);
        assert_eq!(url_param("https://x/y?sitekey=a%2Db", "sitekey").as_deref(), Some("a-b"));
    }

    #[test]
    fn prefers_page_sitekey_over_fallback() {
        // O jogo já usou 06bbf0e3 (full_solve.js) e hoje usa ef8c43cb na URL.
        // A URL aberta sempre vence; o fallback é só emergência.
        let url = "https://captcha.muaway.net/android.html?sitekey=ef8c43cb-a59d-461f-a710-239b13569c4d&lang=en";
        let chosen = url_param(url, "sitekey").unwrap_or_else(|| webview::SITEKEY.to_string());
        assert_eq!(chosen, "ef8c43cb-a59d-461f-a710-239b13569c4d");
        assert_ne!(chosen, "06bbf0e3-4583-44df-ad7e-61c028c50e38");
    }

    #[test]
    fn falls_back_to_default_sitekey_when_url_has_none() {
        let url = "https://captcha.muaway.net/android.html";
        let chosen = url_param(url, "sitekey").unwrap_or_else(|| webview::SITEKEY.to_string());
        assert_eq!(chosen, webview::SITEKEY);
        assert_eq!(chosen, "ef8c43cb-a59d-461f-a710-239b13569c4d");
    }

    #[test]
    fn status_starts_stopped() {
        let status = AutoStatus::default();
        assert!(!status.running);
        assert_eq!(status.injections, 0);
    }
}

#[cfg(test)]
mod manual_timing_tests {
    use super::*;

    #[test]
    fn manual_values_enable_pre_solve() {
        // Sem nenhuma amostra, mas com valores definidos: libera a pré-resolução.
        let timing = Timing::with_manual(Some(300.0), Some(40.0));
        assert!(timing.ready());
        assert_eq!(timing.interval_mean(), Some(300.0));
        assert_eq!(timing.solve_mean(), Some(40.0));
        assert!(timing.solves.is_empty() && timing.detections.is_empty());
    }

    #[test]
    fn real_samples_override_manual_values() {
        let mut timing = Timing::with_manual(Some(300.0), Some(40.0));
        timing.record_solve(10.0);
        timing.record_solve(20.0);
        let base = std::time::Instant::now();
        timing.last_detection = Some(base);
        timing.detections = vec![60.0, 80.0];
        // Médias reais assumem; o manual deixa de ser usado.
        assert_eq!(timing.solve_mean(), Some(15.0));
        assert_eq!(timing.interval_mean(), Some(70.0));
        assert!(!timing.solves.is_empty() && !timing.detections.is_empty());
    }

    #[test]
    fn zero_or_missing_manual_keeps_timing_disabled() {
        for (interval, solve) in [(None, None), (Some(0.0), Some(0.0)), (Some(300.0), None)] {
            let timing = Timing::with_manual(interval, solve);
            assert!(!timing.ready(), "{interval:?}/{solve:?} não deveria liberar");
        }
    }

    #[test]
    fn manual_lead_uses_solve_plus_margin() {
        let timing = Timing::with_manual(Some(300.0), Some(40.0));
        let lead = timing.lead_seconds().unwrap();
        // Sem amostras, margem = 25% da média (10s) => 50s.
        assert!((49.0..=51.0).contains(&lead), "lead={lead}");
    }
}
