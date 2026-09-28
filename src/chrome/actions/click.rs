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
    // Check the page itself before either click path runs. The accessibility
    // action interface happily "activates" a disabled control and reports
    // success while nothing changes, so this cannot be left to the tree.
    crate::dom::refuse_if_disabled(node, label).await?;

    // A control with a checked state must actually change it. The action
    // interface reports success whether or not anything happened -- a radio on a
    // real booking form answered "clicked" while the page still showed the other
    // option selected -- so the state is read first and checked after, and a
    // physical click is tried when the action changed nothing.
    let before = if crate::overlay::is_checkable_role(&node.role) {
        crate::overlay::checked_state(node).await
    } else {
        None
    };

    if invoke_default_action(node).await? {
        match before {
            // The state was readable and did move, so the click is confirmed.
            Some(before) if crate::overlay::checked_state_changed(node, before).await => {
                return Ok(format!("clicked {} via AT-SPI action ({})", label, path));
            }
            // Nothing to verify, so the action is all the evidence there is.
            None => return Ok(format!("clicked {} via AT-SPI action ({})", label, path)),
            // The action claimed success but the state did not move, so fall
            // through to a physical click instead of reporting a no-op.
            Some(_) => {}
        }
    }

    let (browser_window, relative_x, relative_y, dismissed) =
        crate::overlay::guarded_or_direct_click(node, label, root).await?;
    let activation_note = context::activate_window_note(&browser_window.id);
    window::mousemove_click(&browser_window.id, relative_x, relative_y)?;
    if crate::overlay::is_text_input_role(&node.role) {
        crate::overlay::verify_text_input_focus(node).await?;
    }

    // Do not report a click whose whole purpose was to change a state when the
    // state never changed.
    if let Some(before) = before
        && !crate::overlay::checked_state_changed(node, before).await
    {
        anyhow::bail!(
            "clicked {} at {},{} in window {} but its state did not change, so the click had no effect",
            label,
            relative_x,
            relative_y,
            browser_window.id
        );
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

/// Invoke the node's default accessibility action, if it exposes one.
///
/// This needs no coordinates, so it is immune to overlays and stale positions
/// and is preferred over any physical click.
pub(crate) async fn invoke_default_action(node: &LiveNode) -> Result<bool> {
    let connection = crate::inspect::connect_accessibility().await?;
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
