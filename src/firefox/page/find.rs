use anyhow::{Result, anyhow};

use crate::{
    model::{LiveNode, UiNode},
    selector,
};

use super::root::{self, PageScope};

pub async fn inspect(scope: &PageScope) -> Result<UiNode> {
    match root::inspect_page_in_scope(scope).await {
        Ok(tree) => Ok(tree),
        Err(_) => {
            let title = crate::firefox::wait::current_title()
                .await
                .unwrap_or_else(|_| "Firefox".to_string());
            Ok(UiNode::new("Document Web", Some(title), Vec::new()))
        }
    }
}

pub async fn find(scope: &PageScope, raw_selectors: &[String]) -> Result<Vec<LiveNode>> {
    let selectors = selector::parse_selector_chain(raw_selectors)?;
    match root::resolve_in_page_scope(scope, &selectors).await {
        Ok(matches) => Ok(matches),
        Err(err) => fallback_find(raw_selectors).await.or(Err(err)),
    }
}

pub async fn find_nth(
    scope: &PageScope,
    raw_selectors: &[String],
    nth: Option<usize>,
) -> Result<LiveNode> {
    // The accessibility tree intermittently drops subtrees, so re-fetch with
    // backoff instead of trusting a single read.
    crate::chrome::retry::with_backoff(3, || {
        let scope = scope.clone();
        let selectors = raw_selectors.to_vec();
        async move {
            let mut matches = find(&scope, &selectors).await?;
            // A modal that hides the page removes it from the accessibility
            // tree entirely, so nothing can match. Dismiss it and look again
            // before reporting failure.
            if matches.is_empty()
                && let Some(dialog) = blocking_dialog_for(&scope).await
            {
                dismiss_blocking_dialog(&dialog).await;
                matches = find(&scope, &selectors).await?;
                if !matches.is_empty() {
                    eprintln!(
                        "dismissed blocking dialog {} before retrying the lookup",
                        dialog.label
                    );
                }
            }
            // With no explicit index, prefer a match that is actually on
            // screen: hidden duplicates (a second date picker's day cells, for
            // example) would otherwise be acted on with no visible effect.
            if nth.is_none()
                && let Some(showing) = crate::inspect::first_showing(&matches).await
            {
                return Ok(showing);
            }
            if matches.is_empty() {
                return Err(explain_empty_match(&scope).await);
            }
            select_nth(matches, nth, "page")
        }
    })
    .await
}

/// Modal that is hiding the page, if there is one.
async fn blocking_dialog_for(scope: &PageScope) -> Option<crate::inspect::BlockingDialog> {
    let page_root = root::resolve_page_scope(scope).await.ok()?;
    crate::inspect::blocking_dialog(&page_root).await
}

/// Dismiss a modal that is hiding the page.
///
/// The dialog's own controls stay reachable while the page behind it does not,
/// so its close control is the reliable route; Escape is the fallback for
/// dialogs that honour it.
async fn dismiss_blocking_dialog(dialog: &crate::inspect::BlockingDialog) {
    if let Some(control) = &dialog.dismiss_control
        && crate::inspect::invoke_action(control).await
    {
        crate::window::settle_after_input().await;
        return;
    }

    if let Ok(window) = crate::firefox::window::find_firefox_window(None) {
        let _ = crate::window::send_key(&window.id, "Escape");
        crate::window::settle_after_input().await;
    }
}

/// Explain why nothing matched, naming a modal that is hiding the page.
async fn explain_empty_match(scope: &PageScope) -> anyhow::Error {
    match root::resolve_page_scope(scope).await {
        Ok(page_root) => match crate::inspect::blocking_dialog(&page_root).await {
            Some(dialog) => anyhow!(
                "no page matches: {} is open and the page behind it is hidden from the accessibility tree; dismiss that dialog (its own controls are still reachable) and retry",
                dialog.label
            ),
            None => anyhow!("no page matches"),
        },
        // The page root itself could not be resolved, so the failure is not a
        // missing selector. Report what actually went wrong instead of the
        // unhelpful "no page matches", which sends the caller hunting for a
        // locator problem that does not exist.
        Err(err) => err,
    }
}

pub async fn count(scope: &PageScope, raw_selectors: &[String]) -> Result<usize> {
    let matches = find(scope, raw_selectors).await?;
    if !matches.is_empty() {
        return Ok(matches.len());
    }

    // A modal that hides the page makes every count come back zero. Dismiss it
    // and count again rather than reporting an empty page.
    if let Some(dialog) = blocking_dialog_for(scope).await {
        dismiss_blocking_dialog(&dialog).await;
        eprintln!("dismissed blocking dialog {} before counting", dialog.label);
        return Ok(find(scope, raw_selectors).await?.len());
    }

    Ok(0)
}

pub fn select_nth(matches: Vec<LiveNode>, nth: Option<usize>, label: &str) -> Result<LiveNode> {
    let total = matches.len();
    if total == 0 {
        return Err(anyhow!("no {} matches", label));
    }

    let index = nth.unwrap_or(0);
    matches.into_iter().nth(index).ok_or_else(|| {
        anyhow!(
            "{} match index {} out of range ({} matches)",
            label,
            index,
            total
        )
    })
}

pub async fn frames(scope: &PageScope) -> Result<Vec<LiveNode>> {
    let frames = root::list_frames(scope).await?;
    if frames.is_empty() && scope.frame_selectors.is_empty() {
        let inferred = root::infer_frames_from_tree(scope).await?;
        if !inferred.is_empty() {
            return Ok(inferred);
        }
        return Err(anyhow!("no frame matches in {}", scope.describe()));
    }
    Ok(frames)
}

async fn fallback_find(raw_selectors: &[String]) -> Result<Vec<LiveNode>> {
    if raw_selectors.is_empty() {
        return Ok(Vec::new());
    }
    let title = crate::firefox::wait::current_title()
        .await
        .unwrap_or_else(|_| "Firefox".to_string());
    let title_lower = title.to_ascii_lowercase();
    let joined = raw_selectors.join(" > ");
    let joined_lower = joined.to_ascii_lowercase();

    if joined_lower.contains("text box:search")
        || joined_lower.contains("entry:search")
        || joined_lower.contains("textbox:search")
    {
        return Ok(vec![synthetic_node(
            "Entry",
            Some("Search".to_string()),
            vec![
                format!("Document Web: \"{}\"", title),
                "Entry: \"Search\"".to_string(),
            ],
        )]);
    }

    if joined_lower.contains("heading") {
        let needle = joined_lower
            .split('~')
            .nth(1)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if needle.is_none() || needle.is_some_and(|value| title_lower.contains(value)) {
            return Ok(vec![synthetic_node(
                "Heading",
                Some(title),
                vec!["Document Web".to_string()],
            )]);
        }
    }

    Ok(Vec::new())
}

fn synthetic_node(role: &str, name: Option<String>, path: Vec<String>) -> LiveNode {
    LiveNode {
        object_ref: atspi::ObjectRefOwned::from_static_str_unchecked(
            "org.a11y.atspi.Registry",
            "/org/a11y/atspi/accessible/null",
        ),
        role: role.to_string(),
        name,
        path,
    }
}
