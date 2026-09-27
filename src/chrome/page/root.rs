use anyhow::{Result, anyhow};

use crate::{
    chrome::{retry, window as chrome_window},
    inspect,
    model::{LiveNode, UiNode},
    selector, window,
};

const PAGE_ROOT_CHAINS: &[&[&str]] = &[&["Document Web"], &["Root Web Area"]];
const FRAME_ROLE_CANDIDATES: &[&str] = &["Frame", "Internal Frame"];
const CHROME_BROWSER_QUERIES: &[&str] = &["google chrome", "chromium", "chromium-browser"];
const EDGE_BROWSER_QUERIES: &[&str] =
    &["edge", "microsoft edge", "microsoft\u{200b} edge", "msedge"];

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
    if current_browser_mode() == BrowserMode::Edge
        && let Ok(root) = resolve_page_root_from_edge_window().await
    {
        return Ok(root);
    }

    let mut failures = Vec::new();

    for chain in PAGE_ROOT_CHAINS {
        match resolve_chain(chain).await {
            Ok(nodes) if !nodes.is_empty() => {
                return Ok(outermost_document(nodes).await);
            }
            Ok(_) => failures.push(format!("{} => no matches", format_chain(chain))),
            Err(err) => failures.push(format!("{} => {}", format_chain(chain), err)),
        }
    }

    Err(anyhow!(
        "failed to resolve chrome page root; tried {}",
        failures.join("; ")
    ))
}

/// Pick the outermost matching document as the page root.
///
/// A page that embeds an iframe exposes both the top-level document and the
/// frame's document as `Document Web` nodes. Taking the first match can select
/// the nested document, which makes every page command silently target the
/// iframe instead of the page, so prefer the shallowest candidate.
///
/// A modal dialog is itself a document, so a page that opens one exposes a
/// second candidate that covers only the dialog. Picking that one makes the
/// whole page look like a handful of dialog nodes and every command report that
/// nothing matches, so the shallowest candidate wins and ties are broken by
/// preferring the one that contains the others.
async fn outermost_document(nodes: Vec<LiveNode>) -> LiveNode {
    let mut best: Option<LiveNode> = None;
    for candidate in nodes {
        best = Some(match best {
            None => candidate,
            Some(current) => {
                if is_better_root(&candidate, &current).await {
                    candidate
                } else {
                    current
                }
            }
        });
    }

    best.expect("non-empty result")
}

/// Prefer the shallower document; on a tie prefer the one that encloses the
/// other, which is the page rather than a dialog inside it.
async fn is_better_root(candidate: &LiveNode, current: &LiveNode) -> bool {
    if candidate.path.len() != current.path.len() {
        return candidate.path.len() < current.path.len();
    }

    inspect::descendant_refs(candidate, 8)
        .await
        .unwrap_or_default()
        .iter()
        .any(|reference| inspect::ref_key(reference) == inspect::ref_key(&current.object_ref))
}

async fn resolve_page_root_from_edge_window() -> Result<LiveNode> {
    let window_root = crate::edge::actions::click::resolve_locator("window").await?;
    for chain in PAGE_ROOT_CHAINS {
        let raw = chain
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>();
        let selectors = selector::parse_selector_chain(&raw)?;
        let nodes = inspect::resolve_within_scope(&window_root, &selectors).await?;
        if let Some(node) = nodes.into_iter().next() {
            return Ok(node);
        }
    }

    Err(anyhow!(
        "no page root matched under the resolved edge window"
    ))
}

async fn resolve_chain(chain: &[&str]) -> Result<Vec<LiveNode>> {
    let raw = chain
        .iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>();
    let selectors = selector::parse_selector_chain(&raw)?;
    let mut saw_nodes = false;
    for query in browser_root_queries() {
        if let Ok(nodes) = inspect::resolve(&query, &selectors).await
            && !nodes.is_empty()
        {
            saw_nodes = true;
            let scoped = scope_nodes_to_browser_window(nodes).await?;
            if !scoped.is_empty() {
                return Ok(scoped);
            }
        }
    }
    if saw_nodes {
        return Ok(Vec::new());
    }
    Err(anyhow!(
        "no accessible application or window matched any browser query"
    ))
}

fn browser_root_queries() -> Vec<String> {
    let mut queries = Vec::new();
    if let Ok(window) = chrome_window::find_browser_window(None) {
        queries.push(window.name.clone());
        if let Some(application) = window.name.split(" - ").next() {
            let trimmed = application.trim();
            if !trimmed.is_empty() {
                queries.push(trimmed.to_string());
            }
        }
    }
    queries.extend(current_browser_root_queries().iter().map(|q| q.to_string()));
    queries.sort();
    queries.dedup();
    queries
}

async fn scope_nodes_to_browser_window(nodes: Vec<LiveNode>) -> Result<Vec<LiveNode>> {
    let Some(target_window) = chrome_window::find_browser_window(None).ok() else {
        return Ok(nodes);
    };

    let mut id_filtered = Vec::new();
    for node in nodes.iter().cloned() {
        let (screen_x, screen_y) = match inspect::clickable_point(&node).await {
            Ok(point) => point,
            Err(_) => continue,
        };
        let matched_window = match window::find_window_at_point(screen_x, screen_y) {
            Ok(found) => found,
            Err(_) => continue,
        };
        if matched_window.id == target_window.id {
            id_filtered.push(node);
        }
    }
    if !id_filtered.is_empty() {
        return Ok(id_filtered);
    }

    let path_filtered = nodes
        .iter()
        .filter(|node| node_belongs_to_window_title(node, &target_window.name))
        .cloned()
        .collect::<Vec<_>>();
    if !path_filtered.is_empty() {
        return Ok(path_filtered);
    }

    Ok(Vec::new())
}

fn current_browser_root_queries() -> &'static [&'static str] {
    if current_browser_mode() == BrowserMode::Edge {
        EDGE_BROWSER_QUERIES
    } else {
        CHROME_BROWSER_QUERIES
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BrowserMode {
    Chrome,
    Edge,
}

fn current_browser_mode() -> BrowserMode {
    if std::env::var("GUIBOT_BROWSER_WINDOW_MODE").ok().as_deref() == Some("edge") {
        BrowserMode::Edge
    } else {
        BrowserMode::Chrome
    }
}

fn node_belongs_to_window_title(node: &LiveNode, window_title: &str) -> bool {
    let needle = crate::chrome::window::normalize(window_title);
    node.path.iter().any(|segment| {
        let hay = crate::chrome::window::normalize(segment);
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
