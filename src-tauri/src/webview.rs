//! Orquestração: PID, socket DevTools, forward, alvos e injeção.
//!
//! Fluxo de uma sessão, sempre sem PID fixo:
//!   1. `pidof <pacote>` no emulador;
//!   2. `su -c cat /proc/net/unix` → `webview_devtools_remote_<pid>`;
//!   3. (opcional) habilita a depuração via Frida quando o socket não existe;
//!   4. `adb forward tcp:<porta livre> localabstract:<socket>`;
//!   5. `/json/list` → alvos do tipo `page` em `captcha.muaway.net`;
//!   6. `Runtime.evaluate` chamando `AndroidBridge.handleCallback(token)`.
//!
//! Nada aqui reinicia o aplicativo, altera o APK ou lê cookies/credenciais.

use crate::adb::{self, Adb};
use crate::cdp::{self, Target};
use crate::memu::RunningVm;
use serde::Serialize;
use std::{
    net::TcpListener,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const CAPTCHA_HOST: &str = "captcha.muaway.net";
pub const DEFAULT_PACKAGE: &str = "com.away.game.br";
/// Sitekey padrão do hCaptcha de `captcha.muaway.net`.
///
/// O jogo usa mais de um sitekey (`06bbf0e3-...` no `full_solve.js` e
/// `ef8c43cb-...` na URL `android.html`). O worker sempre prefere o sitekey
/// presente na URL da página aberta; este valor é só o fallback.
pub const SITEKEY: &str = "ef8c43cb-a59d-461f-a710-239b13569c4d";
/// URL usada na resolução.
pub const SITE_URL: &str = "https://captcha.muaway.net/";

/// Token pronto para injeção.
#[derive(Clone, Debug)]
pub struct Token {
    pub value: String,
}

/// Uma página aberta no WebView.
#[derive(Clone, Debug, Serialize)]
pub struct PageInfo {
    pub id: String,
    pub title: String,
    pub url: String,
    pub captcha: bool,
}

/// Estado de um emulador para a interface.
#[derive(Clone, Debug, Serialize)]
pub struct SessionReport {
    pub vm_id: Option<u32>,
    pub vm_name: String,
    pub serial: String,
    pub model: String,
    pub app_running: bool,
    pub app_pid: Option<u32>,
    pub socket: Option<String>,
    pub debug_enabled: bool,
    pub pages: Vec<PageInfo>,
    pub error: Option<String>,
}

/// Porta TCP livre em 127.0.0.1 (o SO escolhe).
pub fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("não foi possível reservar uma porta local: {e}"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| e.to_string())
}

/// Infere o índice da VM a partir do serial ADB (`21503 + id * 10`).
pub fn infer_vm_id(serial: &str) -> Option<u32> {
    let port: u16 = serial.rsplit(':').next()?.parse().ok()?;
    if port >= 21_513 && (port - 21_503).is_multiple_of(10) {
        Some(((port - 21_503) / 10) as u32)
    } else {
        None
    }
}

/// Serial ADB esperado para um índice de VM.
pub fn serial_for_vm(vm_id: u32) -> String {
    format!("127.0.0.1:{}", 21_503 + vm_id * 10)
}

pub async fn remove_forward(adb: &Adb, serial: &str, spec: &str) {
    let _ = adb
        .raw_serial(serial, &["forward", "--remove", spec])
        .await;
}

pub async fn open_forward(adb: &Adb, serial: &str, socket: &str) -> Result<(u16, String), String> {
    let port = free_port()?;
    let spec = format!("tcp:{port}");
    adb.raw_serial(
        serial,
        &["forward", &spec, &format!("localabstract:{socket}")],
    )
    .await?;
    Ok((port, spec))
}

fn fetch_targets(port: u16) -> Result<Vec<Target>, String> {
    let text = cdp::http_get(port, "/json/list", Duration::from_secs(5))?;
    cdp::parse_targets(&text)
}

fn fetch_targets_with_retry(port: u16) -> Result<Vec<Target>, String> {
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut last_error: String;
    loop {
        match fetch_targets(port) {
            Ok(targets) => return Ok(targets),
            Err(error) => last_error = error,
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "o endpoint DevTools não respondeu na porta {port}: {last_error}"
            ));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}


fn to_pages(targets: Vec<Target>, host: &str) -> Vec<PageInfo> {
    targets
        .into_iter()
        .filter(|target| target.kind == "page")
        .map(|target| PageInfo {
            captcha: target.url.contains(host),
            id: target.id,
            title: target.title,
            url: target.url,
        })
        .collect()
}

/// Status de um emulador: app, socket e páginas (abre e fecha um forward).
#[allow(clippy::too_many_arguments)]
pub async fn session_report(
    adb: &Adb,
    serial: &str,
    vm_id: Option<u32>,
    vm_name: &str,
    package: &str,
    host: &str,
    enable_debug: bool,
    allow_download: bool,
) -> SessionReport {
    let mut report = SessionReport {
        vm_id,
        vm_name: vm_name.to_string(),
        serial: serial.to_string(),
        model: String::new(),
        app_running: false,
        app_pid: None,
        socket: None,
        debug_enabled: false,
        pages: Vec::new(),
        error: None,
    };
    match adb.shell(serial, "getprop ro.product.model").await {
        Ok(model) => report.model = model.trim().to_string(),
        Err(error) => {
            report.error = Some(format!("ADB não respondeu em {serial}: {error}"));
            return report;
        }
    }
    let mut pid = match adb::app_pid(adb, serial, package).await {
        Some(pid) => pid,
        None => {
            report.error = Some(format!("O aplicativo {package} não está em execução."));
            return report;
        }
    };
    report.app_running = true;
    report.app_pid = Some(pid);
    let mut socket = adb::devtools_socket(adb, serial, pid).await;
    if socket.is_none() && enable_debug {
        // A depuração precisa ser habilitada antes de qualquer forward; o PID é
        // relido porque o app pode ter reiniciado durante a instrumentação.
        match ensure_devtools_socket(adb, serial, pid, package, allow_download).await {
            Ok((new_pid, new_socket)) => {
                pid = new_pid;
                socket = Some(new_socket);
                report.app_pid = Some(pid);
                report.debug_enabled = true;
            }
            Err(error) => {
                report.error = Some(error);
                return report;
            }
        }
    }
    let socket = match socket {
        Some(socket) => socket,
        None => {
            report.error = Some(
                "WebView sem socket DevTools. Marque 'Habilitar depuração via Frida' e verifique de novo."
                    .into(),
            );
            return report;
        }
    };
    report.socket = Some(socket.clone());
    match open_forward(adb, serial, &socket).await {
        Ok((port, spec)) => {
            match fetch_targets_with_retry(port) {
                Ok(targets) => report.pages = to_pages(targets, host),
                Err(error) => report.error = Some(error),
            }
            remove_forward(adb, serial, &spec).await;
        }
        Err(error) => report.error = Some(format!("Falha ao criar adb forward: {error}")),
    }
    report
}

/// Lista sessões para as VMs em execução, cruzando com os dispositivos ADB.
pub async fn sessions(
    adb_path: &str,
    running: &[RunningVm],
    package: &str,
    host: &str,
    enable_debug: bool,
    allow_download: bool,
) -> Result<Vec<SessionReport>, String> {
    let adb = Adb::new(adb::locate_adb(adb_path)?);
    let mut ready = adb.devices().await.unwrap_or_default();
    if ready.is_empty() {
        adb.connect_common_ports().await;
        ready = adb.devices().await.unwrap_or_default();
    }
    let mut reports = Vec::new();
    let mut used: Vec<String> = Vec::new();
    for vm in running {
        let preferred = serial_for_vm(vm.id);
        let serial = if ready.contains(&preferred) {
            preferred
        } else {
            // Sem a porta esperada, escolhe um dispositivo ainda não associado.
            match ready
                .iter()
                .find(|serial| !used.contains(serial))
                .cloned()
            {
                Some(serial) => serial,
                None => {
                    reports.push(SessionReport {
                        vm_id: Some(vm.id),
                        vm_name: vm.name.clone(),
                        serial: preferred,
                        model: String::new(),
                        app_running: false,
                        app_pid: None,
                        socket: None,
                        debug_enabled: false,
                        pages: Vec::new(),
                        error: Some(
                            "Emulador sem ADB conectado. Confirme a depuração USB e aguarde o boot."
                                .into(),
                        ),
                    });
                    continue;
                }
            }
        };
        used.push(serial.clone());
        reports.push(
            session_report(
                &adb, &serial, Some(vm.id), &vm.name, package, host, enable_debug,
                allow_download,
            )
            .await,
        );
    }
    // Dispositivos ADB prontos que não correspondem a uma VM listada.
    for serial in ready {
        if used.contains(&serial) {
            continue;
        }
        let vm_id = infer_vm_id(&serial);
        reports.push(
            session_report(
                &adb, &serial, vm_id, "MEmu (ADB)", package, host, enable_debug,
                allow_download,
            )
            .await,
        );
    }
    Ok(reports)
}

/// Garante que o frida-server esteja em execução no dispositivo.
///
/// O `frida` CLI só consegue anexar se o servidor estiver ativo. Ele é iniciado
/// como root com o mesmo binário já enviado ao aparelho; se não existir e o
/// download estiver permitido, uma mensagem explica o que fazer.
async fn ensure_frida_server(adb: &Adb, serial: &str, allow_download: bool) -> Result<(), String> {
    let running = adb
        .shell(serial, "ps -A -o PID,NAME")
        .await
        .map(|text| text.lines().any(|line| line.contains("frida-server")))
        .unwrap_or(false);
    if running {
        return Ok(());
    }
    let installed = adb
        .shell(serial, "[ -x /data/local/tmp/frida-server ] && echo YES")
        .await
        .map(|text| text.contains("YES"))
        .unwrap_or(false);
    if !installed {
        return Err(if allow_download {
            "frida-server não está em /data/local/tmp. Envie o binário oficial com o projeto Python (python main.py enable-debug --download --yes) e tente de novo."
                .into()
        } else {
            "frida-server não está em /data/local/tmp. Envie o binário oficial (mesma versão do frida-tools) antes de habilitar a depuração."
                .into()
        });
    }
    adb.shell(
        serial,
        "su -c 'nohup /data/local/tmp/frida-server >/dev/null 2>&1 &'",
    )
    .await?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let running = adb
            .shell(serial, "ps -A -o PID,NAME")
            .await
            .map(|text| text.lines().any(|line| line.contains("frida-server")))
            .unwrap_or(false);
        if running {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err("frida-server não iniciou. Verifique root e a arquitetura do binário.".into())
}

/// Habilita a depuração e devolve o PID e o socket atualizados.
pub async fn ensure_devtools_socket(
    adb: &Adb,
    serial: &str,
    pid: u32,
    package: &str,
    allow_download: bool,
) -> Result<(u32, String), String> {
    ensure_frida_server(adb, serial, allow_download).await?;
    enable_webview_debugging(serial, pid, Duration::from_secs(60)).await?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut current = pid;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(400));
        current = adb::app_pid(adb, serial, package).await.unwrap_or(current);
        if let Some(socket) = adb::devtools_socket(adb, serial, current).await {
            return Ok((current, socket));
        }
    }
    Err(
        "A depuração foi habilitada, mas o socket não apareceu. Se o aplicativo já tinha criado o WebView, reinicie-o (é necessário para a flag valer) e verifique de novo."
            .into(),
    )
}

// -- Frida (habilitação da depuração) ---------------------------------------

const ENABLE_JS: &str = r#"
var done = false;
var ok = false;
var err = null;
try {
  Java.perform(function () {
    Java.scheduleOnMainThread(function () {
      try {
        Java.use("android.webkit.WebView").setWebContentsDebuggingEnabled(true);
        ok = true;
      } catch (e) { err = String(e); }
      done = true;
    });
  });
} catch (e) { err = String(e); done = true; }
// scheduleOnMainThread apenas enfileira: espera o callback da UI thread.
if (!done) {
  try {
    var Thread = Java.use("java.lang.Thread");
    var deadline = Date.now() + 5000;
    while (!done && Date.now() < deadline) { Thread.sleep(20); }
  } catch (e) { err = err || String(e); }
}
if (ok) { console.log("AWC_ENABLE_OK"); }
else { console.log("AWC_ENABLE_FAIL:" + (err || (done ? "sem detalhe" : "timeout aguardando a UI thread"))); }
"#;

fn frida_executable() -> PathBuf {
    for name in ["frida.exe", "frida"] {
        if let Ok(path) = which(name) {
            return path;
        }
    }
    PathBuf::from("frida")
}

fn which(name: &str) -> Result<PathBuf, ()> {
    let paths = std::env::var_os("PATH").ok_or(())?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(())
}

/// Habilita `setWebContentsDebuggingEnabled(true)` via frida CLI.
pub async fn enable_webview_debugging(
    serial: &str,
    pid: u32,
    timeout: Duration,
) -> Result<(), String> {
    let script_path = std::env::temp_dir().join("memu-hub-enable-webview.js");
    std::fs::write(&script_path, ENABLE_JS)
        .map_err(|e| format!("não foi possível gravar o script Frida: {e}"))?;
    let mut cmd = Command::new(frida_executable());
    cmd.args([
        "-D",
        serial,
        "-p",
        &pid.to_string(),
        "-q",
        "-t",
        "15",
        "-l",
    ])
    .arg(&script_path)
    .kill_on_drop(true);
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| "O Frida não respondeu dentro do tempo limite.".to_string())?
        .map_err(|e| {
            format!("Não foi possível executar o Frida ({e}). Instale frida-tools (mesma versão do frida-server).")
        })?;
    let _ = std::fs::remove_file(&script_path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stdout.contains("AWC_ENABLE_OK") {
        return Ok(());
    }
    let detail = stdout
        .lines()
        .chain(stderr.lines())
        .find(|line| line.contains("AWC_ENABLE_FAIL") || line.contains("Error"))
        .unwrap_or("sem detalhe");
    Err(format!("Falha ao habilitar a depuração via Frida: {detail}"))
}

// -- injeção ----------------------------------------------------------------

/// Injeta o token no WebView do emulador indicado.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_vm_id_from_serial() {
        assert_eq!(infer_vm_id("127.0.0.1:21513"), Some(1));
        assert_eq!(infer_vm_id("127.0.0.1:21523"), Some(2));
        assert_eq!(infer_vm_id("127.0.0.1:21511"), None);
        assert_eq!(infer_vm_id("emulator-5554"), None);
        assert_eq!(serial_for_vm(2), "127.0.0.1:21523");
    }

    #[test]
    fn free_port_is_usable() {
        let port = free_port().unwrap();
        assert!(port > 1024);
    }

    #[test]
    fn pages_flag_captcha_by_host() {
        let pages = to_pages(
            vec![
                Target {
                    id: "A".into(),
                    kind: "page".into(),
                    title: "MuAwaY".into(),
                    url: "https://captcha.muaway.net/android.html".into(),
                },
                Target {
                    id: "B".into(),
                    kind: "page".into(),
                    title: "Outro".into(),
                    url: "https://example.com/".into(),
                },
                Target {
                    id: "C".into(),
                    kind: "iframe".into(),
                    title: "".into(),
                    url: "https://captcha.muaway.net/frame".into(),
                },
            ],
            CAPTCHA_HOST,
        );
        assert_eq!(pages.len(), 2);
        assert!(pages[0].captcha);
        assert!(!pages[1].captcha);
    }
}
