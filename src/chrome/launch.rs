use std::{
    collections::HashSet,
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use tokio::time::sleep;

use crate::chrome::{session, window as chrome_window};

#[derive(Debug, Clone, Serialize)]
pub struct ChromeLaunchState {
    pub pid: u32,
    pub profile: String,
    pub log_path: String,
    pub url: String,
    pub window_id: String,
    pub window_title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub async fn launch(
    initial_url: Option<&str>,
    profile_override: Option<&str>,
    timeout_ms: u64,
    poll_ms: u64,
) -> Result<String> {
    let state = launch_and_wait(initial_url, profile_override, timeout_ms, poll_ms).await?;
    Ok(format!(
        concat!(
            "launched chrome pid {} with profile {}\n",
            "window: {} ({}, {}x{} at {},{})\n",
            "url: {}\n",
            "log: {}",
        ),
        state.pid,
        state.profile,
        state.window_id,
        state.window_title,
        state.width,
        state.height,
        state.x,
        state.y,
        state.url,
        state.log_path,
    ))
}

pub async fn launch_and_wait(
    initial_url: Option<&str>,
    profile_override: Option<&str>,
    timeout_ms: u64,
    poll_ms: u64,
) -> Result<ChromeLaunchState> {
    // The browser only registers with the accessibility bus at startup, so the
    // stack has to be ready before it is spawned. Otherwise the window appears
    // but the tree stays empty for the life of that browser, and every page
    // command fails while the window is plainly on screen.
    crate::runtime::ensure_accessibility_stack()?;
    let url = normalize_launch_url(initial_url.unwrap_or("about:blank"))?;
    let profile_dir = prepare_profile_dir(profile_override)?;
    cleanup_existing_profile_processes(&profile_dir)?;
    let baseline: HashSet<String> = chrome_window::list_browser_windows(None)
        .unwrap_or_default()
        .into_iter()
        .map(|window| window.id)
        .collect();
    let log_path = unique_path("axonbrowser-chrome-launch", "log");
    let browser_binary = find_browser_binary()?;
    let pid = spawn_browser(&browser_binary, &profile_dir, &log_path, &url)?;
    let _ = session::remember_browser_profile(&profile_dir.display().to_string());
    let _ = session::remember_browser_url(&url);

    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    let interval = Duration::from_millis(poll_ms.max(1));

    loop {
        if let Some(window) = find_window_for_pid(pid)?
            && devtools_ready(&url)?
        {
            let _ = crate::window::activate_window(&window.id);
            let _ = session::remember_browser_window_target(&window.id);
            let _ = remember_initial_devtools_tab(&url);
            return Ok(ChromeLaunchState {
                pid,
                profile: profile_dir.display().to_string(),
                log_path: log_path.display().to_string(),
                url: url.clone(),
                window_id: window.id,
                window_title: window.name,
                x: window.x,
                y: window.y,
                width: window.width,
                height: window.height,
            });
        }

        if let Some(window) = detect_new_window(&baseline)?
            && devtools_ready(&url)?
        {
            let _ = crate::window::activate_window(&window.id);
            let _ = session::remember_browser_window_target(&window.id);
            let _ = remember_initial_devtools_tab(&url);
            return Ok(ChromeLaunchState {
                pid,
                profile: profile_dir.display().to_string(),
                log_path: log_path.display().to_string(),
                url: url.clone(),
                window_id: window.id,
                window_title: window.name,
                x: window.x,
                y: window.y,
                width: window.width,
                height: window.height,
            });
        }

        if start.elapsed() >= timeout {
            if let Some(window) = find_window_for_pid(pid)?.or_else(|| {
                chrome_window::list_browser_windows(None)
                    .ok()?
                    .into_iter()
                    .next()
            }) && devtools_ready(&url)?
            {
                let _ = crate::window::activate_window(&window.id);
                let _ = session::remember_browser_window_target(&window.id);
                let _ = remember_initial_devtools_tab(&url);
                return Ok(ChromeLaunchState {
                    pid,
                    profile: profile_dir.display().to_string(),
                    log_path: log_path.display().to_string(),
                    url: url.clone(),
                    window_id: window.id,
                    window_title: window.name,
                    x: window.x,
                    y: window.y,
                    width: window.width,
                    height: window.height,
                });
            }
            session::clear_browser_session_state();
            bail!(
                "timed out after {}ms waiting for a new chrome window; log: {}",
                timeout_ms,
                log_path.display()
            );
        }

        sleep(interval).await;
    }
}

fn detect_new_window(baseline: &HashSet<String>) -> Result<Option<crate::window::WindowMatch>> {
    for window in chrome_window::list_browser_windows(None)? {
        if !is_launch_candidate(&window) {
            continue;
        }
        if !baseline.contains(&window.id) {
            return Ok(Some(window));
        }
    }
    Ok(None)
}

fn find_window_for_pid(pid: u32) -> Result<Option<crate::window::WindowMatch>> {
    let output = Command::new("xdotool")
        .args(["search", "--onlyvisible", "--pid", &pid.to_string(), ".*"])
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }

    let output_text = String::from_utf8_lossy(&output.stdout).to_string();
    let ids = output_text
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();

    Ok(chrome_window::list_browser_windows(None)?
        .into_iter()
        .find(|window| ids.iter().any(|id| id == &window.id)))
}

fn is_launch_candidate(window: &crate::window::WindowMatch) -> bool {
    let normalized = chrome_window::normalize(&window.name);
    !normalized.contains("clipboard") && window.width >= 200 && window.height >= 120
}

fn spawn_browser(
    browser_binary: &str,
    profile_dir: &Path,
    log_path: &Path,
    url: &str,
) -> Result<u32> {
    let stdout = File::create(log_path).with_context(|| {
        format!(
            "failed to create chrome launch log at {}",
            log_path.display()
        )
    })?;
    let stderr = stdout.try_clone().with_context(|| {
        format!(
            "failed to clone chrome launch log handle {}",
            log_path.display()
        )
    })?;

    Command::new("setsid")
        .arg("-f")
        .arg(browser_binary)
        .arg(format!("--user-data-dir={}", profile_dir.display()))
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-search-engine-choice-screen",
            "--force-renderer-accessibility",
            "--remote-debugging-port=0",
            "--new-window",
            "--window-size=1280,900",
            "--disable-background-networking",
            "--disable-sync",
            "--disable-component-update",
            "--disable-breakpad",
            "--no-sandbox",
        ])
        .arg(url)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| format!("failed to detach {browser_binary}"))?;

    find_browser_pid(profile_dir, url)
}

fn find_browser_pid(profile_dir: &Path, url: &str) -> Result<u32> {
    let profile_arg = chrome_profile_arg(profile_dir);
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        let output = Command::new("pgrep").args(["-af", "chrome"]).output()?;
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if !line.contains(&profile_arg) || !line.contains(url) {
                    continue;
                }
                if let Some((pid_text, _)) = line.trim().split_once(' ')
                    && let Ok(pid) = pid_text.parse::<u32>()
                {
                    return Ok(pid);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    Err(anyhow!(
        "failed to resolve launched chrome pid for profile {}",
        profile_dir.display()
    ))
}

fn cleanup_existing_profile_processes(profile_dir: &Path) -> Result<()> {
    let profile_arg = chrome_profile_arg(profile_dir);
    session::clear_browser_session_state();
    let output = Command::new("pgrep")
        .args(["-af", "chrome"])
        .output()
        .context("failed to inspect existing chrome processes")?;
    if output.status.success() {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if !line.contains(&profile_arg) {
                continue;
            }
            let Some((pid_text, _)) = line.trim().split_once(' ') else {
                continue;
            };
            let Ok(pid) = pid_text.parse::<u32>() else {
                continue;
            };
            let _ = terminate_pid(pid);
        }
    }

    for name in [
        "DevToolsActivePort",
        "SingletonCookie",
        "SingletonLock",
        "SingletonSocket",
    ] {
        let path = profile_dir.join(name);
        if path.exists() {
            let _ = fs::remove_file(path);
        }
    }

    Ok(())
}

fn chrome_profile_arg(profile_dir: &Path) -> String {
    format!("--user-data-dir={}", profile_dir.display())
}

fn terminate_pid(pid: u32) -> Result<()> {
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        let alive = Command::new("sh")
            .args(["-lc", &format!("kill -0 {pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !alive {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Ok(())
}

fn find_browser_binary() -> Result<String> {
    for candidate in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
    ] {
        let output = Command::new("which")
            .arg(candidate)
            .output()
            .with_context(|| format!("failed checking browser binary {candidate}"))?;
        if output.status.success() {
            let resolved = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !resolved.is_empty() {
                return Ok(resolved);
            }
        }
    }

    Err(anyhow!(
        "could not find a Chrome/Chromium binary in PATH (tried google-chrome, google-chrome-stable, chromium, chromium-browser)"
    ))
}

fn prepare_profile_dir(profile_override: Option<&str>) -> Result<PathBuf> {
    let path = profile_override
        .map(PathBuf::from)
        .unwrap_or_else(|| unique_path("axonbrowser-chrome-profile", "dir"));
    fs::create_dir_all(&path)
        .with_context(|| format!("failed to create chrome profile dir {}", path.display()))?;
    Ok(path)
}

fn unique_path(prefix: &str, suffix: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    std::env::temp_dir().join(format!("{prefix}-{stamp}.{suffix}"))
}

fn normalize_launch_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("launch URL must not be empty");
    }
    if trimmed.contains(char::is_whitespace) {
        bail!("launch URL must not contain whitespace: {:?}", raw);
    }
    if trimmed.contains("://")
        || trimmed.starts_with("about:")
        || trimmed.starts_with("chrome:")
        || trimmed.starts_with("file:")
        || trimmed.starts_with("data:")
    {
        return Ok(trimmed.to_string());
    }
    Ok(format!("https://{}", trimmed))
}

fn remember_initial_devtools_tab(url: &str) -> Result<()> {
    if let Some(page) = crate::chrome::devtools::current_page_with_hint(None, Some(url))? {
        let _ = session::remember_browser_tab_target(&page.id);
        let _ = session::remember_browser_url(&page.url);
    }
    Ok(())
}

fn devtools_ready(url: &str) -> Result<bool> {
    if let Some(page) = crate::chrome::devtools::current_page_with_hint(None, Some(url))? {
        let _ = session::remember_browser_tab_target(&page.id);
        let _ = session::remember_browser_url(&page.url);
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::normalize_launch_url;

    #[test]
    fn preserves_special_urls() {
        assert_eq!(normalize_launch_url("about:blank").unwrap(), "about:blank");
        assert_eq!(
            normalize_launch_url("https://example.com").unwrap(),
            "https://example.com"
        );
    }

    #[test]
    fn prefixes_https_for_bare_hosts() {
        assert_eq!(
            normalize_launch_url("example.com").unwrap(),
            "https://example.com"
        );
    }
}
