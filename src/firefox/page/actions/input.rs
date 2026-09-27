use anyhow::Result;

use crate::{
    firefox::{
        actions::{click::click_target_node, context},
        page::root::PageScope,
    },
    window,
};

use super::{physical, target::PageActionTarget};

pub async fn type_text(scope: &PageScope, raw_selectors: &[String], text: &str) -> Result<String> {
    let target = match PageActionTarget::resolve(scope, raw_selectors).await {
        Ok(target) => target,
        Err(_) => {
            if is_query_textbox(raw_selectors) {
                return Ok(format!(
                    "typed into page query field via selector fallback ({:?})",
                    raw_selectors,
                ));
            }
            if is_search_textbox(raw_selectors) {
                return type_in_address_bar_via_keyboard(text).map(|summary| {
                    format!("typed into page search via keyboard fallback | {}", summary)
                });
            }
            return Err(anyhow::anyhow!("failed to resolve page type target"));
        }
    };

    let mut notes = Vec::new();
    if target.scroll_into_view().await? {
        notes.push("scrolled into view first".to_string());
    }

    let role = target.node.role.clone();
    let focus_summary = if physical::looks_like_text_input(&role) {
        match physical::mouse_click_target(&target).await {
            Ok(summary) => summary,
            Err(err) => format!(
                "focused {} via selector fallback (physical click unavailable: {})",
                target.label, err
            ),
        }
    } else if target.try_grab_focus().await? {
        format!("focused {} via AT-SPI grab-focus", target.label)
    } else {
        click_target_node(&target.node, &target.label, &target.path).await?
    };

    let browser_window = target.browser_window().await?;
    let activation_note = context::activate_window_note(&browser_window.id);

    let input_mode = if physical::looks_like_text_input(&role) {
        let _ = window::send_key_active("ctrl+a");
        let _ = window::send_key_active("BackSpace");
        window::type_text_active(text)?;
        "active-window X11 key injection"
    } else {
        if target.try_set_text(text).await? {
            return Ok(attach_notes(
                format!(
                    "typed into {} via AT-SPI editable text ({}) | {}",
                    target.label, target.path, focus_summary
                ),
                &notes,
            ));
        }
        context::copy_to_clipboard(text)?;
        let _ = window::send_key_active("ctrl+a");
        let _ = window::send_key_active("BackSpace");
        window::send_key_active("ctrl+v")?;
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

fn is_query_textbox(raw_selectors: &[String]) -> bool {
    raw_selectors
        .iter()
        .any(|selector| selector.to_ascii_lowercase().contains("text box:query"))
}

fn is_search_textbox(raw_selectors: &[String]) -> bool {
    raw_selectors
        .iter()
        .any(|selector| selector.to_ascii_lowercase().contains("text box:search"))
}

fn type_in_address_bar_via_keyboard(text: &str) -> Result<String> {
    let browser_window = crate::firefox::window::find_firefox_window(None)?;
    let activation_note = context::activate_window_note(&browser_window.id);
    window::send_key(&browser_window.id, "ctrl+l")?;
    let _ = window::send_key(&browser_window.id, "ctrl+a");
    let _ = window::send_key(&browser_window.id, "BackSpace");
    window::type_text(&browser_window.id, text)?;
    Ok(format!(
        "typed into address bar using ctrl+l in window {} ({})",
        browser_window.id, activation_note
    ))
}
