use std::{future::Future, time::Duration};

use anyhow::{Result, anyhow};
use tokio::time::sleep;

const DEFAULT_ATTEMPTS: usize = 6;
const DEFAULT_DELAY_MS: u64 = 150;

pub async fn with_transient_retry<T, F, Fut>(mut op: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempt = 1usize;

    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                let message = err.to_string();
                if attempt >= DEFAULT_ATTEMPTS || !is_transient_accessibility_error(&message) {
                    return Err(err);
                }

                sleep(Duration::from_millis(DEFAULT_DELAY_MS)).await;
                attempt += 1;
            }
        }
    }
}

/// Retry an operation with exponential backoff.
///
/// The accessibility tree intermittently returns errors or an incomplete
/// subtree, so a single read is not trustworthy. Re-fetch with backoff instead
/// of failing the step.
pub async fn with_backoff<T, F, Fut>(attempts: usize, mut op: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let attempts = attempts.max(1);
    let mut delay = Duration::from_millis(100);
    let mut last_error = None;

    for attempt in 0..attempts {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) => last_error = Some(err),
        }
        if attempt + 1 < attempts {
            sleep(delay).await;
            delay = delay.saturating_mul(2);
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("operation failed after {attempts} attempts")))
}

pub fn is_transient_accessibility_error(message: &str) -> bool {
    let normalized = message.trim().to_ascii_lowercase();
    [
        "failed to connect to the at-spi accessibility bus",
        "org.freedesktop.dbus.error.noreply",
        "org.freedesktop.dbus.error.disconnected",
        "org.freedesktop.dbus.error.servicename",
        "the name :1.",
        "timed out waiting for reply",
        r#"no accessible application or window matched query "chrome""#,
        "no accessible application or window matched any edge query",
        "no accessible application or window matched any firefox query",
        "no visible chrome/chromium window found",
        "no visible microsoft edge window found",
        "no chrome tabs matched",
        "failed to get the at-spi registry root",
        "failed to list desktop applications from the at-spi registry",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::is_transient_accessibility_error;

    #[test]
    fn classifies_noreply_as_transient() {
        assert!(is_transient_accessibility_error(
            "org.freedesktop.DBus.Error.NoReply: Message recipient disconnected from message bus"
        ));
    }

    #[test]
    fn classifies_missing_chrome_tree_as_transient() {
        assert!(is_transient_accessibility_error(
            r#"no accessible application or window matched query "chrome""#
        ));
    }

    #[test]
    fn classifies_missing_edge_tree_as_transient() {
        assert!(is_transient_accessibility_error(
            "no accessible application or window matched any edge query"
        ));
    }

    #[test]
    fn classifies_missing_firefox_tree_as_transient() {
        assert!(is_transient_accessibility_error(
            "no accessible application or window matched any firefox query"
        ));
    }

    #[test]
    fn leaves_normal_selector_failures_alone() {
        assert!(!is_transient_accessibility_error(
            r#"unknown chrome locator "banana""#
        ));
    }
}
