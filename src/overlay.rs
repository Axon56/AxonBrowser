use anyhow::{Result, bail};

use crate::{inspect, model::LiveNode, window};

/// What actually occupies the point a click is about to land on.
#[derive(Debug, Clone)]
pub enum PointOwner {
    /// The point belongs to the target element itself.
    Target,
    /// The point belongs to the target's own label or another descendant, so a
    /// click there still reaches the target.
    Related,
    /// Something else owns the point; the click would never reach the target.
    Blocked { label: String },
    /// Ownership could not be established. Treated as unsafe, never as success.
    Unknown,
}

/// Minimal view of a page action target needed to guard a click.
pub trait ClickTarget {
    fn node(&self) -> &LiveNode;
    fn root(&self) -> &LiveNode;
    fn label(&self) -> &str;
}

/// A click target assembled from a node and the root it was resolved under.
pub struct NodeTarget<'a> {
    pub node: &'a LiveNode,
    pub root: &'a LiveNode,
    pub label: &'a str,
}

impl ClickTarget for NodeTarget<'_> {
    fn node(&self) -> &LiveNode {
        self.node
    }

    fn root(&self) -> &LiveNode {
        self.root
    }

    fn label(&self) -> &str {
        self.label
    }
}

/// Whether this role is an editable field, where a click must leave it focused.
pub fn is_text_input_role(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "entry" | "password text" | "text" | "text box" | "combo box"
    )
}

/// Whether this role's activation is observable as a checked state.
pub fn is_checkable_role(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "radio button" | "check box" | "toggle button" | "switch"
    )
}

/// Whether this role's activation is observable as an opened dropdown.
pub fn is_dropdown_role(role: &str) -> bool {
    role.trim().eq_ignore_ascii_case("combo box")
}

/// Whether the control's dropdown opened, polling for the list to appear.
///
/// Opening the list is the observable effect of clicking a combo box. A custom
/// dropdown is frequently a `div` that never takes keyboard focus, so a click
/// that worked would otherwise be reported as having landed elsewhere.
///
/// Two signals are accepted. The control's own expanded state is the direct one,
/// but its name often comes from a sibling label rather than an attribute, in
/// which case no element matches that name. The set of visible lists is then
/// compared instead, which needs no name at all: a new or changed list is the
/// list this click opened.
pub async fn dropdown_opened(node: &LiveNode, lists_before: Option<&str>) -> bool {
    for attempt in 0..6u64 {
        if crate::dom::control_expanded(node).await == Some(true) {
            return true;
        }
        if let Some(before) = lists_before
            && let Some(now) = crate::dom::visible_option_lists().await
            && now != before
        {
            return true;
        }
        if attempt + 1 < 6 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    false
}

/// The control's checked state, from the page first and the accessibility tree
/// second.
///
/// The page is authoritative here. The tree reports a radio group loosely: on a
/// real booking form it described both options as checked, so comparing its
/// answers before and after a click found no change and the click was written off
/// as having had no effect even when the page had switched. The tree is still
/// consulted when the page cannot answer, because a control whose accessible name
/// does not match any element still has a state worth reporting.
///
/// `None` means the state could not be read, which callers must treat as
/// "unknown" rather than as "unchecked".
pub async fn checked_state(node: &LiveNode) -> Option<bool> {
    if let Some(checked) = crate::dom::control_checked(node).await {
        return Some(checked);
    }

    let states = inspect::read_state_set(node).await.ok()?;
    Some(
        states.contains(atspi::State::Checked)
            || states.contains(atspi::State::Selected)
            || states.contains(atspi::State::Pressed),
    )
}

/// Whether the control's checked state moved away from `before`.
///
/// Polled, because the page applies the change asynchronously and a single read
/// straight after the click can still describe the previous state.
pub async fn checked_state_changed(node: &LiveNode, before: bool) -> bool {
    for attempt in 0..6u64 {
        if let Some(now) = checked_state(node).await
            && now != before
        {
            return true;
        }
        if attempt + 1 < 6 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    false
}

/// Whether a node lives inside rendered page content rather than browser chrome.
///
/// Browser chrome (tool bar, tab strip) has no page to overlay it, so it does
/// not need the page-level guard.
pub fn is_page_content(node: &LiveNode) -> bool {
    node.path.iter().any(|segment| {
        let segment = segment.to_ascii_lowercase();
        segment.contains("document web") || segment.contains("root web area")
    })
}

/// Resolve a physical click point, guarding it when the node is page content.
///
/// `root` is the resolved page root for page content, and `None` for browser
/// chrome. Page content goes through the full guard, so a click can never land
/// on a sticky header, an overlay, or a stale position and still be reported as
/// a success.
pub async fn guarded_or_direct_click(
    node: &LiveNode,
    label: &str,
    root: Option<&LiveNode>,
) -> Result<(window::WindowMatch, i32, i32, Option<String>)> {
    let (screen_x, screen_y) = inspect::clickable_point_stable(node).await?;
    let browser_window = window::find_window_at_point(screen_x, screen_y)?;

    match root {
        Some(root) => {
            let target = NodeTarget { node, root, label };
            let (relative_x, relative_y, dismissed) =
                guarded_click_point(&target, &browser_window).await?;
            Ok((browser_window, relative_x, relative_y, dismissed))
        }
        None => {
            if let Some(state) =
                crate::dom::element_at_point(crate::dom::current_flavor(), screen_x, screen_y).await
                && crate::dom::state_blocks_click(&state)
            {
                bail!(
                    "{} at this point is {state}, so clicking it would have no effect",
                    label
                );
            }

            let relative_x = screen_x - browser_window.x;
            let relative_y = screen_y - browser_window.y;
            Ok((browser_window, relative_x, relative_y, None))
        }
    }
}

/// Classify what owns a screen point relative to `target`.
///
/// The hit test runs against the page root so the answer is the topmost element
/// at that point, which is how a modal, promo popup, or sticky header is
/// detected. Only the target itself or one of its own descendants counts as
/// safe: an ancestor or unrelated element means the click would land somewhere
/// else, and an unreadable hit test is treated as unsafe rather than assumed
/// fine. Reporting a click that actually hit a navigation header is worse than
/// refusing it.
pub async fn classify_point(
    root: &LiveNode,
    target: &LiveNode,
    screen_x: i32,
    screen_y: i32,
) -> Result<PointOwner> {
    let Some(hit) = inspect::accessible_at_point(root, screen_x, screen_y).await? else {
        return Ok(PointOwner::Unknown);
    };

    let hit_key = inspect::ref_key(&hit);
    if hit_key == inspect::ref_key(&target.object_ref) {
        return Ok(PointOwner::Target);
    }

    // The point may land on a child of the target, such as the label inside a
    // button.
    let descendants = match inspect::descendant_refs(target, 4).await {
        Ok(descendants) => descendants,
        // Could not enumerate the subtree, so ownership is undetermined.
        Err(_) => return Ok(PointOwner::Unknown),
    };
    if descendants
        .iter()
        .any(|reference| inspect::ref_key(reference) == hit_key)
    {
        return Ok(PointOwner::Related);
    }

    // Some toolkits report a control's own label as a sibling rather than a
    // child, so an identical accessible name also means the click lands on the
    // target rather than on an overlay.
    let target_name = target.name.as_deref().unwrap_or_default().trim();
    let hit_name = inspect::ref_name(&hit).trim();
    if !target_name.is_empty() && target_name.eq_ignore_ascii_case(hit_name) {
        return Ok(PointOwner::Related);
    }

    // Anything drawn inside the target's own bounds is part of the target: a
    // label or icon inside a button or link. This is a geometric test, so it
    // holds regardless of how the toolkit arranges the tree.
    if let (Ok(target_rect), Ok(hit_rect)) = (
        inspect::component_extents(target).await,
        inspect::extents_of_ref(&hit).await,
    ) && contains_rect(target_rect, hit_rect)
    {
        return Ok(PointOwner::Related);
    }

    Ok(PointOwner::Blocked {
        label: inspect::describe_ref(&hit).await,
    })
}

/// Whether `inner` lies within `outer`, allowing a small tolerance for
/// sub-pixel rounding.
fn contains_rect(outer: (i32, i32, i32, i32), inner: (i32, i32, i32, i32)) -> bool {
    const TOLERANCE: i32 = 4;
    let (ox, oy, ow, oh) = outer;
    let (ix, iy, iw, ih) = inner;
    if ow <= 0 || oh <= 0 || iw <= 0 || ih <= 0 {
        return false;
    }

    ix >= ox - TOLERANCE
        && iy >= oy - TOLERANCE
        && ix + iw <= ox + ow + TOLERANCE
        && iy + ih <= oy + oh + TOLERANCE
}

/// Resolve a click point for `target` in window-relative coordinates.
///
/// Coordinates are re-read from freshly settled extents on every attempt, so a
/// preceding scroll can never leave the caller clicking a stale position. When
/// the point is not owned by the target, or the page itself reports that a
/// click there would hit a navigation link or a fixed overlay, the page is
/// nudged and the point re-resolved: an overlay is dismissed with Escape, and a
/// target parked under a sticky header is scrolled back to the middle of the
/// viewport. If the target still cannot be reached the click is refused instead
/// of being sent into whatever is on top.
pub async fn guarded_click_point<T: ClickTarget>(
    target: &T,
    browser_window: &window::WindowMatch,
) -> Result<(i32, i32, Option<String>)> {
    const ATTEMPTS: usize = 3;
    let mut blocked = None;
    let mut dismissed: Option<String> = None;
    // A link may legitimately be under the point when the target is that link,
    // so the navigation check is only applied to other kinds of control.
    let target_is_link = target.node().role.trim().eq_ignore_ascii_case("link");

    // Clear anything covering the page before the first coordinate is resolved.
    // Resolving first and dismissing afterwards leaves the first click aimed at
    // where the target sits under the overlays, which on a page with a promo and a
    // cookie banner stacked means the click is sent to a point the overlay owns.
    // The guard catches that and refuses, but the aim was already wrong, so the
    // step costs a retry it should never have needed.
    if crate::dom::covering_overlay().await.is_some() {
        let _ = window::send_key(&browser_window.id, "Escape");
        window::settle_after_input().await;
        if let Some(label) = crate::modal::dismiss_if_present().await {
            dismissed = Some(label);
        }
        window::settle_after_input().await;
    }

    for attempt in 0..ATTEMPTS {
        let (screen_x, screen_y) = inspect::clickable_point_stable(target.node()).await?;

        // Ask the page what a click at this point would actually hit. The
        // accessibility hit test can name the target as the owner while the
        // page's own hit test resolves the point to a sticky header sitting on
        // top of it, and that is how a click on a form field silently opens a
        // navigation menu instead. The page's answer is the one that decides.
        if let Some(hit) =
            crate::dom::point_hit(crate::dom::current_flavor(), screen_x, screen_y).await
        {
            let unreachable = if !hit.inside {
                // Off screen entirely, so the extents describe a position the
                // user cannot click.
                Some("the target is outside the visible viewport".to_string())
            } else if hit.link && !target_is_link {
                Some("a navigation link is on top of the target".to_string())
            } else if hit.overlay && !target_is_link {
                Some("a fixed or sticky overlay is on top of the target".to_string())
            } else {
                None
            };

            if let Some(reason) = unreachable {
                blocked = Some(reason);
                if attempt + 1 < ATTEMPTS {
                    // Dismiss first, then reposition. An overlay is removed with
                    // Escape and a sticky header is not going anywhere, so both
                    // are needed: without the dismissal the retry finds the same
                    // cover, and without the reposition it finds the same covered
                    // point. The coordinate is resolved again at the top of the
                    // next attempt, after both have been applied.
                    if hit.overlay && !target_is_link {
                        let _ = window::send_key(&browser_window.id, "Escape");
                        window::settle_after_input().await;
                    }
                    center_target(target, browser_window, screen_x, screen_y).await?;
                }
                continue;
            }
        }

        match classify_point(target.root(), target.node(), screen_x, screen_y).await? {
            PointOwner::Target | PointOwner::Related => {
                // The accessibility tree cannot always tell a disabled control
                // from an enabled one, so confirm against the page itself
                // before reporting a click that would change nothing.
                if let Some(state) =
                    crate::dom::element_at_point(crate::dom::current_flavor(), screen_x, screen_y)
                        .await
                    && crate::dom::state_blocks_click(&state)
                {
                    bail!(
                        "{} at this point is {state}, so clicking it would have no effect",
                        target.label()
                    );
                }

                let relative_x = screen_x - browser_window.x;
                let relative_y = screen_y - browser_window.y;
                if relative_x < 0 || relative_y < 0 {
                    bail!("resolved click point landed outside the target window");
                }
                return Ok((relative_x, relative_y, dismissed));
            }
            owner @ (PointOwner::Blocked { .. } | PointOwner::Unknown) => {
                if attempt + 1 < ATTEMPTS {
                    // Dismiss whatever is on top, then move the target itself.
                    // Both are needed: a popup is removed with Escape, while a
                    // sticky header is not going anywhere, so the target has to
                    // be brought out from under it. Without the reposition the
                    // retry re-resolves the same covered point and fails again,
                    // which is how a field parked at the top edge stayed
                    // unclickable no matter how many times the click was retried.
                    let _ = window::send_key(&browser_window.id, "Escape");
                    window::settle_after_input().await;
                    if let PointOwner::Blocked { label } = &owner {
                        dismissed = Some(label.clone());
                    }
                    center_target(target, browser_window, screen_x, screen_y).await?;
                }
                blocked = Some(match owner {
                    PointOwner::Blocked { label } => label,
                    _ => "an element that could not be identified".to_string(),
                });
            }
        }
    }

    let blocker = blocked.unwrap_or_else(|| "an unknown element".to_string());
    bail!(
        "click on {} is blocked by {}; nudged the page and retried {} times without the target becoming clickable, so the click was not sent",
        target.label(),
        blocker,
        ATTEMPTS - 1
    )
}

/// Move the target to the middle of the viewport so sticky chrome cannot cover
/// it.
///
/// Scroll-into-view aligns the target with the closest viewport edge, which puts
/// it underneath sticky headers (or footers). The correction is computed in the
/// page's own coordinates and applied with a page scroll, because a scroll wheel
/// nudge moves by an unknown amount and can leave the target exactly where it
/// was. The wheel is kept as a fallback for pages the DOM cannot be queried on.
async fn center_target<T: ClickTarget>(
    target: &T,
    browser_window: &window::WindowMatch,
    screen_x: i32,
    screen_y: i32,
) -> Result<()> {
    // The control is found by its accessible name, not by the point: the point
    // is exactly what the occluder has taken over, so asking the page about it
    // returns the header and finds nothing to scroll.
    let name = target.node().name.as_deref().unwrap_or_default().trim();
    let flavor = crate::dom::current_flavor();
    if crate::dom::bring_into_view(flavor, name, screen_x, screen_y).await {
        window::settle_after_input().await;
        return Ok(());
    }

    // The point-based correction is tried next, because it works when the target
    // has no accessible name -- and on a real booking form the airport combos are
    // unnamed, so the name lookup above finds nothing for them.
    if crate::dom::center_at_point(flavor, screen_x, screen_y).await {
        window::settle_after_input().await;
        return Ok(());
    }

    // The page could not be reached, or it had nothing left to scroll. Fall back
    // to nudging the scroll wheel, which is imprecise but better than giving up.
    let _ = window::scroll(&browser_window.id, window::ScrollDirection::Up, 1);
    window::settle_after_input().await;
    Ok(())
}

/// Confirm a click on an editable field actually took effect.
///
/// A click that is injected successfully but lands on something else otherwise
/// looks like success. For an editable field the observable effect is focus, so
/// a field that never takes focus is reported as a failure rather than a click.
/// Focus can lag the click slightly, so it is polled before giving up.
///
/// Focus is only evidence when it *changed*. If the field was already focused
/// before the click -- which happens whenever an earlier command focused it --
/// then observing focus proves nothing, and reporting success would be the same
/// false success this check exists to prevent. In that case the caller's
/// pre-click checks are the only real evidence, so the outcome is reported as
/// unverified rather than confirmed.
pub async fn verify_text_input_focus_after(
    node: &LiveNode,
    was_focused_before: bool,
) -> Result<FocusOutcome> {
    let focused = focus_is_settled(node).await;
    if !focused {
        bail!(
            "click on {} did not take effect: the field never took focus, so the click landed on something else",
            node.line_label()
        );
    }

    Ok(if was_focused_before {
        FocusOutcome::AlreadyFocused
    } else {
        FocusOutcome::FocusGained
    })
}

/// How a click's focus effect was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusOutcome {
    /// The click moved focus onto the field, which proves it landed there.
    FocusGained,
    /// The field already had focus, so focus is not evidence either way.
    AlreadyFocused,
}

/// Whether the field reports itself as focused, polling briefly for it to settle.
async fn focus_is_settled(node: &LiveNode) -> bool {
    for attempt in 0..6u64 {
        if inspect::read_state_set(node)
            .await
            .map(|states| states.contains(atspi::State::Focused))
            .unwrap_or(false)
        {
            return true;
        }
        if attempt + 1 < 6 {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        }
    }
    false
}

pub async fn verify_text_input_focus(node: &LiveNode) -> Result<()> {
    if focus_is_settled(node).await {
        return Ok(());
    }

    bail!(
        "click on {} did not take effect: the field never took focus, so the click landed on something else",
        node.line_label()
    )
}

#[cfg(test)]
mod tests {
    use super::{is_checkable_role, is_text_input_role};

    #[test]
    fn checkable_roles_are_the_ones_with_an_observable_state() {
        assert!(is_checkable_role("Radio Button"));
        assert!(is_checkable_role("Check Box"));
        assert!(is_checkable_role("Toggle Button"));
        // A plain button has no state to verify, so it must not be treated as
        // checkable: doing so would demand a change that never comes.
        assert!(!is_checkable_role("Push Button"));
        assert!(!is_checkable_role("Entry"));
    }

    #[test]
    fn text_input_roles_include_the_editable_controls() {
        assert!(is_text_input_role("Entry"));
        assert!(is_text_input_role("Combo Box"));
        assert!(!is_text_input_role("Push Button"));
    }
}
