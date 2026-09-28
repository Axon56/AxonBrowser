//! Detecting and dismissing a modal that is covering the page.
//!
//! A page that opens a modal usually marks the content behind it `aria-hidden`
//! or `inert`, and the browser then removes that content from the accessibility
//! tree, so the tree contains only the dialog and every page lookup fails.
//! Clicking "through" the backdrop happens to work on some sites but dies on
//! any site that traps pointer events or focus, so the dialog is dismissed
//! first and the action retried.

use crate::inspect::BlockingDialog;
use crate::{dom, inspect, window};

/// Modal that is currently hiding the page, if there is one.
pub async fn blocking_dialog() -> Option<BlockingDialog> {
    let root = match dom::current_flavor() {
        dom::Flavor::Firefox => crate::firefox::page::root::resolve_page_scope(&Default::default())
            .await
            .ok()?,
        _ => crate::chrome::page::root::resolve_page_scope(&Default::default())
            .await
            .ok()?,
    };
    inspect::blocking_dialog(&root).await
}

/// Dismiss a modal that is covering the page.
///
/// The dialog's own close control is used because it stays reachable while the
/// page behind it does not; Escape is the fallback for dialogs that honour it.
/// Success is reported only once the page confirms the cover is gone: an
/// injected click or key that did nothing must not be reported as a dismissal,
/// because the caller then acts on a page that is still blocked.
pub async fn dismiss(dialog: &BlockingDialog) -> bool {
    if let Some(control) = &dialog.dismiss_control
        && inspect::invoke_action(control).await
    {
        window::settle_after_input().await;
        if crate::dom::covering_overlay().await.is_none() {
            return true;
        }
    }

    let browser_window = match dom::current_flavor() {
        dom::Flavor::Firefox => crate::firefox::window::find_firefox_window(None).ok(),
        _ => crate::chrome::window::find_browser_window(None).ok(),
    };
    if let Some(browser_window) = browser_window {
        let _ = window::send_key(&browser_window.id, "Escape");
        window::settle_after_input().await;
        return crate::dom::covering_overlay().await.is_none();
    }

    false
}

/// Dismiss a modal that is covering the page, if any, and report its label.
pub async fn dismiss_if_present() -> Option<String> {
    let dialog = blocking_dialog().await?;
    if dismiss(&dialog).await {
        Some(dialog.label)
    } else {
        None
    }
}
