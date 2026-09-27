use anyhow::{Context, Result};
use atspi::proxy::{accessible::ObjectRefExt, proxy_ext::ProxyExt};

use crate::{model::LiveNode, window};

use super::context::{self, ActionTarget};

pub async fn click(locator_raw: &str) -> Result<String> {
    let target = context::resolve_target(locator_raw).await?;
    click_target(&target).await
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

/// Click a page node with an already-resolved page root, so the physical
/// fallback is guarded without walking the tree again.
pub async fn click_target_node_with_root(
    node: &LiveNode,
    label: &str,
    path: &str,
    root: &LiveNode,
) -> Result<String> {
    click_target_node_guarded(node, label, path, Some(root)).await
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
    Ok(summary)
}

/// Invoke the node's default accessibility action, if it exposes one.
///
/// This needs no coordinates, so it is immune to overlays and stale positions
/// and is preferred over any physical click.
pub(crate) async fn invoke_default_action(node: &LiveNode) -> Result<bool> {
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
