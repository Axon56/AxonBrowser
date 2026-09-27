use anyhow::{Result, bail};

use crate::chrome::actions::click::click_target_node_with_root;

use super::{physical, target::PageActionTarget};
use crate::chrome::page::root::PageScope;

pub async fn click(
    scope: &PageScope,
    raw_selectors: &[String],
    nth: Option<usize>,
) -> Result<String> {
    let target = PageActionTarget::resolve_nth(scope, raw_selectors, nth).await?;
    let mut notes = Vec::new();
    if target.scroll_into_view().await? {
        notes.push("scrolled into view first".to_string());
    }

    // A click on a form control must never navigate. Capture the URL so an
    // unintended navigation is reported instead of silently discarding the form.
    let url_before = if form_control_click(&target.node.role) {
        crate::chrome::wait::current_url().await.ok()
    } else {
        None
    };

    let summary = if physical::looks_like_text_input(&target.node.role) {
        // A refused or ineffective click must still report any navigation it
        // caused: a misclick that changes the page destroys the form, and that
        // is the most important thing for the caller to know.
        match physical::mouse_click_target(&target).await {
            Ok(summary) => summary,
            Err(err) => {
                return Err(describe_with_navigation(err, url_before, target.label.as_str()).await);
            }
        }
    } else {
        click_target_node_with_root(&target.node, &target.label, &target.path, &target.root).await?
    };

    if let Some(before) = url_before
        && let Ok(after) = crate::chrome::wait::current_url().await
        && after != before
    {
        bail!(
            "click on {} unexpectedly navigated from {:?} to {:?}; the click did not reach the control",
            target.label,
            before,
            after
        );
    }

    if notes.is_empty() {
        Ok(summary)
    } else {
        Ok(format!("{} | {}", summary, notes.join(", ")))
    }
}

/// Enrich a click failure with any navigation it caused.
async fn describe_with_navigation(
    err: anyhow::Error,
    url_before: Option<String>,
    label: &str,
) -> anyhow::Error {
    let Some(before) = url_before else {
        return err;
    };
    match crate::chrome::wait::current_url().await {
        Ok(after) if after != before => anyhow::anyhow!(
            "{}; the click also navigated from {:?} to {:?}, so it did not reach {}",
            err,
            before,
            after,
            label
        ),
        _ => err,
    }
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
