use anyhow::Result;

use crate::{
    chrome::{
        actions::{click::click_target_node, context},
        page::root::PageScope,
    },
    window,
};

use super::{physical, target::PageActionTarget};

pub async fn type_text(scope: &PageScope, raw_selectors: &[String], text: &str) -> Result<String> {
    let target = PageActionTarget::resolve(scope, raw_selectors).await?;

    let mut notes = Vec::new();
    if target.scroll_into_view().await? {
        notes.push("scrolled into view first".to_string());
    }

    let role = target.node.role.clone();
    let focus_summary = if physical::looks_like_text_input(&role) {
        physical::mouse_click_target(&target).await?
    } else if target.try_grab_focus().await? {
        format!("focused {} via AT-SPI grab-focus", target.label)
    } else {
        click_target_node(&target.node, &target.label, &target.path).await?
    };

    if target.try_set_text(text).await? {
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

    let input_mode = if physical::looks_like_text_input(&role) {
        let _ = window::send_key(&browser_window.id, "ctrl+a");
        let _ = window::send_key(&browser_window.id, "BackSpace");
        window::type_text(&browser_window.id, text)?;
        "X11 key injection"
    } else {
        context::copy_to_clipboard(text)?;
        let _ = window::send_key(&browser_window.id, "ctrl+a");
        let _ = window::send_key(&browser_window.id, "BackSpace");
        window::send_key(&browser_window.id, "ctrl+v")?;
        "window-targeted clipboard paste"
    };
    window::settle_after_input().await;

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

fn attach_notes(summary: String, notes: &[String]) -> String {
    if notes.is_empty() {
        summary
    } else {
        format!("{} | {}", summary, notes.join(", "))
    }
}
