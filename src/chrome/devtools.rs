use std::{
    fs,
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::session;

#[derive(Debug, Clone)]
pub struct PageInfo {
    pub id: String,
    pub title: String,
    pub url: String,
    pub web_socket_debugger_url: String,
}

pub fn current_page() -> Result<Option<PageInfo>> {
    current_page_with_hint(None, None)
}

pub fn current_page_with_hint(
    title_hint: Option<&str>,
    url_hint: Option<&str>,
) -> Result<Option<PageInfo>> {
    let pages = list_pages_for_session()?;
    if pages.is_empty() {
        return Ok(None);
    }

    if let Some(target_id) = session::read_browser_tab_target()
        && let Some(page) = pages.iter().find(|page| page.id == target_id).cloned()
    {
        return Ok(Some(page));
    }

    if let Some(url_hint) = session::read_browser_url() {
        let want = canonical_url(&url_hint);
        if let Some(page) = pages
            .iter()
            .find(|page| canonical_url(&page.url) == want)
            .cloned()
        {
            return Ok(Some(page));
        }
    }

    if let Some(url_hint) = url_hint {
        let want = canonical_url(url_hint);
        if let Some(page) = pages
            .iter()
            .find(|page| canonical_url(&page.url) == want)
            .cloned()
        {
            return Ok(Some(page));
        }
    }

    if let Some(title_hint) = title_hint {
        let want = title_hint.trim().to_ascii_lowercase();
        if let Some(page) = pages
            .iter()
            .find(|page| page.title.trim().to_ascii_lowercase() == want)
            .cloned()
        {
            return Ok(Some(page));
        }
    }

    Ok(pages.into_iter().next())
}

pub fn current_url() -> Result<Option<String>> {
    Ok(current_page()?.map(|page| page.url))
}

pub fn current_title() -> Result<Option<String>> {
    Ok(current_page()?.map(|page| page.title))
}

pub fn list_pages_for_session() -> Result<Vec<PageInfo>> {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let Some(port) = devtools_port()? else {
            if std::time::Instant::now() >= deadline {
                return Ok(Vec::new());
            }
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        let pages = list_pages(port)?;
        if !pages.is_empty() || std::time::Instant::now() >= deadline {
            return Ok(pages);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub async fn navigate(
    target_url: &str,
    title_hint: Option<&str>,
    url_hint: Option<&str>,
) -> Result<PageInfo> {
    let page = current_page_with_hint(title_hint, url_hint)?
        .ok_or_else(|| anyhow!("no Chrome page target exposed by DevTools"))?;
    send_page_command(
        &page.web_socket_debugger_url,
        "Page.navigate",
        json!({ "url": target_url }),
    )
    .await?;
    remember_page_target(&page)?;
    let _ = session::remember_browser_url(target_url);
    Ok(page)
}

pub async fn new_page(target_url: &str) -> Result<PageInfo> {
    let value = send_browser_command("Target.createTarget", json!({ "url": target_url })).await?;
    let target_id = value
        .get("targetId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Target.createTarget did not return targetId"))?;
    let page = wait_for_page_by_id(target_id, Duration::from_secs(3)).await?;
    activate_page(&page.id).await?;
    remember_page_target(&page)?;
    let _ = session::remember_browser_url(target_url);
    Ok(page)
}

pub async fn activate_page(target_id: &str) -> Result<()> {
    send_browser_command("Target.activateTarget", json!({ "targetId": target_id })).await?;
    let _ = session::remember_browser_tab_target(target_id);
    if let Some(page) = current_page_by_id(target_id)? {
        remember_page_target(&page)?;
    }
    Ok(())
}

pub async fn close_page(target_id: &str) -> Result<()> {
    send_browser_command("Target.closeTarget", json!({ "targetId": target_id })).await?;
    Ok(())
}

pub async fn capture_screenshot() -> Result<Vec<u8>> {
    let page =
        current_page()?.ok_or_else(|| anyhow!("no Chrome page target exposed by DevTools"))?;
    let value = send_page_command(
        &page.web_socket_debugger_url,
        "Page.captureScreenshot",
        json!({ "format": "png", "fromSurface": true }),
    )
    .await?;
    let encoded = value
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Page.captureScreenshot did not return data"))?;
    BASE64
        .decode(encoded)
        .context("failed to decode Chrome DevTools screenshot payload")
}

/// Evaluate JavaScript in the current page and return its value.
///
/// Used to check facts the accessibility tree cannot express, such as whether
/// the element under a point is disabled.
pub async fn evaluate(expression: &str) -> Result<Value> {
    let page =
        current_page()?.ok_or_else(|| anyhow!("no Chrome page target exposed by DevTools"))?;
    let response = send_page_command(
        &page.web_socket_debugger_url,
        "Runtime.evaluate",
        json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
    )
    .await?;

    // `send_page_command` returns the command result, and Runtime.evaluate
    // nests the value one level deeper, so unwrap it here rather than making
    // every caller know the shape.
    Ok(response
        .get("result")
        .and_then(|result| result.get("value"))
        .cloned()
        .unwrap_or(Value::Null))
}

fn devtools_port() -> Result<Option<u16>> {
    let profile = match session::read_browser_profile() {
        Some(value) => PathBuf::from(value),
        None => return Ok(None),
    };
    let active_port = profile.join("DevToolsActivePort");
    if !active_port.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&active_port)
        .with_context(|| format!("failed to read {}", active_port.display()))?;
    let mut lines = content.lines();
    let port = lines
        .next()
        .ok_or_else(|| anyhow!("DevToolsActivePort missing port line"))?
        .trim();
    if port.is_empty() {
        return Ok(None);
    }

    Ok(Some(port.parse::<u16>().with_context(|| {
        format!("invalid DevTools port {:?}", port)
    })?))
}

fn browser_websocket_url() -> Result<Option<String>> {
    let profile = match session::read_browser_profile() {
        Some(value) => PathBuf::from(value),
        None => return Ok(None),
    };
    let active_port = profile.join("DevToolsActivePort");
    if !active_port.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&active_port)
        .with_context(|| format!("failed to read {}", active_port.display()))?;
    let mut lines = content.lines();
    let port = lines
        .next()
        .ok_or_else(|| anyhow!("DevToolsActivePort missing port line"))?
        .trim();
    if port.is_empty() {
        return Ok(None);
    }
    let path = lines
        .next()
        .ok_or_else(|| anyhow!("DevToolsActivePort missing websocket path line"))?
        .trim();
    if path.is_empty() {
        return Ok(None);
    }

    Ok(Some(format!("ws://127.0.0.1:{}{}", port, path)))
}

fn list_pages(port: u16) -> Result<Vec<PageInfo>> {
    let body = http_get_json(port, "/json/list")?;
    let parsed: Value = serde_json::from_str(&body).context("failed to parse /json/list JSON")?;
    let items = parsed
        .as_array()
        .ok_or_else(|| anyhow!("Chrome DevTools /json/list returned non-array JSON"))?;

    let mut pages = Vec::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("page") {
            continue;
        }
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(title) = item.get("title").and_then(Value::as_str) else {
            continue;
        };
        let Some(url) = item.get("url").and_then(Value::as_str) else {
            continue;
        };
        let Some(ws) = item.get("webSocketDebuggerUrl").and_then(Value::as_str) else {
            continue;
        };
        pages.push(PageInfo {
            id: id.to_string(),
            title: title.to_string(),
            url: url.to_string(),
            web_socket_debugger_url: ws.to_string(),
        });
    }
    Ok(pages)
}

fn http_get_json(port: u16, path: &str) -> Result<String> {
    let addr = ("127.0.0.1", port)
        .to_socket_addrs()
        .context("failed to resolve Chrome DevTools address")?
        .next()
        .ok_or_else(|| anyhow!("no Chrome DevTools address resolved"))?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
        .context("failed to connect to Chrome DevTools HTTP endpoint")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .context("failed to set Chrome DevTools read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .context("failed to set Chrome DevTools write timeout")?;
    write!(
        stream,
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        path, port
    )
    .context("failed to write Chrome DevTools HTTP request")?;
    let mut response = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&chunk[..n]),
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if response.is_empty() {
                    return Err(err).context("failed to read Chrome DevTools HTTP response");
                }
                break;
            }
            Err(err) => return Err(err).context("failed to read Chrome DevTools HTTP response"),
        }
    }
    let response =
        String::from_utf8(response).context("Chrome DevTools HTTP response was not valid UTF-8")?;
    let (_, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow!("malformed Chrome DevTools HTTP response"))?;
    Ok(body.to_string())
}

fn current_page_by_id(target_id: &str) -> Result<Option<PageInfo>> {
    Ok(list_pages_for_session()?
        .into_iter()
        .find(|page| page.id == target_id))
}

async fn wait_for_page_by_id(target_id: &str, timeout: Duration) -> Result<PageInfo> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Some(page) = current_page_by_id(target_id)? {
            return Ok(page);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Err(anyhow!(
        "timed out waiting for Chrome DevTools target {} to appear",
        target_id
    ))
}

async fn send_browser_command(method: &str, params: Value) -> Result<Value> {
    let ws = browser_websocket_url()?
        .ok_or_else(|| anyhow!("no Chrome browser DevTools websocket exposed"))?;
    send_command(&ws, method, params).await
}

async fn send_page_command(page_ws: &str, method: &str, params: Value) -> Result<Value> {
    send_command(page_ws, method, params).await
}

async fn send_command(websocket_url: &str, method: &str, params: Value) -> Result<Value> {
    let (mut socket, _) = connect_async(websocket_url).await.with_context(|| {
        format!(
            "failed to connect to Chrome DevTools websocket {}",
            websocket_url
        )
    })?;

    socket
        .send(Message::Text(
            json!({
                "id": 1,
                "method": method,
                "params": params
            })
            .to_string()
            .into(),
        ))
        .await
        .with_context(|| format!("failed to send Chrome DevTools {}", method))?;

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        let Some(message) = socket.next().await else {
            break;
        };
        let message = message.context("failed reading Chrome DevTools websocket response")?;
        let Message::Text(text) = message else {
            continue;
        };
        let parsed: Value =
            serde_json::from_str(&text).context("failed to parse Chrome DevTools JSON")?;
        if parsed.get("id").and_then(Value::as_i64) == Some(1) {
            if let Some(error) = parsed.get("error") {
                bail!("Chrome DevTools {} failed: {}", method, error);
            }
            return Ok(parsed
                .get("result")
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default())));
        }
    }

    bail!("timed out waiting for Chrome DevTools {} response", method)
}

fn remember_page_target(page: &PageInfo) -> Result<()> {
    session::remember_browser_tab_target(&page.id)?;
    session::remember_browser_url(&page.url)?;
    Ok(())
}

fn canonical_url(raw: &str) -> String {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("http://") {
        rest.to_string()
    } else if let Some(rest) = lower.strip_prefix("https://") {
        rest.to_string()
    } else {
        lower
    }
}
