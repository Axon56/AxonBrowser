use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};

const DISPLAY_CANDIDATES: std::ops::RangeInclusive<u16> = 99..=110;
const XVFB_SCREEN: &str = "1440x900x24";
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const WAIT_POLL: Duration = Duration::from_millis(200);
const SESSION_ENV_NAME: &str = "session.env";
const HEADLESS_SESSION_DIR_ENV: &str = "GUIBOT_HEADLESS_SESSION_DIR";
const VNC_SESSION_ENV_ENV: &str = "GUIBOT_VNC_SESSION_ENV";
const HEADLESS_MODE_ENV: &str = "GUIBOT_HEADLESS";
const ATSPI_SOCKET_PATH: &str = "/run/user/1000/at-spi/bus";

pub fn bootstrap_headless_session() -> Result<()> {
    if has_working_display(None) {
        return Ok(());
    }

    let session = match load_persisted_session()? {
        Some(session) => session,
        None => start_session()?,
    };
    apply_env(&session.env);
    Ok(())
}

struct HeadlessSession {
    env: BTreeMap<String, String>,
}

fn start_session() -> Result<HeadlessSession> {
    ensure_commands(["Xvfb", "xdpyinfo", "dbus-launch", "dbus-send"])?;

    // A second session must never be started while a browser is running. The
    // browser is registered with the first session's accessibility bus, so a new
    // session means a new bus, and every lookup on it finds nothing: the window is
    // plainly on screen while the tree comes back empty. Refusing is the only safe
    // answer, and saying why is what makes it fixable.
    if browser_is_running() {
        bail!(
            "a browser is already running but its headless session could not be reused, and starting a second session would leave that browser unregistered with the accessibility bus; restart the browser"
        );
    }

    // A duplicate stack from an earlier session would be joined rather than
    // replaced, so it is collapsed first. This is safe here: no browser is running
    // in this session yet, so nothing is registered with the stack being removed.
    collapse_duplicate_registries();

    let dir = session_dir()?;
    for display_number in DISPLAY_CANDIDATES {
        let display = format!(":{display_number}");
        if display_is_usable(&display) {
            continue;
        }

        reclaim_display(display_number)?;

        let xvfb_log = dir.join(format!("xvfb-{display_number}.log"));

        let xvfb_pid = spawn_logged(
            "Xvfb",
            &[
                &display,
                "-screen",
                "0",
                XVFB_SCREEN,
                "-ac",
                "-nolisten",
                "tcp",
                "-noreset",
            ],
            &xvfb_log,
            None,
        )?;
        write_pid_file(&dir.join("xvfb.pid"), xvfb_pid)?;

        if wait_for_display(&display).is_err() {
            let _ = cleanup_session_outputs(&dir);
            continue;
        }

        let mut env = base_session_env(&display);
        let dbus_env = launch_dbus_session()?;
        env.extend(dbus_env);

        env.insert(HEADLESS_MODE_ENV.to_string(), "1".to_string());
        env.insert(
            HEADLESS_SESSION_DIR_ENV.to_string(),
            dir.display().to_string(),
        );
        env.insert(
            VNC_SESSION_ENV_ENV.to_string(),
            dir.join(SESSION_ENV_NAME).display().to_string(),
        );
        if let Some(runtime_dir) = default_xdg_runtime_dir() {
            env.insert("XDG_RUNTIME_DIR".to_string(), runtime_dir);
        }

        let _ = warm_accessibility_bus(&env, &dir);

        persist_env(&dir.join(SESSION_ENV_NAME), &env)?;

        return Ok(HeadlessSession { env });
    }

    bail!(
        "could not start a usable X11 session in {:?}",
        DISPLAY_CANDIDATES
    )
}

fn load_persisted_session() -> Result<Option<HeadlessSession>> {
    let env_path = session_env_path()?;
    if !env_path.exists() {
        return Ok(None);
    }

    let env = parse_env_file(&env_path)?;
    if env.is_empty() {
        return Ok(None);
    }

    if session_is_usable(&env) {
        return Ok(Some(HeadlessSession { env }));
    }

    cleanup_persisted_session()?;
    Ok(None)
}

fn session_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    let dir = PathBuf::from(home).join(".cache/axonbrowser/headless");
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create session dir {}", dir.display()))?;
    Ok(dir)
}

fn ensure_commands<const N: usize>(commands: [&str; N]) -> Result<()> {
    let missing = commands
        .into_iter()
        .filter(|command| !command_exists(command))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }

    bail!("missing headless runtime commands: {}", missing.join(", "))
}

fn command_exists(command: &str) -> bool {
    Command::new("sh")
        .args(["-lc", &format!("command -v {command}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn has_working_display(env_override: Option<&BTreeMap<String, String>>) -> bool {
    let display = env_override
        .and_then(|env| env.get("DISPLAY"))
        .cloned()
        .or_else(|| std::env::var("DISPLAY").ok());
    let Some(display) = display else {
        return false;
    };

    // Probe more than once. A single `xdpyinfo` can fail spuriously while Xvfb is
    // busy, and this answer decides whether the whole session is torn down: a
    // false negative kills Xvfb, the session bus, and the accessibility bus under
    // a browser that is still running, and that browser never re-registers. The
    // tree then stays empty for the rest of its life. A couple of retries cost
    // milliseconds and remove that failure mode.
    for attempt in 0..3u32 {
        if display_probe_succeeds(&display, env_override) {
            return true;
        }
        if attempt < 2 {
            thread::sleep(Duration::from_millis(200));
        }
    }
    false
}

/// One `xdpyinfo` probe against a display.
fn display_probe_succeeds(display: &str, env_override: Option<&BTreeMap<String, String>>) -> bool {
    let mut command = Command::new("xdpyinfo");
    command.arg("-display").arg(display);
    if let Some(env) = env_override {
        command.env_clear();
        command.envs(env);
        command.env("PATH", std::env::var("PATH").unwrap_or_default());
    }

    command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn session_is_usable(env: &BTreeMap<String, String>) -> bool {
    if !has_working_display(Some(env)) {
        return false;
    }

    if let Some(dir) = env.get(HEADLESS_SESSION_DIR_ENV)
        && !pid_file_is_alive(Path::new(dir).join("xvfb.pid").as_path())
    {
        return false;
    }

    if let Some(pid) = env.get("DBUS_SESSION_BUS_PID") {
        let Ok(pid) = pid.parse::<u32>() else {
            return false;
        };
        if !process_is_alive(pid) {
            return false;
        }
    }

    true
}

fn wait_for_display(display: &str) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < WAIT_TIMEOUT {
        let status = Command::new("xdpyinfo")
            .arg("-display")
            .arg(display)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if status.map(|s| s.success()).unwrap_or(false) {
            return Ok(());
        }
        thread::sleep(WAIT_POLL);
    }

    bail!("timed out waiting for X11 display {display} to become ready")
}

fn display_is_usable(display: &str) -> bool {
    Command::new("xdpyinfo")
        .arg("-display")
        .arg(display)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn reclaim_display(display_number: u16) -> Result<()> {
    let socket = PathBuf::from(format!("/tmp/.X11-unix/X{display_number}"));
    let lock = PathBuf::from(format!("/tmp/.X{display_number}-lock"));
    if socket.exists() {
        let _ = fs::remove_file(&socket);
    }
    if lock.exists() {
        let _ = fs::remove_file(&lock);
    }
    Ok(())
}

fn cleanup_session_outputs(dir: &Path) -> Result<()> {
    // Tearing the session down while a browser is running is never right: the
    // browser would be left on a dead display, holding a connection to a bus that
    // no longer exists, and it never re-registers. The session is kept instead.
    if browser_is_running() {
        eprintln!(
            "keeping the headless session up: a browser is running in it, and restarting the session would orphan that browser"
        );
        return Ok(());
    }

    for name in ["xvfb.pid", "atspi-bus.pid"] {
        let path = dir.join(name);
        if path.exists() {
            let _ = kill_pid_from_file(&path);
            let _ = fs::remove_file(&path);
        }
    }
    let env_path = dir.join(SESSION_ENV_NAME);
    if env_path.exists()
        && let Ok(env) = parse_env_file(&env_path)
        && let Some(pid) = env
            .get("DBUS_SESSION_BUS_PID")
            .and_then(|value| value.parse::<u32>().ok())
    {
        let _ = terminate_pid(pid);
    }
    let _ = terminate_accessibility_processes();
    let _ = fs::remove_file(dir.join(SESSION_ENV_NAME));
    Ok(())
}

fn cleanup_persisted_session() -> Result<()> {
    let dir = session_dir()?;
    cleanup_session_outputs(&dir)
}

fn launch_dbus_session() -> Result<BTreeMap<String, String>> {
    let output = Command::new("dbus-launch")
        .arg("--sh-syntax")
        .output()
        .context("failed to launch dbus session")?;
    if !output.status.success() {
        bail!(
            "dbus-launch failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut env = BTreeMap::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("echo ") {
            continue;
        }
        let Some((key, rest)) = trimmed.split_once('=') else {
            continue;
        };
        let value = rest.split(';').next().unwrap_or_default().trim();
        env.insert(key.to_string(), strip_shell_quotes(value));
    }

    if !env.contains_key("DBUS_SESSION_BUS_ADDRESS") {
        bail!("dbus-launch did not return DBUS_SESSION_BUS_ADDRESS");
    }
    Ok(env)
}

fn strip_shell_quotes(value: &str) -> String {
    value
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .to_string()
}

fn base_session_env(display: &str) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("DISPLAY".to_string(), display.to_string());
    env.insert("XDG_SESSION_TYPE".to_string(), "x11".to_string());
    env.insert("NO_AT_BRIDGE".to_string(), "0".to_string());
    env.insert("GTK_MODULES".to_string(), "gail:atk-bridge".to_string());
    env.insert(
        "QT_LINUX_ACCESSIBILITY_ALWAYS_ON".to_string(),
        "1".to_string(),
    );
    env.insert("ACCESSIBILITY_ENABLED".to_string(), "1".to_string());
    env.insert("GNOME_ACCESSIBILITY".to_string(), "1".to_string());
    if let Ok(home) = std::env::var("HOME") {
        env.insert("HOME".to_string(), home);
    }
    env
}

fn default_xdg_runtime_dir() -> Option<String> {
    std::env::var("XDG_RUNTIME_DIR").ok().or_else(|| {
        let output = Command::new("id").arg("-u").output().ok()?;
        if !output.status.success() {
            return None;
        }
        let uid = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if uid.is_empty() {
            None
        } else {
            Some(format!("/run/user/{uid}"))
        }
    })
}

fn warm_accessibility_bus(env: &BTreeMap<String, String>, dir: &Path) -> Result<()> {
    // This runs while a session is being created, before any browser has been
    // launched, so clearing a stale bus from an earlier session is safe here. It
    // is deliberately not done during repair, where a browser is already running
    // and would be left unregistered.
    let _ = terminate_accessibility_processes();
    enable_accessibility_flags(env)?;

    // Sending to `org.a11y.Bus` activates the bus launcher, which starts a
    // registry of its own, so a second one is only started when nothing appeared.
    ensure_registry_running(env, dir)?;

    wait_for_accessibility_bus(env)
}

pub fn repair_accessibility_stack() -> Result<()> {
    let dir = session_dir()?;
    let mut env = BTreeMap::new();
    for key in [
        "DISPLAY",
        "DBUS_SESSION_BUS_ADDRESS",
        "DBUS_SESSION_BUS_PID",
        "XDG_SESSION_TYPE",
        "NO_AT_BRIDGE",
        "GTK_MODULES",
        "QT_LINUX_ACCESSIBILITY_ALWAYS_ON",
        "ACCESSIBILITY_ENABLED",
        "GNOME_ACCESSIBILITY",
        "HOME",
        "XDG_RUNTIME_DIR",
    ] {
        if let Ok(value) = std::env::var(key) {
            env.insert(key.to_string(), value);
        }
    }
    if !env.contains_key("DISPLAY") || !env.contains_key("DBUS_SESSION_BUS_ADDRESS") {
        bail!("cannot repair AT-SPI stack without DISPLAY and DBUS_SESSION_BUS_ADDRESS");
    }

    // A bus that already answers must be left completely alone. Sending to
    // `org.a11y.Bus` activates the bus launcher when the name is unowned, and a
    // launcher started while a registry is already running produces a second
    // registry that takes the name over -- leaving the browser registered with
    // the first one and every later call unanswered.
    if accessibility_stack_is_up(&env) {
        return Ok(());
    }

    // Repair must never tear the accessibility bus down. This runs when a
    // connection attempt failed, which is usually while a browser is already
    // running and holding its own connection to that bus: killing the bus and the
    // registry under it leaves the browser unregistered, and Chrome does not
    // re-register on its own. The tree then stays empty for the life of that
    // browser, so every later command fails with "no accessible application".
    //
    // Re-asserting the accessibility flags and starting the registry only when it
    // is genuinely absent fixes the usual cause without that side effect.
    enable_accessibility_flags(&env)?;
    if !accessibility_registry_is_running()
        && let Some(registryd) = accessibility_registry_binary()
    {
        let pid = spawn_logged(
            &registryd,
            &["--use-gnome-session"],
            &dir.join("atspi-bus.log"),
            Some(&env),
        )?;
        write_pid_file(&dir.join("atspi-bus.pid"), pid)?;
    }

    wait_for_accessibility_bus_within(&env, REPAIR_WAIT_TIMEOUT)
}

/// Wait for the accessibility bus, giving up after `timeout`.
fn wait_for_accessibility_bus_within(
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if accessibility_bus_is_usable(env) {
            return Ok(());
        }
        thread::sleep(WAIT_POLL);
    }
    bail!("timed out waiting for the AT-SPI accessibility bus to become usable")
}

/// Make sure the accessibility stack is ready for a browser to register with.
///
/// A browser only exposes its tree if it can reach the accessibility bus at the
/// moment it starts, and it never tries again. When the registry happens to be
/// down -- it is started lazily by the session bus, and an earlier session may
/// have left the environment pointing at a dead one -- the browser comes up
/// unregistered and every later page command fails with an empty tree while the
/// window is plainly on screen.
///
/// This is deliberately non-destructive and safe to call before every launch:
/// it re-asserts the accessibility flags and starts the registry only when it is
/// missing. It never stops anything, so a browser that is already running keeps
/// its registration.
pub fn ensure_accessibility_stack() -> Result<()> {
    let dir = session_dir()?;
    let mut env = BTreeMap::new();
    for key in [
        "DISPLAY",
        "DBUS_SESSION_BUS_ADDRESS",
        "DBUS_SESSION_BUS_PID",
        "XDG_SESSION_TYPE",
        "NO_AT_BRIDGE",
        "GTK_MODULES",
        "QT_LINUX_ACCESSIBILITY_ALWAYS_ON",
        "ACCESSIBILITY_ENABLED",
        "GNOME_ACCESSIBILITY",
        "HOME",
        "XDG_RUNTIME_DIR",
    ] {
        if let Ok(value) = std::env::var(key) {
            env.insert(key.to_string(), value);
        }
    }
    if !env.contains_key("DISPLAY") || !env.contains_key("DBUS_SESSION_BUS_ADDRESS") {
        return Ok(());
    }

    // If the bus already answers, do nothing at all. This guard is what keeps the
    // stack from being duplicated: `dbus-send --dest=org.a11y.Bus` *activates* the
    // bus launcher when that name is not currently owned, and a launcher started
    // alongside a registry that is already running produces a second registry
    // that takes over the well-known name. The tool then talks to the new
    // registry while the browser stays registered with the old one, so the tree
    // comes back empty or the call blocks -- for every command afterwards.
    if accessibility_stack_is_up(&env) {
        // A duplicate left behind by an earlier session splits the browser and the
        // tool across two registries, so it is collapsed even though the stack
        // otherwise looks healthy.
        collapse_duplicate_registries();
        return Ok(());
    }

    enable_accessibility_flags(&env)?;
    ensure_registry_running(&env, &dir)?;
    wait_for_accessibility_bus_within(&env, REPAIR_WAIT_TIMEOUT)?;

    Ok(())
}

/// How long to wait for the accessibility bus before giving up.
///
/// A session being created can take a moment to come up, so the initial wait is
/// generous. A repair is different: it runs because a connection already failed,
/// and waiting the full session timeout there turns every failing command into a
/// ten-second stall that ends in the same error. The shorter bound lets the
/// command report the real problem promptly.
const REPAIR_WAIT_TIMEOUT: Duration = Duration::from_secs(2);

/// Whether an accessibility registry daemon is currently running.
fn accessibility_registry_is_running() -> bool {
    Command::new("pgrep")
        .args(["-af", "at-spi2-registryd"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Whether a browser that may be registered with the accessibility bus is up.
///
/// Matched by process name, because the point is to avoid pulling the bus out
/// from under *any* running browser: one that is already up has connected once
/// and will not connect again, whether it was launched by this tool or by hand.
fn browser_is_running() -> bool {
    ["chrome", "chromium", "msedge", "firefox", "camoufox"]
        .iter()
        .any(|name| {
            Command::new("pgrep")
                .args(["-x", name])
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false)
        })
}

/// Wait briefly for a registry to appear, then start one only if none did.
///
/// Enabling the accessibility flags is what makes the session bus activate the
/// bus launcher, and the launcher starts a registry -- but asynchronously. Asking
/// immediately whether a registry is running answers "no" while the real one is
/// still starting, and starting a second one then gives two registries competing
/// for the same well-known name. Whichever the browser did not register with wins
/// some calls, and the tree comes back empty. Waiting first closes that window.
fn ensure_registry_running(env: &BTreeMap<String, String>, dir: &Path) -> Result<()> {
    for _ in 0..20 {
        if accessibility_registry_is_running() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(150));
    }

    // Nothing was activated, so this bus has no at-spi2 service configured and
    // the registry has to be started directly.
    let Some(registryd) = accessibility_registry_binary() else {
        return Ok(());
    };
    let pid = spawn_logged(
        &registryd,
        &["--use-gnome-session"],
        &dir.join("atspi-bus.log"),
        Some(env),
    )?;
    write_pid_file(&dir.join("atspi-bus.pid"), pid)?;
    Ok(())
}

/// Whether the accessibility stack is already up and must be left alone.
///
/// Both halves matter. The registry process must be running, and its socket must
/// exist, because a registry whose socket was removed is not usable.
///
/// The registry is checked before anything is sent to `org.a11y.Bus`, because
/// that message is itself what starts a second bus launcher when the name happens
/// to be unowned. A launcher started alongside a live registry produces a second
/// registry that takes the well-known name over, leaving the browser registered
/// with the first one -- and every later call unanswered or blocked.
fn accessibility_stack_is_up(env: &BTreeMap<String, String>) -> bool {
    if !accessibility_registry_is_running() {
        return false;
    }

    let socket = env
        .get("XDG_RUNTIME_DIR")
        .map(|dir| PathBuf::from(dir).join("at-spi/bus"))
        .unwrap_or_else(|| PathBuf::from(ATSPI_SOCKET_PATH));
    socket.exists()
}

/// Make sure exactly one accessibility registry is serving the session bus.
///
/// Extra registries are worse than none: each one claims the same well-known
/// name, so a browser registers with whichever answered first while the tool may
/// reach another, and the tree then comes back empty or the call blocks. Only the
/// registry the browser is actually talking to is kept, which is the oldest one
/// still running, and any later duplicate is stopped.
fn collapse_duplicate_registries() {
    // Every part of the stack is collapsed together, not just the registry. Each
    // accessibility bus runs its own launcher, its own dbus-daemon, and its own
    // registry; killing only the extra registry leaves the extra launcher alive,
    // and the bus it serves starts a fresh registry on the next request. The
    // oldest instance of each is kept, because that is the stack a browser
    // launched earlier registered with.
    let mut stopped = Vec::new();
    for pattern in [
        "at-spi2-registryd",
        "at-spi-bus-launcher",
        "dbus-daemon --config-file=/usr/share/defaults/at-spi2/accessibility.conf",
    ] {
        let mut pids = pids_by_start_time(pattern);
        if pids.len() < 2 {
            continue;
        }
        stopped.extend(pids.split_off(1));
    }

    if stopped.is_empty() {
        return;
    }
    for pid in &stopped {
        let _ = terminate_pid(*pid);
    }
    // Said out loud because a duplicate stack splits the browser and the tool
    // across two buses, and a caller seeing odd empty-tree failures deserves to
    // know one was found and removed.
    eprintln!(
        "stopped {} duplicate accessibility process(es) ({:?}) so the browser and this tool share one bus",
        stopped.len(),
        stopped
    );
}

/// Pids whose command line matches a pattern, oldest first.
///
/// Start time comes from the proc filesystem, which is what makes "oldest" a fact
/// rather than an assumption: pid order is not start order once pids wrap around,
/// and keeping the wrong instance would break the browser's registration instead
/// of repairing it.
fn pids_by_start_time(pattern: &str) -> Vec<u32> {
    let Ok(output) = Command::new("pgrep").args(["-f", pattern]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let mut pids: Vec<(u64, u32)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .filter_map(|pid| process_start_time(pid).map(|start| (start, pid)))
        .collect();
    pids.sort_unstable();
    pids.into_iter().map(|(_, pid)| pid).collect()
}

/// A process's start time in clock ticks since boot, read from the proc filesystem.
fn process_start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is parenthesised and may itself contain spaces, so the
    // fields are counted from after the closing parenthesis.
    let after_name = stat.rsplit_once(')')?.1;
    after_name.split_whitespace().nth(19)?.parse().ok()
}

/// Turn the accessibility flags on for the current session bus.
///
/// These are what make a newly started browser expose its tree at all, and they
/// are safe to repeat: the call is idempotent and does not disturb anything
/// already connected.
fn enable_accessibility_flags(env: &BTreeMap<String, String>) -> Result<()> {
    for (name, description) in [
        ("IsEnabled", "AT-SPI accessibility"),
        ("ScreenReaderEnabled", "AT-SPI screen reader mode"),
    ] {
        let status = Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.a11y.Bus",
                "--type=method_call",
                "--print-reply",
                "/org/a11y/bus",
                "org.freedesktop.DBus.Properties.Set",
                "string:org.a11y.Status",
                &format!("string:{name}"),
                "variant:boolean:true",
            ])
            .env_clear()
            .envs(env)
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .with_context(|| format!("failed to enable {description} on the session bus"))?;
        if !status.success() {
            bail!("failed to enable {description} on the session bus");
        }
    }
    Ok(())
}

fn accessibility_bus_is_usable(env: &BTreeMap<String, String>) -> bool {
    if let Ok(Some(address)) = query_accessibility_bus_address(env) {
        return ping_dbus_address(env, &address);
    }
    false
}

/// Wait for the accessibility bus during session creation.
fn wait_for_accessibility_bus(env: &BTreeMap<String, String>) -> Result<()> {
    wait_for_accessibility_bus_within(env, WAIT_TIMEOUT)
}

fn query_accessibility_bus_address(env: &BTreeMap<String, String>) -> Result<Option<String>> {
    let output = Command::new("dbus-send")
        .args([
            "--session",
            "--dest=org.a11y.Bus",
            "--type=method_call",
            "--print-reply",
            "/org/a11y/bus",
            "org.a11y.Bus.GetAddress",
        ])
        .env_clear()
        .envs(env)
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .output()
        .context("failed to query AT-SPI bus address")?;
    if !output.status.success() {
        return Ok(None);
    }

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("string \"")
            && let Some(value) = value.strip_suffix('"')
        {
            return Ok(Some(value.to_string()));
        }
    }

    Ok(None)
}

fn ping_dbus_address(env: &BTreeMap<String, String>, address: &str) -> bool {
    Command::new("dbus-send")
        .arg(format!("--bus={address}"))
        .args([
            "--dest=org.freedesktop.DBus",
            "--type=method_call",
            "--print-reply",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.ListNames",
        ])
        .env_clear()
        .envs(env)
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn terminate_accessibility_processes() -> Result<()> {
    // Never take the accessibility stack down while a browser is running. A
    // browser connects to the bus once, at startup, and never re-registers: if
    // the bus or its registry disappears underneath it, the window stays on
    // screen but its tree is gone for the life of that process, and every later
    // command fails with "no accessible application or window matched". Reclaiming
    // a display is not worth orphaning a running browser, so the request is
    // refused and reported instead.
    if browser_is_running() {
        eprintln!(
            "keeping the accessibility bus up: a browser is running and would not re-register if the bus were restarted"
        );
        return Ok(());
    }

    for pattern in [
        "dbus-daemon --config-file=/usr/share/defaults/at-spi2/accessibility.conf",
        "at-spi2-registryd --use-gnome-session",
    ] {
        let output = Command::new("pgrep")
            .args(["-af", pattern])
            .output()
            .with_context(|| format!("failed to inspect accessibility processes for {pattern}"))?;
        if !output.status.success() {
            continue;
        }

        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Some((pid_text, _)) = line.trim().split_once(' ') else {
                continue;
            };
            let Ok(pid) = pid_text.parse::<u32>() else {
                continue;
            };
            let _ = terminate_pid(pid);
        }
    }

    let socket = PathBuf::from(ATSPI_SOCKET_PATH);
    if socket.exists() {
        let _ = fs::remove_file(socket);
    }
    Ok(())
}

fn accessibility_registry_binary() -> Option<String> {
    for candidate in ["/usr/libexec/at-spi2-registryd", "at-spi2-registryd"] {
        if candidate.starts_with('/') {
            if Path::new(candidate).exists() {
                return Some(candidate.to_string());
            }
            continue;
        }
        if command_exists(candidate) {
            return Some(candidate.to_string());
        }
    }
    None
}

fn apply_env(env: &BTreeMap<String, String>) {
    for (key, value) in env {
        unsafe {
            std::env::set_var(key, value);
        }
    }
}

fn spawn_logged(
    command: &str,
    args: &[&str],
    log_path: &Path,
    env_override: Option<&BTreeMap<String, String>>,
) -> Result<u32> {
    let stdout = File::options()
        .create(true)
        .append(true)
        .open(log_path)
        .with_context(|| format!("failed to open log file {}", log_path.display()))?;
    let stderr = stdout.try_clone().with_context(|| {
        format!(
            "failed to clone log handle for background command {}",
            log_path.display()
        )
    })?;

    let mut cmd = Command::new("setsid");
    cmd.arg("-f")
        .arg(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    if let Some(env) = env_override {
        cmd.env_clear();
        cmd.envs(env);
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
    }

    cmd.spawn()
        .with_context(|| format!("failed to detach background command {command}"))?;

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        if let Some(pid) = find_background_pid(command, args, env_override)? {
            return Ok(pid);
        }
        thread::sleep(Duration::from_millis(100));
    }

    bail!("failed to resolve detached pid for {command}")
}

fn find_background_pid(
    command: &str,
    args: &[&str],
    env_override: Option<&BTreeMap<String, String>>,
) -> Result<Option<u32>> {
    let output = Command::new("pgrep")
        .arg("-af")
        .arg(command)
        .output()
        .with_context(|| format!("failed to query detached pid for {command}"))?;
    if !output.status.success() {
        return Ok(None);
    }

    let display_filter = env_override.and_then(|env| env.get("DISPLAY")).cloned();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        let Some((pid_text, cmdline)) = trimmed.split_once(' ') else {
            continue;
        };
        if !args.iter().all(|arg| cmdline.contains(arg)) {
            continue;
        }
        if let Some(display) = display_filter.as_deref()
            && !cmdline.contains(display)
        {
            continue;
        }
        if let Ok(pid) = pid_text.parse::<u32>() {
            return Ok(Some(pid));
        }
    }

    Ok(None)
}

fn persist_env(path: &Path, env: &BTreeMap<String, String>) -> Result<()> {
    let mut file = File::create(path)
        .with_context(|| format!("failed to create session env file {}", path.display()))?;
    for (key, value) in env {
        writeln!(file, "{key}={value}")
            .with_context(|| format!("failed writing session env file {}", path.display()))?;
    }
    Ok(())
}

fn parse_env_file(path: &Path) -> Result<BTreeMap<String, String>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read session env file {}", path.display()))?;
    let mut env = BTreeMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        env.insert(key.trim().to_string(), value.trim().to_string());
    }
    Ok(env)
}

fn write_pid_file(path: &Path, pid: u32) -> Result<()> {
    fs::write(path, pid.to_string())
        .with_context(|| format!("failed to write pid file {}", path.display()))
}

fn session_env_path() -> Result<PathBuf> {
    Ok(session_dir()?.join(SESSION_ENV_NAME))
}

fn pid_file_is_alive(path: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(pid) = raw.trim().parse::<u32>() else {
        return false;
    };
    process_is_alive(pid)
}

fn process_is_alive(pid: u32) -> bool {
    Command::new("sh")
        .args(["-lc", &format!("kill -0 {pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn kill_pid_from_file(path: &Path) -> Result<()> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read pid file {}", path.display()))?;
    let pid = raw
        .trim()
        .parse::<u32>()
        .with_context(|| format!("invalid pid in {}", path.display()))?;
    terminate_pid(pid)?;
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
        if !process_is_alive(pid) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Ok(())
}
