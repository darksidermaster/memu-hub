//! Cliente mínimo de Chrome DevTools Protocol.
//!
//! O CDP é JSON sobre WebSocket. Este módulo abre uma conexão com o target
//! `/devtools/page/<id>` através do `adb forward`, envia `Runtime.evaluate` e
//! correlaciona respostas por `id`. Eventos são descartados (o injetor não
//! precisa deles), mas mensagens de erro do protocolo são propagadas.

use serde::Serialize;
use serde_json::{json, Value};
use std::{
    net::TcpStream,
    time::Duration,
};
use tungstenite::{client::client, stream::MaybeTlsStream, Message};

/// Alvo DevTools exposto por `/json/list`.
#[derive(Clone, Debug, Serialize)]
pub struct Target {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub url: String,
}

/// Seleciona alvos do tipo `page` cuja URL contenha `host`.
pub fn captcha_targets(targets: &[Target], host: &str) -> Vec<Target> {
    targets
        .iter()
        .filter(|target| target.kind == "page" && target.url.contains(host))
        .cloned()
        .collect()
}

/// Extrai os alvos do JSON de `/json/list`.
pub fn parse_targets(text: &str) -> Result<Vec<Target>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("resposta /json/list inválida: {e}"))?;
    let array = value
        .as_array()
        .ok_or("resposta /json/list não é uma lista")?;
    let mut targets = Vec::new();
    for item in array {
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        targets.push(Target {
            id: id.to_string(),
            kind: item
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            title: item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            url: item
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    Ok(targets)
}

/// Consulta `http://127.0.0.1:<porta>/json/list` via HTTP/1.1 simples.
///
/// O endpoint DevTools mantém a conexão aberta mesmo com `Connection: close`,
/// então o corpo é delimitado por `Content-Length` (ou pela leitura até o
/// timeout, quando o servidor não informa o tamanho).
pub fn http_get(port: u16, path: &str, timeout: Duration) -> Result<String, String> {
    use std::io::{Read, Write};

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .map_err(|e| format!("não foi possível conectar na porta {port}: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("falha ao enviar requisição: {e}"))?;

    let mut response = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut header_end: Option<usize> = None;
    let mut content_length: Option<usize> = None;
    loop {
        if let (Some(end), Some(length)) = (header_end, content_length) {
            if response.len() >= end + 4 + length {
                break;
            }
        }
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                response.extend_from_slice(&buffer[..n]);
                if response.len() > 8 * 1024 * 1024 {
                    return Err("resposta HTTP grande demais".into());
                }
                if header_end.is_none() {
                    if let Some(position) = response
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                    {
                        header_end = Some(position);
                        let head = String::from_utf8_lossy(&response[..position]).to_string();
                        content_length = head.lines().find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.trim()
                                .eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        });
                        if content_length.is_none() {
                            break;
                        }
                    }
                }
            }
            // Timeout de leitura: a resposta é considerada completa.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                break
            }
            Err(error) => return Err(format!("falha ao ler resposta: {error}")),
        }
    }
    let text = String::from_utf8_lossy(&response).to_string();
    let position = header_end
        .or_else(|| text.find("\r\n\r\n"))
        .ok_or("resposta HTTP sem cabeçalho")?;
    let head = &text[..position];
    let body = &text[position + 4..];
    let status_ok = head
        .lines()
        .next()
        .map(|line| line.contains(" 200"))
        .unwrap_or(false);
    if !status_ok {
        return Err(format!(
            "HTTP inesperado em {path}: {}",
            head.lines().next().unwrap_or("")
        ));
    }
    Ok(body.to_string())
}

/// Estado da página do captcha, usado para decidir se vale resolver.
#[derive(Clone, Debug, PartialEq)]
pub struct PageState {
    /// `performance.timeOrigin`: muda a cada navegação/recarga, então identifica
    /// a instância da página. É o sinal para distinguir "mesma janela" de
    /// "captcha nova".
    pub time_origin: f64,
    /// Já existe resposta preenchida no campo (captcha respondida).
    pub answered: bool,
    /// O widget do hCaptcha está presente no DOM.
    pub widget: bool,
}

/// Lê `timeOrigin`, o campo de resposta e a presença do widget.
///
/// Fechar a janela do WebView no jogo **não** remove o target de `/json/list`
/// nem muda `document.visibilityState`: a página continua "visible" e o worker
/// resolveria de novo. `timeOrigin` é o sinal que realmente muda quando o jogo
/// recarrega/recria a página, então é ele que delimita uma nova resolução.
pub const PAGE_STATE_JS: &str = r#"(function () {
  var campo = document.querySelector('textarea[name="h-captcha-response"]')
    || document.querySelector('textarea[name="g-recaptcha-response"]');
  var widget = document.querySelector('#h-captcha, .h-captcha, iframe[src*="hcaptcha"]');
  return {
    origin: (window.performance && performance.timeOrigin) || 0,
    answered: !!(campo && campo.value && campo.value.length > 20),
    widget: !!widget
  };
})()"#;

/// Consulta o estado da página sem gastar crédito de solver.
pub fn page_state(port: u16, target: &Target, timeout: Duration) -> Result<PageState, String> {
    let mut cdp = Cdp::connect(port, &target.id, timeout)?;
    let value = cdp.evaluate(PAGE_STATE_JS);
    cdp.close();
    let value = value?;
    Ok(PageState {
        time_origin: value
            .get("origin")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        answered: value
            .get("answered")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        widget: value
            .get("widget")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// Decide se vale resolver agora.
///
/// O critério é **o campo de resposta estar vazio**. Verificado no ambiente:
/// quando a janela do captcha é fechada, a página continua em `/json/list`,
/// `document.visibilityState` continua `"visible"` e `timeOrigin` não muda —
/// mas o token injetado permanece no campo, então `answered` fica `true` e o
/// worker não gasta crédito. Quando o jogo pede um captcha novo, o campo volta
/// a ficar vazio (`answered == false`) e a resolução acontece.
///
/// `timeOrigin` é mantido em [`PageState`] apenas para diagnóstico: usá-lo como
/// bloqueio impediria resolver uma captcha nova na mesma página.
pub fn should_solve(state: &PageState) -> bool {
    !state.answered
}

/// Resultado de uma injeção.
#[derive(Clone, Debug, Serialize)]
pub struct InjectionOutcome {
    pub injected: bool,
    pub bridge_present: bool,
    pub bridge_called: bool,
    pub fields_filled: Vec<String>,
    pub readback_len: Option<usize>,
    pub readback_ok: bool,
    pub error: Option<String>,
}

/// Monta o JavaScript de injeção com o token escapado por `serde_json`.
///
/// A chamada final é a mesma do callback original da página:
/// `AndroidBridge.handleCallback(token)`.
pub fn injection_script(token: &str, fill_fields: bool) -> String {
    let token_json = serde_json::to_string(token).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"(function (token) {{
  var report = {{ bridge_present: false, bridge_called: false, fields_filled: [], error: null }};
  try {{
    if ({fill}) {{
      var campos = document.querySelectorAll(
        'textarea[name="h-captcha-response"], textarea[name="g-recaptcha-response"]'
      );
      for (var i = 0; i < campos.length; i++) {{
        var campo = campos[i];
        campo.value = token;
        campo.innerHTML = token;
        campo.dispatchEvent(new Event('change', {{ bubbles: true }}));
        report.fields_filled.push(campo.name || campo.id || ('index-' + i));
      }}
    }}
    var bridge = window.AndroidBridge;
    if (bridge && typeof bridge.handleCallback === 'function') {{
      report.bridge_present = true;
      bridge.handleCallback(token);
      report.bridge_called = true;
    }}
  }} catch (e) {{ report.error = String(e); }}
  return report;
}})({token_json})"#,
        fill = if fill_fields { "true" } else { "false" },
        token_json = token_json
    )
}

pub const BRIDGE_CHECK: &str = "!!(window.AndroidBridge && typeof window.AndroidBridge.handleCallback === 'function')";

pub const READBACK: &str = r#"(function () {
  var campo = document.querySelector('textarea[name="h-captcha-response"]');
  if (!campo) { return null; }
  return { length: campo.value.length, prefix: campo.value.slice(0, 16) };
})()"#;

fn script_result_text(value: &Value) -> String {
    match value.get("result") {
        Some(inner) => inner
            .get("description")
            .and_then(Value::as_str)
            .or_else(|| inner.get("value").and_then(Value::as_str))
            .unwrap_or("erro JavaScript")
            .to_string(),
        None => "erro JavaScript".into(),
    }
}

/// Conexão CDP com um target.
pub struct Cdp {
    socket: tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl Cdp {
    pub fn connect(port: u16, target_id: &str, timeout: Duration) -> Result<Self, String> {
        let address = format!("127.0.0.1:{port}");
        let stream = TcpStream::connect(&address)
            .map_err(|e| format!("não foi possível conectar em {address}: {e}"))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|e| e.to_string())?;
        let url = format!("ws://{address}/devtools/page/{target_id}");
        let (socket, _response) = client(url.as_str(), MaybeTlsStream::Plain(stream))
            .map_err(|e| format!("handshake CDP falhou: {e}"))?;
        Ok(Self { socket, next_id: 1 })
    }

    fn send_json(&mut self, method: &str, params: Value) -> Result<u64, String> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({ "id": id, "method": method, "params": params });
        self.socket
            .send(Message::Text(payload.to_string().into()))
            .map_err(|e| format!("falha ao enviar {method}: {e}"))?;
        Ok(id)
    }

    fn read_until(&mut self, wanted_id: u64) -> Result<Value, String> {
        for _ in 0..64 {
            let message = self
                .socket
                .read()
                .map_err(|e| format!("falha ao ler do CDP: {e}"))?;
            let text = match message {
                Message::Text(text) => text.as_str().to_string(),
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => return Err("conexão CDP encerrada".into()),
                Message::Binary(_) | Message::Frame(_) => continue,
            };
            let value: Value = match serde_json::from_str(&text) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if value.get("id").and_then(Value::as_u64) == Some(wanted_id) {
                return Ok(value);
            }
            // Eventos e respostas de outras chamadas são ignorados: o injetor
            // é síncrono e usa uma conexão por injeção.
        }
        Err("nenhuma resposta CDP recebida".into())
    }

    pub fn evaluate(&mut self, expression: &str) -> Result<Value, String> {
        self.evaluate_with(expression, false)
    }

    fn evaluate_with(&mut self, expression: &str, await_promise: bool) -> Result<Value, String> {
        let id = self.send_json(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": await_promise,
                "userGesture": true
            }),
        )?;
        let response = self.read_until(id)?;
        if let Some(error) = response.get("error") {
            return Err(format!(
                "CDP: {}",
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("erro")
            ));
        }
        let result = response.get("result").cloned().unwrap_or(Value::Null);
        if let Some(details) = result.get("exceptionDetails") {
            return Err(format!("JavaScript: {}", script_result_text(details)));
        }
        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub fn close(&mut self) {
        let _ = self.socket.close(None);
    }
}

/// Executa a injeção completa em um target.
pub fn inject(
    port: u16,
    target: &Target,
    token: &str,
    fill_fields: bool,
    bridge_wait: Duration,
) -> Result<InjectionOutcome, String> {
    let mut cdp = Cdp::connect(port, &target.id, Duration::from_secs(10))?;
    let result = (|| -> Result<InjectionOutcome, String> {
        let deadline = std::time::Instant::now() + bridge_wait;
        loop {
            match cdp.evaluate(BRIDGE_CHECK) {
                Ok(value) if value.as_bool().unwrap_or(false) => break,
                _ if std::time::Instant::now() >= deadline => {
                    return Ok(InjectionOutcome {
                        injected: false,
                        bridge_present: false,
                        bridge_called: false,
                        fields_filled: Vec::new(),
                        readback_len: None,
                        readback_ok: false,
                        error: Some(
                            "AndroidBridge não apareceu na página (confirme se é a página do captcha)"
                                .into(),
                        ),
                    })
                }
                _ => std::thread::sleep(Duration::from_millis(250)),
            }
        }

        let script = injection_script(token, fill_fields);
        let report = cdp.evaluate(&script)?;
        let mut outcome = InjectionOutcome {
            injected: false,
            bridge_present: report
                .get("bridge_present")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            bridge_called: report
                .get("bridge_called")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            fields_filled: report
                .get("fields_filled")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            readback_len: None,
            readback_ok: false,
            error: report
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string),
        };
        if let Ok(readback) = cdp.evaluate(READBACK) {
            outcome.readback_len = readback
                .get("length")
                .and_then(Value::as_u64)
                .map(|len| len as usize);
            outcome.readback_ok = outcome.readback_len == Some(token.len());
        }
        outcome.injected = outcome.bridge_called && outcome.error.is_none();
        Ok(outcome)
    })();
    cdp.close();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_page_already_answered() {
        // Estado real após injetar e fechar a janela: campo preenchido.
        let state = PageState { time_origin: 100.0, answered: true, widget: true };
        assert!(!should_solve(&state));
    }

    #[test]
    fn solves_when_field_is_empty() {
        // Captcha nova (ou campo limpo pelo jogo).
        let state = PageState { time_origin: 100.0, answered: false, widget: true };
        assert!(should_solve(&state));
    }

    #[test]
    fn solves_when_field_absent() {
        // Página sem os campos de resposta: precisa resolver.
        let state = PageState { time_origin: 0.0, answered: false, widget: false };
        assert!(should_solve(&state));
    }

    #[test]
    fn same_page_new_captcha_is_not_blocked() {
        // Regressão: uma captcha nova na mesma página (mesmo timeOrigin) precisa
        // ser resolvida; por isso timeOrigin não bloqueia.
        let first = PageState { time_origin: 100.0, answered: false, widget: true };
        assert!(should_solve(&first));
        let after_injection = PageState { time_origin: 100.0, answered: true, widget: true };
        assert!(!should_solve(&after_injection));
        let reset_without_reload =
            PageState { time_origin: 100.0, answered: false, widget: true };
        assert!(should_solve(&reset_without_reload));
    }

    #[test]
    fn parses_targets_and_filters_captcha() {
        let json = r#"[
            {"id":"T1","type":"page","title":"MuAwaY","url":"https://captcha.muaway.net/android.html?x=1"},
            {"id":"T2","type":"page","title":"Outra","url":"https://example.com/"},
            {"id":"T3","type":"iframe","title":"","url":"https://captcha.muaway.net/frame"}
        ]"#;
        let targets = parse_targets(json).unwrap();
        assert_eq!(targets.len(), 3);
        let found = captcha_targets(&targets, "captcha.muaway.net");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "T1");
    }

    #[test]
    fn rejects_malformed_target_list() {
        assert!(parse_targets("{}").is_err());
        assert!(parse_targets("not json").is_err());
    }

    #[test]
    fn script_embeds_token_safely() {
        let token = "P1_abc\";alert(1);\"";
        let script = injection_script(token, true);
        assert!(script.contains("AndroidBridge"));
        assert!(script.contains("handleCallback"));
        assert!(!script.contains("alert(1);\"\""));
        let escaped = serde_json::to_string(token).unwrap();
        assert!(script.contains(&escaped));
    }

    #[test]
    fn script_can_skip_field_filling() {
        let script = injection_script("P1_x", false);
        assert!(script.contains("if (false)"));
        assert!(script.contains("AndroidBridge"));
    }

    #[test]
    fn http_get_reports_bad_connection() {
        // Porta fechada: deve falhar de forma clara, não entrar em pânico.
        let result = http_get(1, "/json/list", Duration::from_millis(200));
        assert!(result.is_err());
    }
}
