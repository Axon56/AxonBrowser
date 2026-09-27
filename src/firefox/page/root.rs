use anyhow::{Result, anyhow};

use crate::{
    chrome::retry,
    inspect,
    model::{LiveNode, UiNode},
    selector, window,
};

const PAGE_ROOT_CHAINS: &[&[&str]] = &[&["Document Web"], &["Root Web Area"]];
const FRAME_ROLE_CANDIDATES: &[&str] = &["Frame", "Internal Frame"];
const BROWSER_QUERY: &str = "firefox";
const FALLBACK_BROWSER_QUERIES: &[&str] = &["mozilla firefox", "firefox"];

#[derive(Debug, Clone, Default)]
pub struct PageScope {
    pub frame_selectors: Vec<selector::Selector>,
}

impl PageScope {
    pub fn from_raw(frame_selectors: &[String]) -> Result<Self> {
        Ok(Self {
            frame_selectors: selector::parse_selector_chain(frame_selectors)?,
        })
    }

    pub fn describe(&self) -> String {
        if self.frame_selectors.is_empty() {
            "top page".to_string()
        } else {
            format!(
                "frame {}",
                self.frame_selectors
                    .iter()
                    .map(describe_selector)
                    .collect::<Vec<_>>()
                    .join(" > ")
            )
        }
    }
}

pub async fn inspect_page_in_scope(scope: &PageScope) -> Result<UiNode> {
    let root = resolve_page_scope(scope).await?;
    inspect::inspect_live(&root).await
}

pub async fn resolve_in_page_scope(
    scope: &PageScope,
    selectors: &[selector::Selector],
) -> Result<Vec<LiveNode>> {
    let root = resolve_page_scope(scope).await?;
    inspect::resolve_within_scope(&root, selectors).await
}

pub async fn resolve_first_in_page_scope(
    scope: &PageScope,
    selectors: &[selector::Selector],
) -> Result<LiveNode> {
    resolve_in_page_scope(scope, selectors)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no page matches in {}", scope.describe()))
}

pub async fn list_frames(scope: &PageScope) -> Result<Vec<LiveNode>> {
    let root = resolve_page_scope(scope).await?;
    inspect::resolve_within_scope(&root, &frame_role_selectors()).await
}

pub async fn infer_frames_from_tree(scope: &PageScope) -> Result<Vec<LiveNode>> {
    let tree = inspect_page_in_scope(scope).await?;
    let mut paths = Vec::new();
    collect_frame_paths(&tree, &mut Vec::new(), &mut paths);
    let mut matches = Vec::new();
    for path in paths {
        let selectors = path_to_selector_chain(&path)?;
        if let Ok(node) = resolve_first_in_page_scope(scope, &selectors).await {
            matches.push(node);
        }
    }
    Ok(matches)
}

pub async fn resolve_page_scope(scope: &PageScope) -> Result<LiveNode> {
    retry::with_transient_retry(|| {
        let scope = scope.clone();
        async move { resolve_page_scope_once(&scope).await }
    })
    .await
}

async fn resolve_page_scope_once(scope: &PageScope) -> Result<LiveNode> {
    let root = resolve_page_root_once().await?;
    if scope.frame_selectors.is_empty() {
        return Ok(root);
    }

    inspect::resolve_within_scope(&root, &scope.frame_selectors)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no frame matches for {}", scope.describe()))
}

async fn resolve_page_root_once() -> Result<LiveNode> {
    let mut failures = Vec::new();

    for chain in PAGE_ROOT_CHAINS {
        match resolve_chain(chain).await {
            Ok(nodes) if !nodes.is_empty() => {
                return Ok(outermost_document(nodes));
            }
            Ok(_) => failures.push(format!("{} => no matches", format_chain(chain))),
            Err(err) => failures.push(format!("{} => {}", format_chain(chain), err)),
        }
    }

    Err(anyhow!(
        "failed to resolve firefox page root; tried {}",
        failures.join("; ")
    ))
}

/// Pick the outermost matching document as the page root.
///
/// A page that embeds an iframe exposes both the top-level document and the
/// frame's document as `Document Web` nodes. Taking the first match can select
/// the nested document, which makes every page command silently target the
/// iframe instead of the page, so prefer the shallowest candidate.
fn outermost_document(mut nodes: Vec<LiveNode>) -> LiveNode {
    nodes.sort_by_key(|node| node.path.len());
    nodes.into_iter().next().expect("non-empty result")
}

async fn resolve_chain(chain: &[&str]) -> Result<Vec<LiveNode>> {
    let raw = chain
        .iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>();
    let selectors = selector::parse_selector_chain(&raw)?;
    let mut last_nodes = Vec::new();
    for query in browser_root_queries() {
        if let Ok(nodes) = inspect::resolve(&query, &selectors).await
            && !nodes.is_empty()
        {
            last_nodes = nodes.clone();
            let scoped = scope_nodes_to_browser_window(nodes).await?;
            if !scoped.is_empty() {
                return Ok(scoped);
            }
        }
    }
    if !last_nodes.is_empty() {
        return Ok(last_nodes);
    }
    Err(anyhow!(
        "no accessible application or window matched any browser query"
    ))
}

fn browser_root_queries() -> Vec<String> {
    let mut queries = Vec::new();
    if let Ok(window) = crate::firefox::window::find_firefox_window(None) {
        queries.push(window.name);
    }
    queries.push(BROWSER_QUERY.to_string());
    queries.extend(FALLBACK_BROWSER_QUERIES.iter().map(|q| q.to_string()));
    queries
}

async fn scope_nodes_to_browser_window(nodes: Vec<LiveNode>) -> Result<Vec<LiveNode>> {
    let Some(target_window) = crate::firefox::window::find_firefox_window(None).ok() else {
        return Ok(nodes);
    };

    let path_filtered = nodes
        .iter()
        .filter(|node| node_belongs_to_window_title(node, &target_window.name))
        .cloned()
        .collect::<Vec<_>>();
    if !path_filtered.is_empty() {
        return Ok(path_filtered);
    }

    let mut filtered = Vec::new();
    for node in nodes.iter().cloned() {
        let extents = match inspect::component_extents(&node).await {
            Ok(extents) => extents,
            Err(_) => continue,
        };
        let (x, y, width, height) = extents;
        if width <= 0 || height <= 0 {
            continue;
        }
        let screen_x = x + (width / 2);
        let screen_y = y + (height / 2);
        let matched_window = match window::find_window_at_point(screen_x, screen_y) {
            Ok(found) => found,
            Err(_) => continue,
        };
        if matched_window.id == target_window.id {
            filtered.push(node);
        }
    }

    if filtered.is_empty() {
        Ok(nodes)
    } else {
        Ok(filtered)
    }
}

fn node_belongs_to_window_title(node: &LiveNode, window_title: &str) -> bool {
    let needle = crate::firefox::window::normalize(window_title);
    node.path.iter().any(|segment| {
        let hay = crate::firefox::window::normalize(segment);
        hay.contains(&needle) || needle.contains(&hay)
    })
}

fn frame_role_selectors() -> Vec<selector::Selector> {
    FRAME_ROLE_CANDIDATES
        .iter()
        .map(|role| selector::Selector::parse(role).expect("static frame selector is valid"))
        .collect()
}

fn collect_frame_paths(node: &UiNode, prefix: &mut Vec<String>, paths: &mut Vec<Vec<String>>) {
    prefix.push(node.line_label());
    let role = node.role.trim().to_ascii_lowercase();
    if role == "internal frame" || role == "frame" {
        paths.push(prefix.clone());
    }
    for child in &node.children {
        collect_frame_paths(child, prefix, paths);
    }
    prefix.pop();
}

fn path_to_selector_chain(path: &[String]) -> Result<Vec<selector::Selector>> {
    path.iter()
        .filter_map(|segment| {
            let (role, name) = segment.split_once(':')?;
            let normalized_role = role.trim().to_ascii_lowercase();
            if normalized_role != "internal frame" && normalized_role != "frame" {
                return None;
            }
            let clean_name = name.trim().trim_matches('"');
            Some(selector::Selector::parse(&format!(
                "{}:{}",
                role.trim(),
                clean_name
            )))
        })
        .collect()
}

fn describe_selector(selector: &selector::Selector) -> String {
    match (&selector.role, &selector.name) {
        (Some(role), Some(selector::NameMatch::Exact(name))) => format!("{}:{}", role, name),
        (Some(role), Some(selector::NameMatch::Contains(name))) => format!("{}~{}", role, name),
        (Some(role), None) => role.clone(),
        (None, Some(selector::NameMatch::Exact(name))) => format!(":{}", name),
        (None, Some(selector::NameMatch::Contains(name))) => format!("~{}", name),
        (None, None) => "*".to_string(),
    }
}

fn format_chain(chain: &[&str]) -> String {
    chain.join(" > ")
}
