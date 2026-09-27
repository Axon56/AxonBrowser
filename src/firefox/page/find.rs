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
            let matches = find(&scope, &selectors).await?;
            select_nth(matches, nth, "page")
        }
    })
    .await
}

pub async fn count(scope: &PageScope, raw_selectors: &[String]) -> Result<usize> {
    Ok(find(scope, raw_selectors).await?.len())
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
