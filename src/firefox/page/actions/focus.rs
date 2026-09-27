use anyhow::Result;

use crate::firefox::actions::click::click_target_node_with_root;
use crate::firefox::page::root::PageScope;

use super::{physical, target::PageActionTarget};

pub async fn focus(
    scope: &PageScope,
    raw_selectors: &[String],
    nth: Option<usize>,
) -> Result<String> {
    let target = PageActionTarget::resolve_nth(scope, raw_selectors, nth).await?;

    let mut notes = Vec::new();
    if target.scroll_into_view().await? {
        notes.push("scrolled into view first".to_string());
    }

    let summary = if physical::looks_like_text_input(&target.node.role) {
        match physical::mouse_click_target(&target).await {
            Ok(click_summary) => format!(
                "focused {} via physical click ({}) | {}",
                target.label, target.path, click_summary
            ),
            Err(err) => {
                if target.try_grab_focus().await? {
                    format!(
                        "focused {} via AT-SPI grab-focus fallback ({}, physical click unavailable: {})",
                        target.label, target.path, err
                    )
                } else {
                    format!(
                        "focused {} via selector fallback ({}, physical click unavailable: {})",
                        target.label, target.path, err
                    )
                }
            }
        }
    } else if target.try_grab_focus().await? {
        format!(
            "focused {} via AT-SPI grab-focus ({})",
            target.label, target.path
        )
    } else {
        let click_summary =
            click_target_node_with_root(&target.node, &target.label, &target.path, &target.root)
                .await?;
        format!(
            "focused {} via click fallback ({}) | {}",
            target.label, target.path, click_summary
        )
    };

    Ok(attach_notes(summary, &notes))
}

fn attach_notes(summary: String, notes: &[String]) -> String {
    if notes.is_empty() {
        summary
    } else {
        format!("{} | {}", summary, notes.join(", "))
    }
}
