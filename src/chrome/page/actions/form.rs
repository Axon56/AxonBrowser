use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use atspi::State;
use tokio::time::sleep;

use crate::{
    chrome::{actions::click::click_target_node_with_root, page::root::PageScope},
    model::LiveNode,
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
    nth: Option<usize>,
) -> Result<String> {
    // `--nth` addresses the control, not the option: the common need is the
    // second combo box on a form, and a custom dropdown's control selector is
    // what matches several of them.
    let target = PageActionTarget::resolve_nth(scope, raw_selectors, nth).await?;
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
            crate::chrome::actions::context::activate_window_note(&browser_window.id);
        window::send_key(&browser_window.id, "Return")?;
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
    let mut open_summary =
        click_target_node_with_root(&target.node, &target.label, &target.path, &target.root)
            .await?;
    // The keyboard attempt above can leave the list toggled shut, so if the
    // options are not there, open the control again and look a second time
    // before giving up.
    let option_node = match resolve_option(scope, option).await {
        Ok(node) => node,
        Err(first_err) => {
            let reopened = click_target_node_with_root(
                &target.node,
                &target.label,
                &target.path,
                &target.root,
            )
            .await;
            match reopened {
                Ok(summary) => {
                    open_summary = summary;
                    resolve_option(scope, option).await.map_err(|_| first_err)?
                }
                Err(_) => return Err(first_err),
            }
        }
    };
    let option_label = option_node.line_label();
    let option_path = option_node.path.join(" > ");
    let select_summary =
        click_target_node_with_root(&option_node, &option_label, &option_path, &target.root)
            .await?;

    // Re-resolve the control before verifying: a native select can expose a
    // stale placeholder through AT-SPI while its value has already changed, so
    // reading the pre-action node would report a false failure. `--nth` is
    // applied again so the control that was acted on is the one verified; without
    // it the first combo box on the page was checked instead, and a successful
    // selection on the second one was reported as unverified.
    let verified = match PageActionTarget::resolve_nth(scope, raw_selectors, nth).await {
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

/// Find the option to click.
///
/// Dropdowns render their choices with different accessibility roles: a native
/// select exposes `Menu Item`, while a custom autocomplete usually exposes
/// `List Item`. Matching only one role made `select-option` report "no page
/// option matched" on dropdowns whose options were list items, so the roles are
/// tried in turn and the first with a match wins.
///
/// The first match is always taken. `--nth` selects which *control* to act on,
/// because the common need is the second combo box on a form; applying it to the
/// option instead made a control's own options unreachable and, on a dropdown
/// with one matching choice, failed outright. When a dropdown really does render
/// the same label twice, the first is the one the user sees at the top.
async fn resolve_option(scope: &PageScope, option: &str) -> Result<LiveNode> {
    const OPTION_ROLES: &[&str] = &["List Item", "Menu Item", "Option", "Table Cell"];

    let mut failures = Vec::new();
    for role in OPTION_ROLES {
        let raw = format!("{}~{}", role, option);
        match crate::chrome::page::root::resolve_in_page_scope(
            scope,
            &[selector::Selector::parse(&raw)?],
        )
        .await
        {
            Ok(matches) if !matches.is_empty() => {
                return Ok(matches.into_iter().next().expect("non-empty result"));
            }
            Ok(_) => failures.push(format!("{role} => no matches")),
            Err(err) => failures.push(format!("{role} => {err}")),
        }
    }

    Err(anyhow!(
        "no page option matched {:?}; tried {}",
        option,
        failures.join("; ")
    ))
}

/// Confirm the control now reports the option as its value.
async fn option_selected(target: &PageActionTarget, option: &str) -> bool {
    let option = option.trim().to_ascii_lowercase();
    if option.is_empty() {
        return true;
    }

    // A custom dropdown usually reports its current value as the control's own
    // accessible name once the list closes, so check that before anything else.
    if target
        .node
        .name
        .as_deref()
        .map(|name| name.to_ascii_lowercase().contains(&option))
        .unwrap_or(false)
    {
        return true;
    }

    // The page is the source of truth for a control's value. A custom dropdown
    // keeps its chosen label as plain text, and a native select can report a
    // stale placeholder over AT-SPI, so both made a real selection look
    // unverified and left the caller unable to tell success from failure.
    if let Some(value) = crate::dom::control_value(&target.node).await
        && value.to_ascii_lowercase().contains(&option)
    {
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
