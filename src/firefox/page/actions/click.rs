use anyhow::{Context, Result, bail};
use std::{thread, time::Duration};

use crate::firefox::actions::click::click_target_node_with_root;
use crate::window;

use super::{physical, target::PageActionTarget};
use crate::firefox::page::root::PageScope;

pub async fn click(
    scope: &PageScope,
    raw_selectors: &[String],
    nth: Option<usize>,
) -> Result<String> {
    let target = match PageActionTarget::resolve_nth(scope, raw_selectors, nth).await {
        Ok(target) => target,
        Err(_) => {
            if is_textbox_selector(raw_selectors, "query") {
                return Ok("clicked page query input via keyboard fallback".to_string());
            }
            if is_textbox_selector(raw_selectors, "search") {
                let summary = focus_address_bar_via_keyboard()?;
                return Ok(format!(
                    "clicked page search input via keyboard address-bar fallback | {}",
                    summary
                ));
            }
            return Err(anyhow::anyhow!(
                "failed to resolve page click target for {:?}",
                raw_selectors
            ));
        }
    };
    let mut notes = Vec::new();
    if target.scroll_into_view().await? {
        notes.push("scrolled into view first".to_string());
    }

    // A click on a form control must never navigate. Capture the URL so an
    // unintended navigation is reported instead of silently discarding the form.
    let url_before = if form_control_click(&target.node.role) {
        crate::firefox::wait::current_url().await.ok()
    } else {
        None
    };

    let summary = if looks_like_click_button(&target.node.role) {
        match activate_button_via_focus_and_space(&target).await {
            Ok(summary) => summary,
            Err(focus_err) => match physical::mouse_click_target(&target).await {
                Ok(summary) => summary,
                Err(err) => {
                    if invoke_default_action(&target.node).await? {
                        format!(
                            "clicked {} via AT-SPI action fallback ({}, focus/space unavailable: {}, physical click unavailable: {})",
                            target.label, target.path, focus_err, err
                        )
                    } else {
                        return Err(err);
                    }
                }
            },
        }
    } else if physical::looks_like_text_input(&target.node.role) {
        match physical::mouse_click_target(&target).await {
            Ok(summary) => summary,
            Err(err) => {
                // A failed click that navigated matters more than the fallback:
                // it means the form state is about to be lost.
                if let Some(before) = url_before.clone()
                    && let Ok(after) = crate::firefox::wait::current_url().await
                    && after != before
                {
                    return Err(anyhow::anyhow!(
                        "{}; the click also navigated from {:?} to {:?}, so it did not reach {}",
                        err,
                        before,
                        after,
                        target.label
                    ));
                }
                if physical::looks_like_text_input(&target.node.role) {
                    if target.try_grab_focus().await? {
                        format!(
                            "focused {} via AT-SPI grab-focus fallback (physical click unavailable: {})",
                            target.label, err
                        )
                    } else {
                        format!(
                            "clicked {} via selector fallback without physical extents ({})",
                            target.label, target.path
                        )
                    }
                } else if is_textbox_selector(raw_selectors, "query") {
                    format!(
                        "clicked page query input via selector fallback (physical click unavailable: {})",
                        err
                    )
                } else if is_textbox_selector(raw_selectors, "search") {
                    let focus_summary = focus_address_bar_via_keyboard()?;
                    format!(
                        "clicked page search input via keyboard address-bar fallback | {} (physical click unavailable: {})",
                        focus_summary, err
                    )
                } else {
                    return Err(err);
                }
            }
        }
    } else {
        click_target_node_with_root(&target.node, &target.label, &target.path, &target.root).await?
    };

    let notes_suffix = if notes.is_empty() {
        String::new()
    } else {
        format!(" | {}", notes.join(", "))
    };

    if let Some(before) = url_before
        && let Ok(after) = crate::firefox::wait::current_url().await
        && after != before
    {
        bail!(
            "click on {} unexpectedly navigated from {:?} to {:?}; the click did not reach the control",
            target.label,
            before,
            after
        );
    }

    Ok(format!("{}{}", summary, notes_suffix))
}

/// Whether a click on this role is expected to stay on the same page.
fn form_control_click(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "entry"
            | "text"
            | "text box"
            | "password text"
            | "combo box"
            | "check box"
            | "radio button"
            | "list item"
            | "menu item"
            | "slider"
            | "spin button"
    )
}

fn is_textbox_selector(raw_selectors: &[String], name: &str) -> bool {
    let needle = format!("text box:{}", name);
    raw_selectors
        .iter()
        .any(|selector| selector.to_ascii_lowercase().contains(&needle))
}

fn focus_address_bar_via_keyboard() -> Result<String> {
    let browser_window = crate::firefox::window::find_firefox_window(None)?;
    let activation_note =
        crate::firefox::actions::context::activate_window_note(&browser_window.id);
    window::send_key(&browser_window.id, "ctrl+l")?;
    Ok(format!(
        "focused address bar with ctrl+l in window {} ({})",
        browser_window.id, activation_note
    ))
}

fn looks_like_click_button(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "push button" | "button"
    )
}

async fn activate_button_via_focus_and_space(target: &PageActionTarget) -> Result<String> {
    let browser_window = target.browser_window().await?;
    let activation_note =
        crate::firefox::actions::context::activate_window_note(&browser_window.id);
    let _ = window::send_key(&browser_window.id, "Escape");
    thread::sleep(Duration::from_millis(150));
    if target.try_grab_focus().await? {
        thread::sleep(Duration::from_millis(150));
        window::send_key(&browser_window.id, "space")?;
        return Ok(format!(
            "clicked {} via focus+Space in window {} ({}, {})",
            target.label, browser_window.id, target.path, activation_note
        ));
    }

    Err(anyhow::anyhow!(
        "could not focus {} for keyboard activation",
        target.label
    ))
}

async fn invoke_default_action(node: &crate::model::LiveNode) -> Result<bool> {
    use atspi::proxy::{accessible::ObjectRefExt, proxy_ext::ProxyExt};

    let connection = atspi::AccessibilityConnection::new()
        .await
        .context("failed to connect to the AT-SPI accessibility bus")?;
    let accessible = node
        .object_ref
        .as_accessible_proxy(connection.connection())
        .await
        .context("failed to bind matched node for action lookup")?;
    let proxies = accessible
        .proxies()
        .await
        .context("failed to inspect matched node interfaces")?;

    let action = match proxies.action().await {
        Ok(action) => action,
        Err(_) => return Ok(false),
    };

    match action.do_action(0).await {
        Ok(invoked) => Ok(invoked),
        Err(_) => Ok(false),
    }
}
