use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};

const DEFAULT_TYPE_DELAY_MS: &str = "120";

#[derive(Debug, Clone)]
pub struct WindowMatch {
    pub id: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub fn active_window_id() -> Result<String> {
    let output = Command::new("xdotool")
        .arg("getactivewindow")
        .output()
        .context("failed to query the active X11 window")?;

    if !output.status.success() {
        if let Some(window) = list_visible_windows()?.into_iter().next() {
            return Ok(window.id);
        }
        return Err(anyhow!(
            "xdotool getactivewindow failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if id.is_empty() {
        bail!("xdotool getactivewindow returned an empty window id");
    }
    Ok(id)
}

pub fn find_window_at_point(x: i32, y: i32) -> Result<WindowMatch> {
    let mut best: Option<WindowMatch> = None;
    for candidate in list_visible_windows()? {
        let right = candidate.x + i32::try_from(candidate.width).unwrap_or(i32::MAX);
        let bottom = candidate.y + i32::try_from(candidate.height).unwrap_or(i32::MAX);
        if x < candidate.x || y < candidate.y || x >= right || y >= bottom {
            continue;
        }

        if best
            .as_ref()
            .map(|current| area(&candidate) < area(current))
            .unwrap_or(true)
        {
            best = Some(candidate);
        }
    }

    best.ok_or_else(|| anyhow!("no visible X11 window contains point ({x},{y})"))
}

pub fn find_window_by_title_contains(query: &str) -> Result<WindowMatch> {
    let needle = normalize_text(query);
    let mut matches = list_visible_windows()?
        .into_iter()
        .filter(|window| normalize_text(&window.name).contains(&needle))
        .collect::<Vec<_>>();
    matches.sort_by_key(area);
    matches
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no visible X11 window title contains {:?}", query))
}

pub fn window_pid(window_id: &str) -> Result<u32> {
    let output = Command::new("xprop")
        .args(["-id", window_id, "_NET_WM_PID"])
        .output()
        .with_context(|| format!("failed to query X11 window pid for {window_id}"))?;

    if !output.status.success() {
        return Err(anyhow!(
            "xprop _NET_WM_PID failed for {}: {}",
            window_id,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pid = stdout
        .split('=')
        .nth(1)
        .map(str::trim)
        .and_then(|raw| raw.parse::<u32>().ok())
        .ok_or_else(|| anyhow!("xprop _NET_WM_PID returned no pid for {}", window_id))?;

    Ok(pid)
}

pub fn pid_cmdline(pid: u32) -> Result<String> {
    let path = format!("/proc/{pid}/cmdline");
    let bytes = std::fs::read(&path).with_context(|| format!("failed to read {path}"))?;
    if bytes.is_empty() {
        bail!("empty cmdline for pid {}", pid);
    }

    Ok(bytes
        .split(|byte| *byte == 0)
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| String::from_utf8_lossy(chunk).to_string())
        .collect::<Vec<_>>()
        .join(" "))
}

pub fn activate_window(id: &str) -> Result<()> {
    let output = Command::new("xdotool")
        .args(["windowactivate", "--sync", id])
        .output()
        .with_context(|| format!("failed to activate X11 window {id}"))?;

    if !output.status.success() {
        return Err(anyhow!(
            "xdotool windowactivate failed for {}: {}",
            id,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

pub fn mousemove(window_id: &str, x: i32, y: i32) -> Result<()> {
    run_xdotool_owned(
        vec![
            "mousemove".into(),
            "--window".into(),
            window_id.into(),
            x.to_string(),
            y.to_string(),
        ],
        format!("failed to move mouse inside X11 window {window_id}"),
        format!("xdotool mousemove failed for {}", window_id),
    )
}

pub fn mousemove_click(window_id: &str, x: i32, y: i32) -> Result<()> {
    mousemove_click_button(window_id, x, y, 1, 1)
}

pub fn mousemove_click_button(
    window_id: &str,
    x: i32,
    y: i32,
    button: u8,
    repeat: u8,
) -> Result<()> {
    let mut args = vec![
        "mousemove".into(),
        "--window".into(),
        window_id.into(),
        x.to_string(),
        y.to_string(),
        "click".into(),
    ];
    if repeat > 1 {
        args.push("--repeat".into());
        args.push(repeat.to_string());
    }
    args.push(button.to_string());

    run_xdotool_owned(
        args,
        format!("failed to click inside X11 window {window_id}"),
        format!("xdotool mousemove/click failed for {}", window_id),
    )
}

pub fn mousemove_click_absolute_button(x: i32, y: i32, button: u8, repeat: u8) -> Result<()> {
    let mut args = vec![
        "mousemove".into(),
        x.to_string(),
        y.to_string(),
        "click".into(),
    ];
    if repeat > 1 {
        args.push("--repeat".into());
        args.push(repeat.to_string());
    }
    args.push(button.to_string());

    run_xdotool_owned(
        args,
        format!("failed to click at absolute screen point {x},{y}"),
        format!("xdotool absolute mousemove/click failed at {x},{y}"),
    )
}

pub fn type_text(window_id: &str, text: &str) -> Result<()> {
    let output = Command::new("xdotool")
        .args([
            "type",
            "--window",
            window_id,
            "--delay",
            DEFAULT_TYPE_DELAY_MS,
            text,
        ])
        .output()
        .with_context(|| format!("failed to type into X11 window {window_id}"))?;

    if !output.status.success() {
        return Err(anyhow!(
            "xdotool type failed for {}: {}",
            window_id,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

pub fn type_text_active(text: &str) -> Result<()> {
    run_xdotool(
        ["type", "--delay", DEFAULT_TYPE_DELAY_MS, text],
        "failed to type into the active X11 window".to_string(),
        "xdotool type failed for active window".to_string(),
    )
}

pub fn send_key(window_id: &str, key: &str) -> Result<()> {
    let output = Command::new("xdotool")
        .args(["key", "--window", window_id, "--clearmodifiers", key])
        .output()
        .with_context(|| format!("failed to send key {key:?} to X11 window {window_id}"))?;

    if !output.status.success() {
        return Err(anyhow!(
            "xdotool key failed for {}: {}",
            window_id,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

pub fn send_key_active(key: &str) -> Result<()> {
    run_xdotool(
        ["key", "--clearmodifiers", key],
        format!("failed to send key {key:?} to the active X11 window"),
        format!("xdotool key failed for active window using {key:?}"),
    )
}

pub fn key_down(window_id: &str, key: &str) -> Result<()> {
    run_xdotool(
        ["keydown", "--window", window_id, "--clearmodifiers", key],
        format!("failed to hold key {key:?} on X11 window {window_id}"),
        format!("xdotool keydown failed for {}", window_id),
    )
}

pub fn key_up(window_id: &str, key: &str) -> Result<()> {
    run_xdotool(
        ["keyup", "--window", window_id, "--clearmodifiers", key],
        format!("failed to release key {key:?} on X11 window {window_id}"),
        format!("xdotool keyup failed for {}", window_id),
    )
}

pub fn scroll(window_id: &str, direction: ScrollDirection, amount: u32) -> Result<()> {
    let button = direction.button_code();
    let repeats = amount.max(1).to_string();
    run_xdotool_owned(
        vec![
            "click".into(),
            "--window".into(),
            window_id.into(),
            "--repeat".into(),
            repeats,
            button.to_string(),
        ],
        format!("failed to scroll X11 window {window_id}"),
        format!("xdotool scroll failed for {}", window_id),
    )
}

pub fn resize_window(window_id: &str, width: u32, height: u32) -> Result<()> {
    // Resize through xdotool so no window manager or wmctrl is required: the
    // headless Xvfb session axonbrowser bootstraps has neither, and wmctrl
    // cannot drive windows without an EWMH-compliant window manager.
    if !list_visible_windows()?
        .into_iter()
        .any(|candidate| candidate.id == window_id)
    {
        return Err(anyhow!("window {} is not visible for resize", window_id));
    }

    run_xdotool_owned(
        vec![
            "windowsize".into(),
            window_id.into(),
            width.to_string(),
            height.to_string(),
        ],
        format!("failed to resize X11 window {window_id}"),
        format!("xdotool windowsize failed for {window_id}"),
    )
}

pub fn list_visible_windows() -> Result<Vec<WindowMatch>> {
    if let Ok(windows) = list_visible_windows_via_wmctrl()
        && !windows.is_empty()
    {
        return Ok(windows);
    }

    list_visible_windows_via_xwininfo()
}

fn list_visible_windows_via_wmctrl() -> Result<Vec<WindowMatch>> {
    let output = Command::new("wmctrl")
        .arg("-lpG")
        .output()
        .context("failed to run wmctrl -lpG")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        if trimmed.contains("_NET_CLIENT_LIST") || trimmed.contains("_WIN_CLIENT_LIST") {
            return Ok(Vec::new());
        }
        return Err(anyhow!("wmctrl -lpG failed: {}", trimmed));
    }

    let mut matches = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let window = match parse_wmctrl_line(line) {
            Some(window) => window,
            None => continue,
        };
        if window.width == 0 || window.height == 0 {
            continue;
        }
        matches.push(window);
    }

    Ok(matches)
}

fn list_visible_windows_via_xwininfo() -> Result<Vec<WindowMatch>> {
    let output = Command::new("xwininfo")
        .args(["-root", "-tree", "-int"])
        .output()
        .context("failed to run xwininfo -root -tree -int")?;

    if !output.status.success() {
        return Err(anyhow!(
            "xwininfo -root -tree -int failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let mut matches = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some(window) = parse_xwininfo_line(line) else {
            continue;
        };
        if window.width == 0 || window.height == 0 {
            continue;
        }
        matches.push(window);
    }
    Ok(matches)
}

#[derive(Debug, Clone, Copy)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

impl ScrollDirection {
    fn button_code(self) -> u8 {
        match self {
            Self::Up => 4,
            Self::Down => 5,
            Self::Left => 6,
            Self::Right => 7,
        }
    }
}

fn parse_wmctrl_line(line: &str) -> Option<WindowMatch> {
    let mut parts = line.split_whitespace();
    let id_hex = parts.next()?;
    let _desktop = parts.next()?;
    let _pid = parts.next()?;
    let x = parts.next()?.parse::<i32>().ok()?;
    let y = parts.next()?.parse::<i32>().ok()?;
    let width = parts.next()?.parse::<u32>().ok()?;
    let height = parts.next()?.parse::<u32>().ok()?;
    let _host = parts.next()?;
    let title = parts.collect::<Vec<_>>().join(" ");
    if title.trim().is_empty() {
        return None;
    }

    let id = if let Some(stripped) = id_hex.strip_prefix("0x") {
        u64::from_str_radix(stripped, 16).ok()?.to_string()
    } else {
        id_hex.to_string()
    };

    Some(WindowMatch {
        id,
        name: title,
        x,
        y,
        width,
        height,
    })
}

fn parse_xwininfo_line(line: &str) -> Option<WindowMatch> {
    let trimmed = line.trim();
    if !(trimmed.starts_with("0x") || trimmed.chars().next()?.is_ascii_digit()) {
        return None;
    }

    let (id_hex, rest) = trimmed.split_once(' ')?;
    let id = if let Some(stripped) = id_hex.strip_prefix("0x") {
        u64::from_str_radix(stripped, 16).ok()?.to_string()
    } else {
        id_hex.parse::<u64>().ok()?.to_string()
    };

    let name_start = rest.find('"')?;
    let name_end = rest[name_start + 1..].find('"')?;
    let name = rest[name_start + 1..name_start + 1 + name_end].to_string();
    if name.trim().is_empty() {
        return None;
    }

    let geometry = rest.split_whitespace().find(|token| {
        let bytes = token.as_bytes();
        bytes
            .iter()
            .position(|byte| *byte == b'x')
            .is_some_and(|x_index| {
                x_index > 0
                    && bytes
                        .iter()
                        .skip(x_index + 1)
                        .filter(|byte| **byte == b'+')
                        .count()
                        >= 2
            })
    })?;
    let (width, rest) = geometry.split_once('x')?;
    let (height, pos) = rest.split_once('+')?;
    let (x, y) = pos.split_once('+')?;

    Some(WindowMatch {
        id,
        name,
        x: x.parse().ok()?,
        y: y.parse().ok()?,
        width: width.parse().ok()?,
        height: height.parse().ok()?,
    })
}

fn run_xdotool<const N: usize>(
    args: [&str; N],
    context_message: String,
    failure_prefix: String,
) -> Result<()> {
    let output = Command::new("xdotool")
        .args(args)
        .output()
        .with_context(|| context_message)?;

    if !output.status.success() {
        return Err(anyhow!(
            "{}: {}",
            failure_prefix,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

fn run_xdotool_owned(
    args: Vec<String>,
    context_message: String,
    failure_prefix: String,
) -> Result<()> {
    let output = Command::new("xdotool")
        .args(&args)
        .output()
        .with_context(|| context_message)?;

    if !output.status.success() {
        return Err(anyhow!(
            "{}: {}",
            failure_prefix,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

fn area(window: &WindowMatch) -> u64 {
    u64::from(window.width) * u64::from(window.height)
}

fn normalize_text(input: &str) -> String {
    input
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
