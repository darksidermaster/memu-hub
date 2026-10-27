//! Descoberta de emuladores e comandos ADB.
//!
//! Os IDs exibidos na interface do MEmu Hub são índices de VM (`memuc listvms`),
//! não seriais ADB. Este módulo cruza as duas coisas: descobre os endpoints
//! `127.0.0.1:<porta>` do MEmu, obtém o PID do app e mapeia o índice da VM para
//! o serial ADB correspondente comparando o PID do processo do emulador.

use std::{
    path::PathBuf,
    time::Duration,
};
use tokio::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn locate_in(candidates: Vec<PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|p| p.is_file())
}

/// Procura o `adb.exe` em locais conhecidos, no PATH e na instalação do MEmu.
pub fn locate_adb(custom: &str) -> Result<PathBuf, String> {
    if !custom.trim().is_empty() {
        let path = PathBuf::from(custom.trim());
        if path.is_absolute() && path.is_file() {
            return Ok(path);
        }
        return Err("Informe o caminho absoluto de um adb.exe existente.".into());
    }
    let mut candidates = Vec::new();
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(base) = std::env::var_os(var) {
            candidates.push(PathBuf::from(base).join(r"platform-tools\adb.exe"));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local).join(r"Android\Sdk\platform-tools\adb.exe"),
        );
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(base) = std::env::var_os(var) {
            let base = PathBuf::from(base);
            candidates.push(base.join(r"Microvirt\MEmu\adb.exe"));
            candidates.push(base.join(r"Android\android-sdk\platform-tools\adb.exe"));
        }
    }
    candidates.push(PathBuf::from(r"D:\Microvirt\MEmu\adb.exe"));
    candidates.push(PathBuf::from(r"C:\Microvirt\MEmu\adb.exe"));
    if let Some(paths) = std::env::var_os("PATH") {
        candidates.extend(
            std::env::split_paths(&paths)
                .filter(|p| p.is_absolute())
                .map(|p| p.join("adb.exe")),
        );
    }
    locate_in(candidates).ok_or_else(|| {
        "adb.exe não encontrado. Informe o caminho nas configurações.".to_string()
    })
}

/// Executa um comando capturando stdout/stderr com timeout.
async fn run_with_timeout(mut cmd: Command, seconds: u64) -> Result<String, String> {
    cmd.kill_on_drop(true);
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = tokio::time::timeout(Duration::from_secs(seconds), cmd.output())
        .await
        .map_err(|_| format!("O comando não respondeu em {seconds}s."))?
        .map_err(|e| format!("Não foi possível executar o comando: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr.trim();
        return Err(if message.is_empty() {
            "O comando retornou erro.".to_string()
        } else {
            message.to_string()
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub struct Adb {
    path: PathBuf,
}

impl Adb {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub async fn raw(&self, args: &[&str]) -> Result<String, String> {
        let mut cmd = Command::new(&self.path);
        cmd.args(args);
        run_with_timeout(cmd, 20).await
    }

    /// Executa um comando ADB direcionado a um serial.
    pub async fn raw_serial(&self, serial: &str, args: &[&str]) -> Result<String, String> {
        let mut cmd = Command::new(&self.path);
        cmd.args(["-s", serial]).args(args);
        run_with_timeout(cmd, 20).await
    }

    /// Executa um comando shell no dispositivo indicado.
    pub async fn shell(&self, serial: &str, command: &str) -> Result<String, String> {
        let mut cmd = Command::new(&self.path);
        cmd.args(["-s", serial, "shell", command]);
        run_with_timeout(cmd, 20).await
    }

    /// Lista dispositivos em estado `device`.
    pub async fn devices(&self) -> Result<Vec<String>, String> {
        let text = self.raw(&["devices"]).await?;
        Ok(text
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let serial = parts.next()?;
                let state = parts.next()?;
                (state == "device").then(|| serial.to_string())
            })
            .collect())
    }

    /// Conecta portas TCP do MEmu que ainda não estão registradas.
    pub async fn connect_common_ports(&self) {
        for port in (21503..=21563).step_by(2) {
            let address = format!("127.0.0.1:{port}");
            let _ = self.raw(&["connect", &address]).await;
        }
    }
}

/// Resolve o PID do app no dispositivo (nunca fixo).
pub async fn app_pid(adb: &Adb, serial: &str, package: &str) -> Option<u32> {
    adb.shell(serial, &format!("pidof {package}"))
        .await
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse().ok())
}

/// Procura o socket DevTools do PID no `/proc/net/unix`.
pub async fn devtools_socket(adb: &Adb, serial: &str, pid: u32) -> Option<String> {
    let output = match adb.shell(serial, "su -c 'cat /proc/net/unix'").await {
        Ok(text) if !text.trim().is_empty() => text,
        _ => adb.shell(serial, "cat /proc/net/unix").await.ok()?,
    };
    let expected = format!("webview_devtools_remote_{pid}");
    output
        .lines()
        .find(|line| line.to_ascii_lowercase().contains(&expected.to_ascii_lowercase()))
        .and_then(|line| line.split_whitespace().find(|token| token.contains("devtools")))
        .map(|token| token.trim_start_matches('@').to_string())
}
