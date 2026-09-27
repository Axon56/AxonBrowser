use anyhow::{Result, anyhow};

use crate::{firefox::actions::context, inspect, window};

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
    let (relative_x, relative_y, dismissed) =
        crate::overlay::guarded_click_point(target, &browser_window).await?;
    let screen_x = browser_window.x + relative_x;
    let screen_y = browser_window.y + relative_y;
    let activation_note = context::activate_window_note(&browser_window.id);
    window::mousemove_click_absolute_button(screen_x, screen_y, button, repeat)?;

    let click_kind = match (button, repeat) {
        (1, 2) => "double-clicked".to_string(),
        (3, _) => "right-clicked".to_string(),
        _ => "clicked".to_string(),
    };

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
