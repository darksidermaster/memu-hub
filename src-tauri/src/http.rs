//! Cliente HTTP mínimo compartilhado pelos provedores de captcha.
//!
//! Todos os provedores nativos falam JSON sobre HTTPS. Este módulo centraliza o
//! `reqwest` com um runtime próprio (o worker já tem um, mas o modo CLI roda sem
//! runtime ativo), de modo que cada solver chame a função de forma síncrona.

use serde_json::Value;
use std::time::Duration;

/// POST JSON simples. Devolve `(status, corpo)`; corpo inválido vira `Null`.
pub fn post_json(url: &str, body: Value) -> Result<(u16, Value), String> {
    post_json_with_headers(url, body, &[])
}

/// POST JSON com cabeçalhos extras (ex.: `Authorization`).
pub fn post_json_with_headers(
    url: &str,
    body: Value,
    headers: &[(&str, &str)],
) -> Result<(u16, Value), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("runtime HTTP: {error}"))?;
    runtime.block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| format!("cliente HTTP: {error}"))?;
        let mut request = client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&body);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("POST {url} falhou: {error}"))?;
        let status = response.status().as_u16();
        let value = response.json::<Value>().await.unwrap_or(Value::Null);
        Ok((status, value))
    })
}
