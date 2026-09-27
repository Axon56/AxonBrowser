use anyhow::{Context, Result, anyhow};
use atspi::proxy::{accessible::ObjectRefExt, proxy_ext::ProxyExt};

use crate::{chrome::retry, inspect, model::LiveNode, selector, window};

use super::context::{self, ActionTarget};

pub async fn click(locator_raw: &str) -> Result<String> {
    if is_browser_button_locator(locator_raw) {
        let node = resolve_locator(locator_raw).await.map_err(|err| {
            anyhow!(
                "edge locator {:?} did not resolve in accessibility tree: {}",
                locator_raw,
                err
            )
        })?;
        let label = node.line_label();
        let path = node.path.join(" > ");
        return click_target_node(&node, &label, &path).await;
    }

    match context::resolve_target(locator_raw).await {
        Ok(target) => click_target(&target).await,
        Err(primary_err) => keyboard_shortcut_fallback(locator_raw, &primary_err.to_string()),
    }
}

pub async fn click_target(target: &ActionTarget) -> Result<String> {
    click_target_node(&target.node, &target.label, &target.path).await
}

pub async fn click_target_node(node: &LiveNode, label: &str, path: &str) -> Result<String> {
    // Page content is guarded against overlays, sticky headers, and stale
    // positions. Browser chrome has no page layered over it, so a direct click
    // is safe there.
    let root = if crate::overlay::is_page_content(node) {
        Some(crate::chrome::page::root::resolve_page_scope(&Default::default()).await?)
    } else {
        None
    };

    click_target_node_guarded(node, label, path, root.as_ref()).await
}

async fn click_target_node_guarded(
    node: &LiveNode,
    label: &str,
    path: &str,
    root: Option<&LiveNode>,
) -> Result<String> {
    if invoke_default_action(node).await? {
        return Ok(format!("clicked {} via AT-SPI action ({})", label, path));
    }

    let (browser_window, relative_x, relative_y, dismissed) =
        crate::overlay::guarded_or_direct_click(node, label, root).await?;
    let activation_note = context::activate_window_note(&browser_window.id);
    window::mousemove_click(&browser_window.id, relative_x, relative_y)?;
    if crate::overlay::is_text_input_role(&node.role) {
        crate::overlay::verify_text_input_focus(node).await?;
    }

    let mut summary = format!(
        "clicked {} via X11 at {},{} in window {} ({}, {})",
        label, relative_x, relative_y, browser_window.id, path, activation_note
    );
    if let Some(overlay) = dismissed {
        summary = format!(
            "{} | dismissed overlay {} before clicking",
            summary, overlay
        );
    }
    if let Some(note) = crate::inspect::unverified_note(node).await {
        summary = format!("{} | {}", summary, note);
    }
    Ok(summary)
}

pub async fn resolve_locator(locator_raw: &str) -> Result<LiveNode> {
    retry::with_transient_retry(|| async {
        let locator = crate::chrome::locators::ChromeLocator::parse(locator_raw)?;
        match locator {
            crate::chrome::locators::ChromeLocator::AddressBar => {
                resolve_first_from_chains(&[
                    &["Tool Bar", "Entry~address"],
                    &["Tool Bar", "Entry~search"],
                    &["Tool Bar", "Entry:Address and search bar"],
                ])
                .await
            }
            crate::chrome::locators::ChromeLocator::Back => {
                resolve_first_from_chains(&[
                    &["Tool Bar", "Push Button:Back"],
                    &["Tool Bar", "Push Button~Back"],
                ])
                .await
            }
            crate::chrome::locators::ChromeLocator::Forward => {
                resolve_first_from_chains(&[
                    &["Tool Bar", "Push Button:Forward"],
                    &["Tool Bar", "Push Button~Forward"],
                ])
                .await
            }
            crate::chrome::locators::ChromeLocator::Reload => {
                // Prefer exact name matching (:) first, then fuzzy contains (~) for
                // locale/variant labels like "Refresh" that differ by channel/build.
                resolve_first_from_chains(&[
                    &["Tool Bar", "Push Button:Reload"],
                    &["Tool Bar", "Push Button:Refresh"],
                    &["Tool Bar", "Push Button~Reload"],
                    &["Tool Bar", "Push Button~Refresh"],
                ])
                .await
            }
            crate::chrome::locators::ChromeLocator::NewTab => {
                resolve_first_from_chains(&[
                    &["Page Tab List", "Push Button:New Tab"],
                    &["Tool Bar", "Push Button:New tab"],
                    &["Page Tab List", "Push Button~New tab"],
                    &["Tool Bar", "Push Button~New tab"],
                ])
                .await
            }
            crate::chrome::locators::ChromeLocator::TabStrip => {
                resolve_first_from_chains(&[&["Page Tab List"]]).await
            }
            crate::chrome::locators::ChromeLocator::CurrentTab => resolve_current_tab().await,
            crate::chrome::locators::ChromeLocator::Window => resolve_window().await,
        }
    })
    .await
}

async fn resolve_window() -> Result<LiveNode> {
    let candidates = resolve_chain(&["Frame"]).await?;
    choose_by_states(
        candidates,
        &[atspi::State::Active, atspi::State::Focused],
        "window",
    )
    .await
}

async fn resolve_current_tab() -> Result<LiveNode> {
    let candidates = resolve_chain(&["Page Tab List", "Page Tab"]).await?;
    choose_by_states(
        candidates,
        &[
            atspi::State::Selected,
            atspi::State::Active,
            atspi::State::Focused,
        ],
        "current tab",
    )
    .await
}

async fn resolve_first_from_chains(chains: &[&[&str]]) -> Result<LiveNode> {
    let mut failures = Vec::new();

    for chain in chains {
        match resolve_chain(chain).await {
            Ok(nodes) if !nodes.is_empty() => {
                return Ok(nodes.into_iter().next().expect("non-empty result"));
            }
            Ok(_) => failures.push(format!("{} => no matches", format_chain(chain))),
            Err(err) => failures.push(format!("{} => {}", format_chain(chain), err)),
        }
    }

    Err(anyhow!(
        "failed to resolve edge locator; tried {}",
        failures.join("; ")
    ))
}

async fn resolve_chain(chain: &[&str]) -> Result<Vec<LiveNode>> {
    resolve_chain_scoped(chain).await
}

pub(crate) async fn resolve_chain_scoped(chain: &[&str]) -> Result<Vec<LiveNode>> {
    let raw = chain
        .iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>();
    let selectors = selector::parse_selector_chain(&raw)?;
    for query in browser_root_queries() {
        if let Ok(nodes) = crate::inspect::resolve(&query, &selectors).await
            && !nodes.is_empty()
        {
            return scope_nodes_to_active_window(nodes).await;
        }
    }
    Err(anyhow!(
        "no accessible application or window matched any edge query"
    ))
}

fn browser_root_queries() -> Vec<String> {
    let mut queries = Vec::new();
    if let Ok(window) = crate::edge::window::find_edge_window(None) {
        let _ = crate::edge::session::remember_target(&window.id);
        queries.push(window.name);
    }
    queries.push("microsoft edge".to_string());
    queries.push("msedge".to_string());
    queries.push("edge".to_string());
    queries
}

async fn choose_by_states(
    candidates: Vec<LiveNode>,
    states: &[atspi::State],
    label: &str,
) -> Result<LiveNode> {
    if candidates.is_empty() {
        return Err(anyhow!("no edge {} candidates matched", label));
    }

    let mut fallback = None;
    for candidate in candidates {
        if fallback.is_none() {
            fallback = Some(candidate.clone());
        }

        let state_set = crate::inspect::read_state_set(&candidate)
            .await
            .unwrap_or_default();
        if states.iter().any(|state| state_set.contains(*state)) {
            return Ok(candidate);
        }
    }

    Ok(fallback.expect("fallback set when candidates is not empty"))
}

async fn scope_nodes_to_active_window(nodes: Vec<LiveNode>) -> Result<Vec<LiveNode>> {
    let Some(target_window) = crate::edge::window::find_edge_window(None).ok() else {
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
        let (screen_x, screen_y) = match inspect::clickable_point(&node).await {
            Ok(point) => point,
            Err(_) => continue,
        };
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
    let needle = crate::edge::window::normalize(window_title);
    node.path.iter().any(|segment| {
        let hay = crate::edge::window::normalize(segment);
        hay.contains(&needle) || needle.contains(&hay)
    })
}

fn format_chain(chain: &[&str]) -> String {
    chain.join(" > ")
}

async fn invoke_default_action(node: &LiveNode) -> Result<bool> {
    let connection = atspi::AccessibilityConnection::new()
        .await
        .context("failed to connect to the AT-SPI accessibility bus")?;
    let accessible = node
        .object_ref
        .as_accessible_proxy(connection.connection())
        .await
        .context("failed to bind matched node for action lookup")?;
    let proxies = accessible
        .proxies()
        .await
        .context("failed to inspect matched node interfaces")?;

    let action = match proxies.action().await {
        Ok(action) => action,
        Err(_) => return Ok(false),
    };

    match action.do_action(0).await {
        Ok(invoked) => Ok(invoked),
        Err(_) => Ok(false),
    }
}

fn keyboard_shortcut_fallback(locator_raw: &str, reason: &str) -> Result<String> {
    let locator = crate::chrome::locators::ChromeLocator::parse(locator_raw)?;
    let edge_window = crate::edge::window::find_edge_window(None)?;
    let activation_note = context::activate_window_note(&edge_window.id);

    let key = match locator {
        crate::chrome::locators::ChromeLocator::Back => "Alt+Left",
        crate::chrome::locators::ChromeLocator::Forward => "Alt+Right",
        crate::chrome::locators::ChromeLocator::Reload => "ctrl+r",
        crate::chrome::locators::ChromeLocator::NewTab => "ctrl+t",
        crate::chrome::locators::ChromeLocator::AddressBar => "ctrl+l",
        _ => {
            return Err(anyhow!(
                "failed to resolve edge locator {:?}: {}",
                locator_raw,
                reason
            ));
        }
    };

    crate::window::send_key(&edge_window.id, key)?;
    Ok(format!(
        "triggered edge {} via keyboard fallback {} in window {} ({}, original locator failed: {})",
        locator.canonical_name(),
        key,
        edge_window.id,
        activation_note,
        reason
    ))
}

fn is_browser_button_locator(locator_raw: &str) -> bool {
    matches!(
        crate::chrome::locators::ChromeLocator::parse(locator_raw),
        Ok(crate::chrome::locators::ChromeLocator::Back)
            | Ok(crate::chrome::locators::ChromeLocator::Forward)
            | Ok(crate::chrome::locators::ChromeLocator::Reload)
            | Ok(crate::chrome::locators::ChromeLocator::NewTab)
    )
}
