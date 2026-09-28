use std::{
    collections::HashSet,
    fs::{self, File},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use tokio::time::sleep;

use super::{bidi, session, window};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserFlavor {
    Firefox,
    Camoufox,
}

impl BrowserFlavor {
    pub fn label(self) -> &'static str {
        match self {
            Self::Firefox => "firefox",
            Self::Camoufox => "camoufox",
        }
    }

    fn user_profile_prefix(self) -> &'static str {
        match self {
            Self::Firefox => "axonbrowser-firefox-profile",
            Self::Camoufox => "axonbrowser-camoufox-profile",
        }
    }

    fn launch_log_prefix(self) -> &'static str {
        match self {
            Self::Firefox => "axonbrowser-firefox-launch",
            Self::Camoufox => "axonbrowser-camoufox-launch",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FirefoxLaunchState {
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
    pub bidi_port: u16,
}

pub async fn launch_with_flavor(
    flavor: BrowserFlavor,
    initial_url: Option<&str>,
    profile_override: Option<&str>,
    timeout_ms: u64,
    poll_ms: u64,
) -> Result<String> {
    let state =
        launch_and_wait_with_flavor(flavor, initial_url, profile_override, timeout_ms, poll_ms)
            .await?;
    Ok(format!(
        concat!(
            "launched {} pid {} with profile {}\n",
            "window: {} ({}, {}x{} at {},{})\n",
            "url: {}\n",
            "bidi: ws://127.0.0.1:{}/session\n",
            "log: {}",
        ),
        flavor.label(),
        state.pid,
        state.profile,
        state.window_id,
        state.window_title,
        state.width,
        state.height,
        state.x,
        state.y,
        state.url,
        state.bidi_port,
        state.log_path,
    ))
}

pub async fn launch_and_wait_with_flavor(
    flavor: BrowserFlavor,
    initial_url: Option<&str>,
    profile_override: Option<&str>,
    timeout_ms: u64,
    poll_ms: u64,
) -> Result<FirefoxLaunchState> {
    // The browser only registers with the accessibility bus at startup, so the
    // stack has to be ready before it is spawned. Otherwise the window appears
    // but the tree stays empty for the life of that browser, and every page
    // command fails while the window is plainly on screen.
    crate::runtime::ensure_accessibility_stack()?;
    let url = normalize_launch_url(initial_url.unwrap_or("about:blank"))?;
    let profile_dir = prepare_profile_dir(flavor, profile_override)?;
    let browser_binary = find_browser_binary(flavor)?;
    cleanup_existing_profile_processes(&profile_dir)?;
    let bidi_port = allocate_bidi_port()?;
    let baseline: HashSet<String> = window::list_firefox_windows(None)
        .unwrap_or_default()
        .into_iter()
        .map(|window| window.id)
        .collect();

    let log_path = unique_path(flavor.launch_log_prefix(), "log");
    let pid = spawn_browser(
        flavor,
        &browser_binary,
        &profile_dir,
        &log_path,
        &url,
        bidi_port,
    )?;
    let _ = session::remember_browser_profile(&profile_dir.display().to_string());
    let _ = session::remember_browser_url(&url);
    let _ = session::remember_browser_port(bidi_port);

    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms.max(15_000));
    let interval = Duration::from_millis(poll_ms.max(1));
    // Keep the most recent BiDi failure so a launch timeout explains itself
    // instead of only reporting that no window appeared.
    let mut last_bidi_error: Option<String> = None;

    loop {
        if let Some(current) = bidi_ready_on_port(bidi_port, &url, &mut last_bidi_error).await? {
            let preferred = choose_window_for_context(pid, &baseline, current.title.as_str())?;
            if let Some(window_match) =
                preferred.or_else(|| find_window_for_pid(pid).ok().flatten())
            {
                let _ = crate::window::activate_window(&window_match.id);
                let _ = session::remember_browser_window_target(&window_match.id);
                return Ok(FirefoxLaunchState {
                    pid,
                    profile: profile_dir.display().to_string(),
                    log_path: log_path.display().to_string(),
                    url: url.clone(),
                    window_id: window_match.id,
                    window_title: window_match.name,
                    x: window_match.x,
                    y: window_match.y,
                    width: window_match.width,
                    height: window_match.height,
                    bidi_port,
                });
            }
        }

        if let Some(window_match) = find_window_for_pid(pid)?
            && bidi_ready_on_port(bidi_port, &url, &mut last_bidi_error)
                .await?
                .is_some()
        {
            let _ = crate::window::activate_window(&window_match.id);
            let _ = session::remember_browser_window_target(&window_match.id);
            return Ok(FirefoxLaunchState {
                pid,
                profile: profile_dir.display().to_string(),
                log_path: log_path.display().to_string(),
                url: url.clone(),
                window_id: window_match.id,
                window_title: window_match.name,
                x: window_match.x,
                y: window_match.y,
                width: window_match.width,
                height: window_match.height,
                bidi_port,
            });
        }

        if let Some(window_match) = detect_new_window(&baseline)?
            && bidi_ready_on_port(bidi_port, &url, &mut last_bidi_error)
                .await?
                .is_some()
        {
            let _ = crate::window::activate_window(&window_match.id);
            let _ = session::remember_browser_window_target(&window_match.id);
            return Ok(FirefoxLaunchState {
                pid,
                profile: profile_dir.display().to_string(),
                log_path: log_path.display().to_string(),
                url: url.clone(),
                window_id: window_match.id,
                window_title: window_match.name,
                x: window_match.x,
                y: window_match.y,
                width: window_match.width,
                height: window_match.height,
                bidi_port,
            });
        }

        if start.elapsed() >= timeout {
            session::clear_browser_session_state();
            bail!(
                "timed out after {}ms waiting for a new {} window; log: {}{}",
                timeout_ms,
                flavor.label(),
                log_path.display(),
                last_bidi_error
                    .as_deref()
                    .map(|err| format!("; last BiDi error: {err}"))
                    .unwrap_or_default(),
            );
        }

        sleep(interval).await;
    }
}

fn detect_new_window(baseline: &HashSet<String>) -> Result<Option<crate::window::WindowMatch>> {
    for window_match in window::list_firefox_windows(None)? {
        if is_launch_candidate(&window_match) && !baseline.contains(&window_match.id) {
            return Ok(Some(window_match));
        }
    }
    Ok(None)
}

fn find_window_for_pid(pid: u32) -> Result<Option<crate::window::WindowMatch>> {
    Ok(window::list_firefox_windows(None)?
        .into_iter()
        .find(|window| {
            is_launch_candidate(window)
                && crate::window::window_pid(&window.id)
                    .map(|window_pid| window_pid == pid)
                    .unwrap_or(false)
        }))
}

fn is_launch_candidate(window: &crate::window::WindowMatch) -> bool {
    let normalized = crate::firefox::window::normalize(&window.name);
    !normalized.contains("clipboard") && window.width >= 200 && window.height >= 120
}

fn choose_window_for_context(
    pid: u32,
    baseline: &HashSet<String>,
    title: &str,
) -> Result<Option<crate::window::WindowMatch>> {
    let windows = window::list_firefox_windows(None)?;
    let title = crate::firefox::window::normalize(title);

    Ok(windows
        .into_iter()
        .filter(is_launch_candidate)
        .max_by_key(|window_match| {
            let normalized = crate::firefox::window::normalize(&window_match.name);
            let title_match = (!title.is_empty() && normalized.contains(&title)) as u8;
            let is_new = (!baseline.contains(&window_match.id)) as u8;
            let pid_match = crate::window::window_pid(&window_match.id)
                .map(|window_pid| window_pid == pid)
                .unwrap_or(false) as u8;
            (
                title_match,
                is_new,
                pid_match,
                window_match.width * window_match.height,
            )
        }))
}

fn spawn_browser(
    flavor: BrowserFlavor,
    browser_binary: &str,
    profile_dir: &Path,
    log_path: &Path,
    url: &str,
    bidi_port: u16,
) -> Result<u32> {
    let stdout = File::create(log_path).with_context(|| {
        format!(
            "failed to create firefox launch log at {}",
            log_path.display()
        )
    })?;
    let stderr = stdout.try_clone().with_context(|| {
        format!(
            "failed to clone firefox launch log handle {}",
            log_path.display()
        )
    })?;

    Command::new("setsid")
        .arg("-f")
        .arg(browser_binary)
        .env_remove("NO_AT_BRIDGE")
        .env("MOZ_ACCESSIBILITY_FORCE_DISABLED", "0")
        .env(
            "DISPLAY",
            std::env::var("DISPLAY").unwrap_or_else(|_| ":99".to_string()),
        )
        .arg("--new-window")
        .arg("--new-instance")
        .arg("--profile")
        .arg(profile_dir)
        .arg(format!("--remote-debugging-port={bidi_port}"))
        .arg(url)
        .args(extra_browser_args(flavor))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| format!("failed to detach {browser_binary}"))?;

    find_browser_pid(profile_dir, url, bidi_port)
}

fn find_browser_pid(profile_dir: &Path, url: &str, bidi_port: u16) -> Result<u32> {
    let profile_text = profile_dir.display().to_string();
    let port_flag = format!("--remote-debugging-port={bidi_port}");
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        let output = Command::new("ps").args(["-eo", "pid=,args="]).output()?;
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if !line.contains(&profile_text)
                    || !line.contains(&port_flag)
                    || !line.contains(url)
                {
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
        "failed to resolve launched browser pid for profile {} on {}",
        profile_text,
        port_flag
    ))
}

fn cleanup_existing_profile_processes(profile_dir: &Path) -> Result<()> {
    let profile_text = profile_dir.display().to_string();
    session::clear_browser_session_state();
    let output = Command::new("ps")
        .args(["-eo", "pid=,args="])
        .output()
        .context("failed to inspect existing firefox processes")?;
    if output.status.success() {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if !line.contains(&profile_text) || !line.contains("--remote-debugging-port=") {
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

    Ok(())
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

fn find_browser_binary(flavor: BrowserFlavor) -> Result<String> {
    let candidates = match flavor {
        BrowserFlavor::Firefox => vec![
            "firefox",
            "firefox-esr",
            "/usr/bin/firefox",
            "/usr/bin/firefox-esr",
        ],
        BrowserFlavor::Camoufox => vec![
            "camoufox",
            "/home/ayovps/.local/bin/camoufox",
            "/home/ayovps/.cache/camoufox/camoufox",
        ],
    };

    for candidate in candidates {
        let output = Command::new("sh")
            .args(["-lc", &format!("command -v {}", candidate)])
            .output()
            .with_context(|| format!("failed checking browser binary {candidate}"))?;
        if output.status.success() {
            let resolved = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !resolved.is_empty() {
                return Ok(resolved);
            }
        }
    }

    if matches!(flavor, BrowserFlavor::Camoufox) {
        let output = Command::new("sh")
            .args([
                "-lc",
                "python3 -m camoufox path 2>/dev/null || /home/ayovps/.local/share/axonbrowser/camoufox-venv/bin/python -m camoufox path 2>/dev/null",
            ])
            .output()
            .context("failed checking camoufox path helper")?;
        if output.status.success() {
            let resolved = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !resolved.is_empty() {
                return Ok(resolved);
            }
        }
    }

    Err(anyhow!(
        "could not find a {} binary in PATH",
        flavor.label()
    ))
}

fn prepare_profile_dir(flavor: BrowserFlavor, profile_override: Option<&str>) -> Result<PathBuf> {
    let path = profile_override
        .map(PathBuf::from)
        .unwrap_or_else(|| unique_path(flavor.user_profile_prefix(), "dir"));
    fs::create_dir_all(&path)
        .with_context(|| format!("failed to create firefox profile dir {}", path.display()))?;
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
        || trimmed.starts_with("firefox:")
        || trimmed.starts_with("moz-extension:")
        || trimmed.starts_with("file:")
        || trimmed.starts_with("data:")
    {
        return Ok(trimmed.to_string());
    }
    Ok(format!("https://{}", trimmed))
}

fn extra_browser_args(flavor: BrowserFlavor) -> Vec<&'static str> {
    match flavor {
        // `--remote-allow-system-access` lets WebDriver BiDi evaluate script in
        // privileged contexts such as `about:home`, which a fresh profile opens
        // by default. Without it those contexts reject `document.title`.
        BrowserFlavor::Firefox => vec!["--browser", "--remote-allow-system-access"],
        BrowserFlavor::Camoufox => vec!["--browser", "--remote-allow-system-access"],
    }
}

fn allocate_bidi_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .context("failed to allocate a local Firefox BiDi port")?;
    let port = listener
        .local_addr()
        .context("failed reading allocated Firefox BiDi port")?
        .port();
    drop(listener);
    Ok(port)
}

async fn bidi_ready_on_port(
    bidi_port: u16,
    url: &str,
    last_error: &mut Option<String>,
) -> Result<Option<bidi::ContextInfo>> {
    match bidi::current_context_on_port(bidi_port, Some(url)).await {
        Ok(Some(current)) if !current.url.trim().is_empty() => {
            let _ = session::remember_browser_url(&current.url);
            Ok(Some(current))
        }
        Ok(_) => Ok(None),
        Err(err) => {
            *last_error = Some(err.to_string());
            Ok(None)
        }
    }
}
