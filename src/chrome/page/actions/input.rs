use anyhow::{Result, bail};

use crate::{
    chrome::{
        actions::{click::click_target_node_with_root, context},
        page::root::PageScope,
    },
    live_access, window,
};

use super::{physical, target::PageActionTarget};

pub async fn type_text(scope: &PageScope, raw_selectors: &[String], text: &str) -> Result<String> {
    let target = PageActionTarget::resolve(scope, raw_selectors).await?;

    let mut notes = Vec::new();
    if target.scroll_into_view().await? {
        notes.push("scrolled into view first".to_string());
    }

    let role = target.node.role.clone();
    let focus_summary = focus_for_typing(&target, &role).await?;

    // AT-SPI editable text needs no coordinates at all, so it is the first
    // choice for inputs and comboboxes.
    if target.try_set_text(text).await? && text_landed(&target, text).await {
        return Ok(attach_notes(
            format!(
                "typed into {} via AT-SPI editable text ({}) | {}",
                target.label, target.path, focus_summary
            ),
            &notes,
        ));
    }

    let browser_window = target.browser_window().await?;
    let activation_note = context::activate_window_note(&browser_window.id);

    let mut input_mode = deliver_text(&browser_window.id, &role, text).await?;
    // Verify the value landed; retry the step once instead of marching on.
    if !text_landed(&target, text).await {
        input_mode = deliver_text(&browser_window.id, &role, text).await?;
        notes.push("retried after the value did not verify".to_string());
    }

    // Do not report success for text that never arrived: a false success here
    // sends the caller on to submit an empty or wrong field.
    if !text_landed(&target, text).await {
        let observed = observed_value(&target).await;
        bail!(
            "typed into {} but the value did not verify after retrying; observed {:?}, expected {:?}",
            target.label,
            observed,
            text
        );
    }

    Ok(attach_notes(
        format!(
            "typed into {} via {} in window {} ({}, {}, focus: {})",
            target.label,
            input_mode,
            browser_window.id,
            target.path,
            activation_note,
            focus_summary
        ),
        &notes,
    ))
}

/// Read back whatever the field currently holds, for error reporting.
async fn observed_value(target: &PageActionTarget) -> String {
    if let Ok(Some(value)) = live_access::read_text(&target.node).await {
        return value;
    }
    crate::inspect::node_text(&target.node)
        .await
        .unwrap_or_else(|| "<unreadable>".to_string())
}

/// Focus a field for typing, asking the accessibility tree first so inputs and
/// comboboxes are reached by keyboard rather than by a coordinate click.
async fn focus_for_typing(target: &PageActionTarget, role: &str) -> Result<String> {
    if target.try_grab_focus().await? {
        return Ok(format!("focused {} via AT-SPI grab-focus", target.label));
    }

    if physical::looks_like_text_input(role)
        && let Ok(summary) = physical::mouse_click_target(target).await
    {
        return Ok(summary);
    }

    // Last resort: the accessibility action interface, which needs no extents.
    click_target_node_with_root(&target.node, &target.label, &target.path, &target.root).await
}

/// Deliver text with the keyboard, returning a label for the mode used.
async fn deliver_text(window_id: &str, role: &str, text: &str) -> Result<&'static str> {
    if physical::looks_like_text_input(role) {
        let _ = window::send_key(window_id, "ctrl+a");
        let _ = window::send_key(window_id, "BackSpace");
        window::type_text(window_id, text)?;
        window::settle_after_input().await;
        Ok("X11 key injection")
    } else {
        context::copy_to_clipboard(text)?;
        let _ = window::send_key(window_id, "ctrl+a");
        let _ = window::send_key(window_id, "BackSpace");
        window::send_key(window_id, "ctrl+v")?;
        window::settle_after_input().await;
        Ok("window-targeted clipboard paste")
    }
}

/// Confirm the typed value actually landed.
async fn text_landed(target: &PageActionTarget, expected: &str) -> bool {
    let expected = expected.trim();
    if expected.is_empty() {
        return true;
    }

    matches!(live_access::read_text(&target.node).await, Ok(Some(value)) if value.contains(expected))
        || matches!(
            crate::inspect::node_text(&target.node).await,
            Some(value) if value.contains(expected)
        )
}

fn attach_notes(summary: String, notes: &[String]) -> String {
    if notes.is_empty() {
        summary
    } else {
        format!("{} | {}", summary, notes.join(", "))
    }
}
