use anyhow::{Result, bail};

use crate::window;

use super::{context, focus};

pub async fn type_text(locator_raw: &str, text: &str) -> Result<String> {
    let target = context::resolve_target(locator_raw).await?;
    let focus_summary = focus::focus(locator_raw).await?;
    if target.try_set_text(text).await? {
        return Ok(format!(
            "typed into {} via AT-SPI editable text ({}) | {}",
            target.label, target.path, focus_summary
        ));
    }

    let browser_window = target.browser_window().await?;
    let activation_note = context::activate_window_note(&browser_window.id);
    context::type_via_clipboard(&browser_window.id, text)?;
    window::settle_after_input().await;
    if target.is_address_bar() {
        let observed = context::read_address_bar_via_clipboard(&browser_window.id)?;
        if observed.trim() != text.trim() {
            bail!(
                "address bar write verification failed: expected {:?}, observed {:?}",
                text,
                observed
            );
        }
    }
    Ok(format!(
        "typed into {} via {} in window {} ({}, {}, focus: {})",
        target.label,
        "clipboard paste",
        browser_window.id,
        target.path,
        activation_note,
        focus_summary
    ))
}
