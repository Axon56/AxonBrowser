use anyhow::{Context, Result};

use crate::{
    chrome::{
        actions::{click::click_target_node_with_root, context},
        page::root::PageScope,
        window as chrome_window,
    },
    window,
};

use super::{focus, target::PageActionTarget};

pub async fn press_key(scope: &PageScope, raw_selectors: &[String], key: &str) -> Result<String> {
    let target = PageActionTarget::resolve(scope, raw_selectors).await?;
    let mut notes = Vec::new();
    if target.scroll_into_view().await? {
        notes.push("scrolled into view first".to_string());
    }
    let focus_summary = if target.try_grab_focus().await? {
        format!("focused {} via AT-SPI grab-focus", target.label)
    } else {
        click_target_node_with_root(&target.node, &target.label, &target.path, &target.root).await?
    };

    let browser_window = target.browser_window().await?;
    let activation_note = context::activate_window_note(&browser_window.id);
    window::send_key(&browser_window.id, key)?;
    window::settle_after_input().await;

    Ok(attach_notes(
        format!(
            "pressed {} on {} in window {} ({}, {}, focus: {})",
            key, target.label, browser_window.id, target.path, activation_note, focus_summary
        ),
        &notes,
    ))
}

pub async fn press_enter(scope: &PageScope, raw_selectors: &[String]) -> Result<String> {
    if raw_selectors.is_empty() {
        return press_enter_active_window("page press-enter with no selectors").await;
    }

    let target = PageActionTarget::resolve(scope, raw_selectors).await?;
    if target.node.role.eq_ignore_ascii_case("push button") {
        let click_summary =
            click_target_node_with_root(&target.node, &target.label, &target.path, &target.root)
                .await?;
        return Ok(format!(
            "activated {} via click fallback ({}) | {}",
            target.label, target.path, click_summary
        ));
    }

    let focus_summary = focus::focus(scope, raw_selectors, None).await?;
    let enter_summary =
        press_enter_active_window("page press-enter after focusing selector").await?;
    Ok(format!("{} | {}", focus_summary, enter_summary))
}

async fn press_enter_active_window(context_label: &str) -> Result<String> {
    if let Ok(window_match) = chrome_window::find_browser_window(None) {
        let activation_note = context::activate_window_note(&window_match.id);
        window::send_key(&window_match.id, "Return").with_context(|| {
            format!(
                "{} and could not send Return to browser window {}",
                context_label, window_match.id
            )
        })?;
        window::settle_after_input().await;
        return Ok(format!(
            "pressed Return via browser-window fallback {} ({})",
            window_match.id, activation_note
        ));
    }

    let active_id = window::active_window_id().ok();
    window::send_key_active("Return").with_context(|| {
        format!(
            "{} and could not send Return to the active window",
            context_label
        )
    })?;
    window::settle_after_input().await;

    Ok(match active_id {
        Some(id) => format!("pressed Return via active-window hard fallback ({})", id),
        None => "pressed Return via active-window hard fallback".to_string(),
    })
}

fn attach_notes(summary: String, notes: &[String]) -> String {
    if notes.is_empty() {
        summary
    } else {
        format!("{} | {}", summary, notes.join(", "))
    }
}
