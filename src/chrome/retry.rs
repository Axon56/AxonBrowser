use std::{future::Future, time::Duration};

use anyhow::{Result, anyhow};
use tokio::time::sleep;

const DEFAULT_ATTEMPTS: usize = 6;
const DEFAULT_DELAY_MS: u64 = 150;
/// Cap on the backoff between re-reads. The accessibility tree can come back
/// empty while a page is still settling, and that gap is measured in hundreds of
/// milliseconds, so the wait grows but stays bounded. The cap is deliberately
/// small: these retries absorb a blip, and when the bus is genuinely gone no
/// amount of waiting helps, so a long backoff only makes the command appear to
/// hang before reporting the same failure.
const MAX_DELAY_MS: u64 = 400;

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

                let delay = DEFAULT_DELAY_MS
                    .saturating_mul(1 << (attempt - 1).min(4))
                    .min(MAX_DELAY_MS);
                sleep(Duration::from_millis(delay)).await;
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
        // The accessibility bus resolves each application to a unique name, and
        // that name disappears whenever an application restarts its accessibility
        // bridge -- which a browser does when a page is re-rendered. The call then
        // fails with ServiceUnknown even though the application is fine, so it is
        // a transient condition and must be retried rather than reported.
        "org.freedesktop.dbus.error.serviceunknown",
        "was not provided by any .service files",
        // The unique-name form of the same failure: `the name :1.27 was not
        // provided by any .service files`.
        "org.freedesktop.dbus.error.unknownobject",
        "org.freedesktop.dbus.error.unknownmethod",
        "the name :1.",
        "timed out waiting for reply",
        // One prefix covers every browser: the query text after it changed when
        // the lookup was unified, and matching the old wording meant this retry
        // never ran for the message the code actually produces. A tree that came
        // back empty is the case it exists for, so it must not depend on which
        // browser phrased it.
        "no accessible application or window matched",
        "no visible chrome/chromium window found",
        "no visible microsoft edge window found",
        "no chrome tabs matched",
        "failed to get the at-spi registry root",
        "failed to list desktop applications from the at-spi registry",
        "failed to read children",
        "failed to bind child proxy",
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
    fn classifies_the_unified_browser_query_failure_as_transient() {
        // This is the message the page-root lookup actually produces. Matching
        // only the older per-browser wording left the retry dead for the one
        // failure it exists to absorb.
        assert!(is_transient_accessibility_error(
            "failed to resolve chrome page root; tried Document Web => no accessible application or window matched any browser query"
        ));
    }

    #[test]
    fn classifies_a_dropped_subtree_read_as_transient() {
        assert!(is_transient_accessibility_error(
            "failed to read children for Accessible { name: \"form\" }"
        ));
    }

    #[test]
    fn leaves_normal_selector_failures_alone() {
        assert!(!is_transient_accessibility_error(
            r#"unknown chrome locator "banana""#
        ));
    }
}
