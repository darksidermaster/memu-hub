use serde::Serialize;
use std::{path::PathBuf, time::Duration};
#[derive(Serialize, Debug)]
pub struct Emulator { pub id: u32, pub name: String, pub pid: u32 }
#[derive(Clone, Debug, Serialize)]
pub struct RunningVm { pub id: u32, pub name: String, pub pid: u32 }
pub fn parse_list(text: &str) -> Vec<Emulator> {
    parse_running(text)
        .into_iter()
        .map(|vm| Emulator { id: vm.id, name: vm.name, pid: vm.pid })
        .collect()
}
/// Interpreta `memuc listvms --running`: `id,titulo,PID,rodando,<handle>`.
pub fn parse_running(text: &str) -> Vec<RunningVm> {
    text.lines().filter_map(|line| {
        // Parse from the right: titles may contain commas.
        let (head, pid) = line.trim().rsplit_once(',')?;
        let (head, running) = head.rsplit_once(',')?;
        let (head, _handle) = head.rsplit_once(',')?;
        let (id, title) = head.split_once(',')?;
        if running.trim() != "1" { return None; }
        Some(RunningVm { id: id.trim().parse().ok()?, name: title.trim().to_string(), pid: pid.trim().parse().ok()? })
    }).collect()
}
pub fn locate(custom: &str) -> Result<PathBuf, String> {
    if !custom.trim().is_empty() {
        let p = PathBuf::from(custom.trim());
        if p.is_absolute() && p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("memuc.exe")) { return Ok(p); }
        return Err("Informe o caminho absoluto de um memuc.exe existente.".into());
    }
    let mut candidates = vec![PathBuf::from(r"C:\Microvirt\MEmu\memuc.exe")];
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(base) = std::env::var_os(var) { candidates.push(PathBuf::from(base).join(r"Microvirt\MEmu\memuc.exe")); }
    }
    // O instalador registra InstallLocation; o MEmu pode estar em outro disco.
    #[cfg(windows)]
    {
        use winreg::{enums::*, RegKey};
        for (root, path) in [
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\MEmu"),
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\MEmu"),
            (HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\MEmu"),
        ] {
            if let Ok(key) = RegKey::predef(root).open_subkey(path) {
                if let Ok(location) = key.get_value::<String, _>("InstallLocation") {
                    candidates.push(PathBuf::from(&location).join(r"MEmu\memuc.exe"));
                    candidates.push(PathBuf::from(&location).join("memuc.exe"));
                }
            }
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&paths).filter(|p| p.is_absolute()).map(|p| p.join("memuc.exe")));
    }
    candidates.into_iter().find(|p| p.is_file()).ok_or("MEmu não encontrado. Configure o caminho do memuc.exe na engrenagem.".into())
}
pub async fn run(path: PathBuf, args: &[&str]) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new(path);
    cmd.args(args).kill_on_drop(true);
    #[cfg(windows)] { cmd.creation_flags(0x08000000); }
    let output = tokio::time::timeout(Duration::from_secs(12), cmd.output()).await
        .map_err(|_| "MEmu não respondeu em 12 segundos.")?
        .map_err(|_| "Não foi possível executar memuc.exe.")?;
    if !output.status.success() { return Err("O MEmu retornou erro ao executar o comando.".into()); }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn running_and_comma_names() {
        let v = parse_list("0,MEmu,123,1,456\r\n1,Desligado,0,0,0\n2,Teste, Brasil,222,1,789\nerror");
        assert_eq!(v.len(), 2); assert_eq!(v[1].name, "Teste, Brasil"); assert_eq!(v[0].pid, 456);
    }
}
