use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use atspi::State;
use tokio::time::sleep;

use crate::{
    firefox::{actions::click::click_target_node_with_root, page::root::PageScope},
    selector, window,
};

use super::target::PageActionTarget;

pub async fn check(scope: &PageScope, raw_selectors: &[String]) -> Result<String> {
    set_toggle(scope, raw_selectors, true).await
}

pub async fn uncheck(scope: &PageScope, raw_selectors: &[String]) -> Result<String> {
    set_toggle(scope, raw_selectors, false).await
}

pub async fn select_option(
    scope: &PageScope,
    raw_selectors: &[String],
    option: &str,
) -> Result<String> {
    let target = PageActionTarget::resolve(scope, raw_selectors).await?;
    let mut notes = Vec::new();
    if target.scroll_into_view().await? {
        notes.push("scrolled into view first".to_string());
    }

    // Keyboard-first, but only for controls that accept typed text: custom
    // comboboxes and autocompletes usually expose no press action, while a
    // native select exposes a menu that the click path below already handles.
    if target.try_grab_focus().await? && target.try_set_text(option).await? {
        let browser_window = target.browser_window().await?;
        let activation_note =
            crate::firefox::actions::context::activate_window_note(&browser_window.id);
        window::send_key_active("Return")?;
        window::settle_after_input().await;

        if option_selected(&target, option).await {
            return Ok(attach_notes(
                format!(
                    "selected option {:?} via keyboard entry in window {} ({})",
                    option, browser_window.id, activation_note
                ),
                &notes,
            ));
        }
        notes.push("keyboard entry did not verify; fell back to the option list".to_string());
    }

    // Fall back to opening the control and clicking the option, then verify.
    let open_summary =
        click_target_node_with_root(&target.node, &target.label, &target.path, &target.root)
            .await?;
    let option_selector = selector::Selector::parse(&format!("~{}", option))?;
    let option_node = crate::firefox::page::root::resolve_in_page_scope(scope, &[option_selector])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no page option matched {:?}", option))?;
    let option_label = option_node.line_label();
    let option_path = option_node.path.join(" > ");
    let select_summary =
        click_target_node_with_root(&option_node, &option_label, &option_path, &target.root)
            .await?;

    // Re-resolve the control before verifying: a native select can expose a
    // stale placeholder through AT-SPI while its value has already changed, so
    // reading the pre-action node would report a false failure.
    let verified = match PageActionTarget::resolve(scope, raw_selectors).await {
        Ok(fresh) => option_selected(&fresh, option).await,
        Err(_) => false,
    };

    let outcome = if verified {
        format!(
            "selected option {:?} via {} | {}",
            option, open_summary, select_summary
        )
    } else {
        // Say so plainly instead of reporting a failure inside a success line.
        format!(
            "selected option {:?} via {} | {} | selection could not be verified: the control does not expose its selected value",
            option, open_summary, select_summary
        )
    };

    Ok(attach_notes(outcome, &notes))
}

/// Confirm the control now reports the option as its value.
async fn option_selected(target: &PageActionTarget, option: &str) -> bool {
    let option = option.trim().to_ascii_lowercase();
    if option.is_empty() {
        return true;
    }

    if let Ok(Some(value)) = crate::live_access::read_text(&target.node).await
        && value.to_ascii_lowercase().contains(&option)
    {
        return true;
    }

    crate::inspect::descendant_option_selected(&target.node, &option).await
}

async fn set_toggle(
    scope: &PageScope,
    raw_selectors: &[String],
    desired_checked: bool,
) -> Result<String> {
    let target = PageActionTarget::resolve(scope, raw_selectors).await?;
    let mut notes = Vec::new();
    if target.scroll_into_view().await? {
        notes.push("scrolled into view first".to_string());
    }

    let states = target.state_set().await?;
    let currently_checked = is_checked(states);
    if currently_checked == desired_checked {
        let state = if desired_checked {
            "already checked"
        } else {
            "already unchecked"
        };
        return Ok(attach_notes(
            format!("{} {} ({})", state, target.label, target.path),
            &notes,
        ));
    }

    let action_summary =
        click_target_node_with_root(&target.node, &target.label, &target.path, &target.root)
            .await?;
    if !wait_for_checked_state(scope, raw_selectors, desired_checked).await? {
        bail!(
            "toggle state for {} did not change to {}",
            target.label,
            if desired_checked {
                "checked"
            } else {
                "unchecked"
            }
        );
    }

    Ok(attach_notes(
        format!(
            "set {} {} ({}) | {}",
            if desired_checked {
                "checked"
            } else {
                "unchecked"
            },
            target.label,
            target.path,
            action_summary
        ),
        &notes,
    ))
}

fn is_checked(states: atspi::StateSet) -> bool {
    states.contains(State::Checked)
        || states.contains(State::Selected)
        || states.contains(State::Pressed)
}

async fn wait_for_checked_state(
    scope: &PageScope,
    raw_selectors: &[String],
    desired_checked: bool,
) -> Result<bool> {
    for _ in 0..15 {
        let target = PageActionTarget::resolve(scope, raw_selectors).await?;
        if is_checked(target.state_set().await?) == desired_checked {
            return Ok(true);
        }
        sleep(Duration::from_millis(100)).await;
    }
    Ok(false)
}

fn attach_notes(summary: String, notes: &[String]) -> String {
    if notes.is_empty() {
        summary
    } else {
        format!("{} | {}", summary, notes.join(", "))
    }
}
