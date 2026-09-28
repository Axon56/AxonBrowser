use anyhow::{Result, anyhow};

use crate::{chrome::actions::context, inspect, window};

use super::target::PageActionTarget;

pub async fn mouse_click_target(target: &PageActionTarget) -> Result<String> {
    mouse_click_target_button(target, 1, 1).await
}

pub async fn mouse_click_target_button(
    target: &PageActionTarget,
    button: u8,
    repeat: u8,
) -> Result<String> {
    let browser_window = target.browser_window().await?;

    // Whether the field already had focus decides whether observing focus after
    // the click proves anything, so the state is captured before the point is
    // resolved: resolving can nudge the page, and a click is only evidence when
    // focus was not already there.
    let was_focused_before = crate::inspect::read_state_set(&target.node)
        .await
        .map(|states| states.contains(atspi::State::Focused))
        .unwrap_or(false);
    // A combo box is confirmed by its list opening, not by focus: a custom one is
    // usually a `div` that never takes keyboard focus, so requiring focus would
    // report a click that worked as a misclick. The visible lists are recorded
    // now so the same set can be compared after the click.
    let lists_before = if crate::overlay::is_dropdown_role(&target.node.role) {
        crate::dom::visible_option_lists().await
    } else {
        None
    };
    // A dropdown that is already open closes on the next click, so the list is
    // only required to appear when it was not showing before.
    let was_expanded_before = if lists_before.is_some() {
        crate::dom::control_expanded(&target.node).await
    } else {
        None
    };
    let (relative_x, relative_y, dismissed) =
        crate::overlay::guarded_click_point(target, &browser_window).await?;
    let activation_note = context::activate_window_note(&browser_window.id);
    window::mousemove_click_button(&browser_window.id, relative_x, relative_y, button, repeat)?;

    let click_kind = match (button, repeat) {
        (1, 2) => "double-clicked".to_string(),
        (3, _) => "right-clicked".to_string(),
        _ => "clicked".to_string(),
    };

    // A plain left click on an editable field must leave it focused; otherwise
    // the click landed somewhere else and must not be reported as a success.
    // A combo box is checked by whether its list opened instead, because focus is
    // not something a custom dropdown reliably reports.
    if button == 1 && repeat == 1 && lists_before.is_some() && was_expanded_before != Some(true) {
        if !crate::overlay::dropdown_opened(&target.node, lists_before.as_deref()).await {
            anyhow::bail!(
                "click on {} at {},{} did not open its list, so the click landed on something else",
                target.label,
                relative_x,
                relative_y
            );
        }
    } else if button == 1 && repeat == 1 && looks_like_text_input(&target.node.role) {
        let outcome =
            crate::overlay::verify_text_input_focus_after(&target.node, was_focused_before).await?;
        if outcome == crate::overlay::FocusOutcome::AlreadyFocused {
            // Focus was already on the field before the click, so focus is not
            // independent evidence that the click arrived. The point itself was
            // confirmed against the page before the click was sent, so that is
            // what the summary reports instead of implying a stronger check.
            let mut summary = format!(
                "sent a {} to {} via X11 at {},{} in window {} ({}, {}) | the point was confirmed to belong to the field, but it already had focus so focus is not evidence the click arrived",
                click_kind.trim_end_matches("ed"),
                target.label,
                relative_x,
                relative_y,
                browser_window.id,
                target.path,
                activation_note
            );
            if let Some(overlay) = dismissed {
                summary = format!("{summary} | dismissed overlay {overlay} before clicking");
            }
            return Ok(summary);
        }
    }

    let mut summary = format!(
        "{} {} via X11 at {},{} in window {} ({}, {})",
        click_kind,
        target.label,
        relative_x,
        relative_y,
        browser_window.id,
        target.path,
        activation_note
    );
    if let Some(overlay) = dismissed {
        summary = format!(
            "{} | dismissed overlay {} before clicking",
            summary, overlay
        );
    }

    Ok(summary)
}

pub async fn window_relative_point(
    target: &PageActionTarget,
) -> Result<(window::WindowMatch, i32, i32)> {
    let (screen_x, screen_y) = inspect::clickable_point_stable(&target.node).await?;
    let browser_window = window::find_window_at_point(screen_x, screen_y)?;
    let relative_x = screen_x - browser_window.x;
    let relative_y = screen_y - browser_window.y;
    if relative_x < 0 || relative_y < 0 {
        return Err(anyhow!(
            "resolved click point landed outside the target window"
        ));
    }
    Ok((browser_window, relative_x, relative_y))
}

pub fn looks_like_text_input(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "entry" | "password text" | "text" | "text box" | "combo box"
    )
}
