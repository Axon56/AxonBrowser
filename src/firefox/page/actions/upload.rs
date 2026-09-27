use std::{path::Path, time::Duration};

use anyhow::{Result, anyhow, bail};

use crate::{firefox::actions::context, firefox::page::root::PageScope, window};

use super::{physical, target::PageActionTarget};

pub async fn upload(scope: &PageScope, raw_selectors: &[String], path: &str) -> Result<String> {
    let expanded = std::fs::canonicalize(Path::new(path))
        .map_err(|err| anyhow!("failed to resolve upload path {:?}: {}", path, err))?;
    if !expanded.is_file() {
        bail!("upload path is not a file: {}", expanded.display());
    }

    let target = PageActionTarget::resolve(scope, raw_selectors).await?;
    let mut notes = Vec::new();
    if target.scroll_into_view().await? {
        notes.push("scrolled into view first".to_string());
    }

    let focus_summary = if physical::looks_like_text_input(&target.node.role) {
        physical::mouse_click_target(&target).await?
    } else if target.try_grab_focus().await? {
        format!("focused {} via AT-SPI grab-focus", target.label)
    } else {
        crate::firefox::actions::click::click_target_node_with_root(
            &target.node,
            &target.label,
            &target.path,
            &target.root,
        )
        .await?
    };

    if target.try_set_text(&expanded.to_string_lossy()).await? {
        return Ok(attach_notes(
            format!(
                "uploaded {} via AT-SPI editable text on {} ({}) | {}",
                expanded.display(),
                target.label,
                target.path,
                focus_summary
            ),
            &notes,
        ));
    }

    let browser_window = target.browser_window().await?;
    let activation_note = context::activate_window_note(&browser_window.id);
    let input_mode = if target.node.role.eq_ignore_ascii_case("push button") {
        crate::firefox::actions::click::click_target_node(
            &target.node,
            &target.label,
            &target.path,
        )
        .await?;
        std::thread::sleep(Duration::from_millis(500));
        let chooser_window = window::find_window_by_title_contains("Open File").or_else(|_| {
            window::active_window_id().and_then(|id| {
                window::list_visible_windows()?
                    .into_iter()
                    .find(|candidate| candidate.id == id)
                    .ok_or_else(|| anyhow!("active chooser window {} not found", id))
            })
        })?;
        let _ = window::activate_window(&chooser_window.id);
        std::thread::sleep(Duration::from_millis(300));
        window::type_text_active(&expanded.to_string_lossy())?;
        window::send_key_active("Return")?;
        format!("native chooser typing via window {}", chooser_window.id)
    } else {
        context::copy_to_clipboard(&expanded.to_string_lossy())?;
        window::send_key_active("ctrl+l")?;
        window::send_key_active("ctrl+v")?;
        window::send_key_active("Return")?;
        "focused file chooser fallback".to_string()
    };

    Ok(attach_notes(
        format!(
            "uploaded {} via {} in window {} ({}, {}, focus: {})",
            expanded.display(),
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
