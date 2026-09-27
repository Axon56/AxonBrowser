use anyhow::Result;

use crate::window;

use super::{context, focus};

pub async fn press_key(locator_raw: &str, key: &str) -> Result<String> {
    let target = context::resolve_target(locator_raw).await?;
    let focus_summary = focus::focus(locator_raw).await?;
    let browser_window = target.browser_window().await?;
    let activation_note = context::activate_window_note(&browser_window.id);
    window::send_key(&browser_window.id, key)?;
    window::settle_after_input().await;

    Ok(format!(
        "pressed {} on {} in window {} ({}, {}, focus: {})",
        key, target.label, browser_window.id, target.path, activation_note, focus_summary
    ))
}

pub async fn press_enter(locator_raw: Option<&str>) -> Result<String> {
    let target = locator_raw.unwrap_or("address-bar");
    press_key(target, "Return").await
}
