use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::session;

#[derive(Debug, Clone)]
pub struct ContextInfo {
    pub context: String,
    pub url: String,
    pub title: String,
    pub is_current: bool,
    pub original_opener: Option<String>,
}

pub async fn list_contexts() -> Result<Vec<ContextInfo>> {
    let mut client = BidiSession::connect().await?;
    client.start().await?;
    let result = client.list_contexts().await;
    let end_result = client.end().await;
    end_result?;
    result
}

pub async fn current_context() -> Result<Option<ContextInfo>> {
    let contexts = list_contexts().await?;
    Ok(contexts
        .iter()
        .find(|context| context.is_current)
        .cloned()
        .or_else(|| {
            let remembered = session::read_browser_url();
            contexts
                .iter()
                .find(|context| remembered.as_deref() == Some(context.url.as_str()))
                .cloned()
        }))
}

pub async fn current_url() -> Result<Option<String>> {
    Ok(current_context().await?.map(|context| context.url))
}

pub async fn current_title() -> Result<Option<String>> {
    Ok(current_context().await?.map(|context| context.title))
}

pub async fn navigate(
    target_url: &str,
    title_hint: Option<&str>,
    url_hint: Option<&str>,
) -> Result<ContextInfo> {
    let mut client = BidiSession::connect().await?;
    client.start().await?;
    let result = async {
        let context = client
            .current_context_with_hint(title_hint, url_hint)
            .await?
            .ok_or_else(|| anyhow!("no Firefox browsing context available"))?;
        client
            .command(
                "browsingContext.navigate",
                json!({
                    "context": context.context,
                    "url": target_url,
                    "wait": "complete"
                }),
            )
            .await?;
        let updated = client
            .current_context_with_hint(None, Some(target_url))
            .await?
            .unwrap_or(context);
        remember_context(&updated)?;
        Ok(updated)
    }
    .await;
    let end_result = client.end().await;
    end_result?;
    result
}

pub async fn navigate_context(context_id: &str, target_url: &str) -> Result<ContextInfo> {
    let mut client = BidiSession::connect().await?;
    client.start().await?;
    let result = async {
        client
            .command(
                "browsingContext.navigate",
                json!({
                    "context": context_id,
                    "url": target_url,
                    "wait": "complete"
                }),
            )
            .await?;
        let contexts = client.list_contexts().await?;
        let updated = contexts
            .into_iter()
            .find(|context| context.context == context_id)
            .ok_or_else(|| anyhow!("navigated Firefox context {} disappeared", context_id))?;
        remember_context(&updated)?;
        Ok(updated)
    }
    .await;
    let end_result = client.end().await;
    end_result?;
    result
}

pub async fn new_context(target_url: &str) -> Result<ContextInfo> {
    new_context_with_type(target_url, "tab").await
}

pub async fn new_tab_via_window_open(target_url: &str) -> Result<ContextInfo> {
    let mut client = BidiSession::connect().await?;
    client.start().await?;
    let result = async {
        let opener = client
            .current_context_with_hint(None, None)
            .await?
            .ok_or_else(|| anyhow!("no Firefox browsing context available for window.open"))?;
        let before = client.list_contexts().await?;
        let before_count = before.len();
        let escaped_url = serde_json::to_string(target_url)
            .context("failed to escape target URL for window.open")?;
        client
            .command(
                "script.evaluate",
                json!({
                    "target": { "context": opener.context.clone() },
                    "expression": format!("window.open({escaped_url}, \"_blank\"); \"ok\""),
                    "awaitPromise": false,
                }),
            )
            .await?;
        let created = client
            .wait_for_new_context(
                &opener.context,
                before_count,
                Some(target_url),
                Duration::from_secs(5),
            )
            .await?;
        client
            .command(
                "browsingContext.activate",
                json!({ "context": created.context.clone() }),
            )
            .await
            .ok();
        remember_context(&created)?;
        Ok(created)
    }
    .await;
    let end_result = client.end().await;
    end_result?;
    result
}

async fn new_context_with_type(target_url: &str, context_type: &str) -> Result<ContextInfo> {
    let mut client = BidiSession::connect().await?;
    client.start().await?;
    let result = async {
        let created = client
            .command("browsingContext.create", json!({ "type": context_type }))
            .await?;
        let context_id = created
            .get("context")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("browsingContext.create did not return a context id"))?
            .to_string();

        if target_url != "about:blank" {
            client
                .command(
                    "browsingContext.navigate",
                    json!({
                        "context": context_id,
                        "url": target_url,
                        "wait": "complete"
                    }),
                )
                .await?;
        }
        client
            .command("browsingContext.activate", json!({ "context": context_id }))
            .await?;
        let contexts = client.list_contexts().await?;
        let context = contexts
            .into_iter()
            .find(|context| canonical_url(&context.url) == canonical_url(target_url))
            .or_else(|| client.last_current_context.clone())
            .ok_or_else(|| anyhow!("new Firefox tab did not appear in context list"))?;
        remember_context(&context)?;
        Ok(context)
    }
    .await;
    let end_result = client.end().await;
    end_result?;
    result
}

pub async fn activate_context_by_index(index: usize) -> Result<ContextInfo> {
    let mut client = BidiSession::connect().await?;
    client.start().await?;
    let result = async {
        let contexts = client.list_contexts().await?;
        let target = contexts
            .get(index)
            .cloned()
            .ok_or_else(|| anyhow!("firefox tab index {} is out of range", index))?;
        if target.is_current {
            remember_context(&target)?;
            return Ok(target);
        }
        client
            .command(
                "browsingContext.activate",
                json!({ "context": target.context.clone() }),
            )
            .await?;
        remember_context(&target)?;
        match client
            .wait_for_current_context(
                |context| context.context == target.context,
                Duration::from_secs(2),
            )
            .await
        {
            Ok(settled) => {
                remember_context(&settled)?;
                Ok(settled)
            }
            Err(_) => Ok(target),
        }
    }
    .await;
    let end_result = client.end().await;
    end_result?;
    result
}

pub async fn activate_context_by_title_contains(needle: &str) -> Result<ContextInfo> {
    let mut client = BidiSession::connect().await?;
    client.start().await?;
    let result = async {
        let contexts = client.list_contexts().await?;
        let target = resolve_by_title_contains(&contexts, needle)?.clone();
        if target.is_current {
            remember_context(&target)?;
            return Ok(target);
        }
        client
            .command(
                "browsingContext.activate",
                json!({ "context": target.context.clone() }),
            )
            .await?;
        remember_context(&target)?;
        match client
            .wait_for_current_context(
                |context| context.context == target.context,
                Duration::from_secs(2),
            )
            .await
        {
            Ok(settled) => {
                remember_context(&settled)?;
                Ok(settled)
            }
            Err(_) => Ok(target),
        }
    }
    .await;
    let end_result = client.end().await;
    end_result?;
    result
}

pub async fn close_context_by_index(index: usize) -> Result<String> {
    let mut client = BidiSession::connect().await?;
    client.start().await?;
    let result = async {
        let contexts = client.list_contexts().await?;
        let target = contexts
            .get(index)
            .cloned()
            .ok_or_else(|| anyhow!("firefox tab index {} is out of range", index))?;
        let previous_count = contexts.len();
        client
            .command(
                "browsingContext.close",
                json!({ "context": target.context.clone() }),
            )
            .await?;
        client
            .wait_for_context_close(&target.context, previous_count, Duration::from_secs(4))
            .await
    }
    .await;
    let end_result = client.end().await;
    end_result?;
    result
}

pub async fn capture_screenshot() -> Result<Vec<u8>> {
    let mut client = BidiSession::connect().await?;
    client.start().await?;
    let result = async {
        let current = client
            .current_context_with_hint(None, None)
            .await?
            .ok_or_else(|| anyhow!("no Firefox browsing context available"))?;
        let value = client
            .command(
                "browsingContext.captureScreenshot",
                json!({ "context": current.context }),
            )
            .await?;
        let encoded = value
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("browsingContext.captureScreenshot did not return data"))?;
        BASE64
            .decode(encoded)
            .context("failed to decode Firefox BiDi screenshot payload")
    }
    .await;
    let end_result = client.end().await;
    end_result?;
    result
}

struct BidiSession {
    stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: u64,
    started: bool,
    last_current_context: Option<ContextInfo>,
}

impl BidiSession {
    async fn connect() -> Result<Self> {
        let port = session::read_browser_port()
            .ok_or_else(|| anyhow!("no Firefox BiDi port remembered for the current session"))?;
        Self::connect_to_port(port).await
    }

    async fn connect_to_port(port: u16) -> Result<Self> {
        let url = format!("ws://127.0.0.1:{port}/session");
        let (stream, _) = connect_async(&url)
            .await
            .with_context(|| format!("failed connecting to Firefox BiDi websocket {}", url))?;
        Ok(Self {
            stream,
            next_id: 0,
            started: false,
            last_current_context: None,
        })
    }

    async fn start(&mut self) -> Result<()> {
        if self.started {
            return Ok(());
        }
        self.command(
            "session.new",
            json!({ "capabilities": { "alwaysMatch": {} } }),
        )
        .await?;
        self.started = true;
        Ok(())
    }

    async fn end(&mut self) -> Result<()> {
        if !self.started {
            return Ok(());
        }
        let _ = self.command("session.end", json!({})).await?;
        self.started = false;
        Ok(())
    }

    async fn command(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let payload = json!({
            "id": id,
            "method": method,
            "params": params,
        });
        self.stream
            .send(Message::Text(payload.to_string().into()))
            .await
            .with_context(|| format!("failed sending Firefox BiDi command {}", method))?;

        tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(message) = self.stream.next().await {
                let message = message.context("failed reading Firefox BiDi websocket message")?;
                match message {
                    Message::Text(text) => {
                        let value: Value = serde_json::from_str(&text).with_context(|| {
                            format!("failed parsing Firefox BiDi message {}", text)
                        })?;
                        if value.get("id").and_then(Value::as_u64) != Some(id) {
                            continue;
                        }
                        match value.get("type").and_then(Value::as_str) {
                            Some("success") => {
                                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
                            }
                            Some("error") => {
                                bail!(
                                    "Firefox BiDi {} failed: {}",
                                    method,
                                    value
                                        .get("message")
                                        .and_then(Value::as_str)
                                        .unwrap_or("unknown error")
                                );
                            }
                            _ => continue,
                        }
                    }
                    Message::Binary(bytes) => {
                        let text = String::from_utf8(bytes.to_vec())
                            .context("failed decoding Firefox BiDi binary message")?;
                        let value: Value = serde_json::from_str(&text).with_context(|| {
                            format!("failed parsing Firefox BiDi message {}", text)
                        })?;
                        if value.get("id").and_then(Value::as_u64) != Some(id) {
                            continue;
                        }
                        match value.get("type").and_then(Value::as_str) {
                            Some("success") => {
                                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
                            }
                            Some("error") => {
                                bail!(
                                    "Firefox BiDi {} failed: {}",
                                    method,
                                    value
                                        .get("message")
                                        .and_then(Value::as_str)
                                        .unwrap_or("unknown error")
                                );
                            }
                            _ => continue,
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }

            Err(anyhow!(
                "Firefox BiDi websocket closed before {} returned",
                method
            ))
        })
        .await
        .with_context(|| format!("timed out waiting for Firefox BiDi {}", method))?
    }

    async fn list_contexts(&mut self) -> Result<Vec<ContextInfo>> {
        let tree = self.command("browsingContext.getTree", json!({})).await?;
        let contexts = tree
            .get("contexts")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("browsingContext.getTree did not return a contexts array"))?;
        let mut listed = Vec::new();
        for item in contexts {
            let context_id = item
                .get("context")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("browsingContext.getTree entry missing context id"))?;
            let url = item
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("about:blank")
                .to_string();
            // Privileged contexts (for example `about:home` on a fresh profile)
            // reject script evaluation unless Firefox was started with
            // `--remote-allow-system-access`. A single unreadable context must not
            // abort enumeration, otherwise launch detection never becomes ready.
            let title = self
                .evaluate_string(context_id, "document.title")
                .await
                .unwrap_or_default();
            let visibility = self
                .evaluate_string(context_id, "document.visibilityState")
                .await
                .unwrap_or_default();
            let info = ContextInfo {
                context: context_id.to_string(),
                url,
                title: if title.trim().is_empty() {
                    "<untitled>".to_string()
                } else {
                    title
                },
                is_current: visibility == "visible",
                original_opener: item
                    .get("originalOpener")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            };
            listed.push(info);
        }

        if listed.is_empty() {
            return Ok(listed);
        }

        normalize_current_contexts(&mut listed, self.last_current_context.as_ref());

        self.last_current_context = listed.iter().find(|context| context.is_current).cloned();
        Ok(listed)
    }

    async fn current_context_with_hint(
        &mut self,
        title_hint: Option<&str>,
        url_hint: Option<&str>,
    ) -> Result<Option<ContextInfo>> {
        let contexts = self.list_contexts().await?;
        if contexts.is_empty() {
            return Ok(None);
        }

        if let Some(url_hint) = url_hint {
            let want = canonical_url(url_hint);
            if let Some(context) = contexts
                .iter()
                .find(|context| canonical_url(&context.url) == want)
                .cloned()
            {
                return Ok(Some(context));
            }
        }

        if let Some(title_hint) = title_hint {
            let want = title_hint.trim().to_ascii_lowercase();
            if let Some(context) = contexts
                .iter()
                .find(|context| context.title.trim().to_ascii_lowercase() == want)
                .cloned()
            {
                return Ok(Some(context));
            }
        }

        Ok(contexts
            .iter()
            .find(|context| context.is_current)
            .cloned()
            .or_else(|| contexts.first().cloned()))
    }

    async fn evaluate_string(&mut self, context_id: &str, expression: &str) -> Result<String> {
        let value = self
            .command(
                "script.evaluate",
                json!({
                    "target": { "context": context_id },
                    "expression": expression,
                    "awaitPromise": false,
                }),
            )
            .await?;
        Ok(value
            .get("result")
            .and_then(|value| value.get("value"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    async fn wait_for_current_context<P>(
        &mut self,
        predicate: P,
        timeout: Duration,
    ) -> Result<ContextInfo>
    where
        P: Fn(&ContextInfo) -> bool,
    {
        let start = Instant::now();
        loop {
            let contexts = self.list_contexts().await?;
            if let Some(context) = contexts
                .iter()
                .find(|context| context.is_current && predicate(context))
                .cloned()
            {
                return Ok(context);
            }
            if start.elapsed() >= timeout {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        Err(anyhow!("timed out waiting for Firefox tab activation"))
    }

    async fn wait_for_context_close(
        &mut self,
        closed_context_id: &str,
        previous_count: usize,
        timeout: Duration,
    ) -> Result<String> {
        let start = Instant::now();
        let mut attempts = 0u64;
        loop {
            attempts += 1;
            let contexts = self.list_contexts().await?;
            if contexts.len() != previous_count
                || contexts
                    .iter()
                    .all(|context| context.context != closed_context_id)
            {
                if let Some(current) = contexts.iter().find(|context| context.is_current) {
                    remember_context(current)?;
                }
                return Ok(format!(
                    "firefox tabs recovered after close; {} tabs visible after {}ms ({} attempts)",
                    contexts.len(),
                    start.elapsed().as_millis(),
                    attempts
                ));
            }
            if start.elapsed() >= timeout {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        Err(anyhow!("timed out waiting for Firefox tab close recovery"))
    }

    async fn wait_for_new_context(
        &mut self,
        opener_context: &str,
        previous_count: usize,
        expected_url: Option<&str>,
        timeout: Duration,
    ) -> Result<ContextInfo> {
        let start = Instant::now();
        let expected_url = expected_url.map(canonical_url);
        loop {
            let contexts = self.list_contexts().await?;
            if contexts.len() > previous_count {
                if let Some(context) = contexts
                    .iter()
                    .find(|context| {
                        context.original_opener.as_deref() == Some(opener_context)
                            && expected_url
                                .as_deref()
                                .map(|expected| canonical_url(&context.url) == expected)
                                .unwrap_or(true)
                    })
                    .cloned()
                {
                    return Ok(context);
                }
                if let Some(context) = contexts
                    .iter()
                    .rev()
                    .find(|context| {
                        expected_url
                            .as_deref()
                            .map(|expected| canonical_url(&context.url) == expected)
                            .unwrap_or(true)
                    })
                    .cloned()
                {
                    return Ok(context);
                }
            }
            if start.elapsed() >= timeout {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        Err(anyhow!(
            "timed out waiting for Firefox window.open tab creation"
        ))
    }
}

pub async fn current_context_on_port(
    port: u16,
    url_hint: Option<&str>,
) -> Result<Option<ContextInfo>> {
    let mut client = BidiSession::connect_to_port(port).await?;
    client.start().await?;
    let result = client.current_context_with_hint(None, url_hint).await;
    let end_result = client.end().await;
    end_result?;
    result
}

fn remember_context(context: &ContextInfo) -> Result<()> {
    let _ = session::remember_browser_url(&context.url);
    Ok(())
}

fn normalize_current_contexts(
    listed: &mut [ContextInfo],
    last_current_context: Option<&ContextInfo>,
) {
    if let Some(url) = session::read_browser_url()
        && let Some(index) = listed
            .iter()
            .position(|context| canonical_url(&context.url) == canonical_url(&url))
    {
        for (i, context) in listed.iter_mut().enumerate() {
            context.is_current = i == index;
        }
        return;
    }

    let current_count = listed.iter().filter(|context| context.is_current).count();
    if current_count == 1 {
        return;
    }

    let preferred_index = preferred_current_index(listed, last_current_context).unwrap_or(0);
    for (index, context) in listed.iter_mut().enumerate() {
        context.is_current = index == preferred_index;
    }
}

fn preferred_current_index(
    listed: &[ContextInfo],
    last_current_context: Option<&ContextInfo>,
) -> Option<usize> {
    if let Some(last) = last_current_context
        && let Some(index) = listed
            .iter()
            .position(|context| context.context == last.context)
    {
        return Some(index);
    }

    listed.iter().rposition(|context| context.is_current)
}

fn resolve_by_title_contains<'a>(
    contexts: &'a [ContextInfo],
    needle: &str,
) -> Result<&'a ContextInfo> {
    let needle = needle.trim();
    if needle.is_empty() {
        return Err(anyhow!("firefox tab title query cannot be empty"));
    }

    let normalized = needle.to_ascii_lowercase();
    let mut matches = contexts
        .iter()
        .filter(|context| context.title.to_ascii_lowercase().contains(&normalized));

    let first = matches
        .next()
        .ok_or_else(|| anyhow!("no firefox tab title contains {:?}", needle))?;

    if let Some(second) = matches.next() {
        return Err(anyhow!(
            "firefox tab title query {:?} is ambiguous; matched {:?} and {:?}",
            needle,
            first.title,
            second.title
        ));
    }

    Ok(first)
}

fn canonical_url(raw: &str) -> String {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("about:")
        || lower.starts_with("firefox:")
        || lower.starts_with("moz-extension:")
        || lower.starts_with("file:")
        || lower.starts_with("data:")
    {
        lower
    } else if let Some(rest) = lower.strip_prefix("http://") {
        rest.to_string()
    } else if let Some(rest) = lower.strip_prefix("https://") {
        rest.to_string()
    } else {
        lower
    }
}
