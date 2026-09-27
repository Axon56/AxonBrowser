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
                && state != "enabled"
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
/// the point is not owned by the target the page is nudged and the point
/// re-resolved: an overlay is dismissed with Escape, and a target parked under
/// a sticky header is scrolled back down into the viewport. If the target still
/// cannot be reached the click is refused instead of being sent into whatever
/// is on top.
pub async fn guarded_click_point<T: ClickTarget>(
    target: &T,
    browser_window: &window::WindowMatch,
) -> Result<(i32, i32, Option<String>)> {
    const ATTEMPTS: usize = 3;
    let mut blocked = None;
    let mut dismissed: Option<String> = None;

    for attempt in 0..ATTEMPTS {
        // `scroll_to` parks the target at the nearest edge of the viewport,
        // which is exactly where sticky headers and footers live. Move it to a
        // safe band before considering the click.
        reposition_if_at_edge(target, browser_window).await?;

        let (screen_x, screen_y) = inspect::clickable_point_stable(target.node()).await?;
        match classify_point(target.root(), target.node(), screen_x, screen_y).await? {
            PointOwner::Target | PointOwner::Related => {
                // The accessibility tree cannot always tell a disabled control
                // from an enabled one, so confirm against the page itself
                // before reporting a click that would change nothing.
                if let Some(state) =
                    crate::dom::element_at_point(crate::dom::current_flavor(), screen_x, screen_y)
                        .await
                    && state != "enabled"
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
                    // Dismiss whatever is on top and try again from a fresh
                    // position.
                    let _ = window::send_key(&browser_window.id, "Escape");
                    window::settle_after_input().await;
                    if let PointOwner::Blocked { label } = &owner {
                        dismissed = Some(label.clone());
                    }
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
        "click on {} is blocked by {}; nudged the page and retried {} times without the target becoming clickable",
        target.label(),
        blocker,
        ATTEMPTS - 1
    )
}

/// Keep the target inside a safe vertical band of the browser window.
///
/// Scroll-into-view aligns the target with the closest viewport edge, which
/// puts it underneath sticky headers (or footers). Rather than trusting that
/// position, nudge the page until the target sits in the middle of the window,
/// where nothing overlaps it.
async fn reposition_if_at_edge<T: ClickTarget>(
    target: &T,
    browser_window: &window::WindowMatch,
) -> Result<()> {
    // Fractions of the browser window height. The upper bound is deliberately
    // generous: the window includes browser chrome, so the page area starts
    // well below the window top.
    const MIN_FRACTION: f64 = 0.35;
    const MAX_FRACTION: f64 = 0.85;

    let window_top = f64::from(browser_window.y);
    let window_height = f64::from(browser_window.height.max(1));

    for _ in 0..4 {
        let (_, y, _, height) = inspect::stable_extents(target.node()).await?;
        let center = f64::from(y) + f64::from(height) / 2.0;
        let fraction = (center - window_top) / window_height;

        let direction = if fraction < MIN_FRACTION {
            // Too close to the top: bring it down.
            window::ScrollDirection::Up
        } else if fraction > MAX_FRACTION {
            window::ScrollDirection::Down
        } else {
            return Ok(());
        };

        if window::scroll(&browser_window.id, direction, 1).is_err() {
            return Ok(());
        }
        window::settle_after_input().await;
    }

    Ok(())
}

/// Confirm a click on an editable field actually took effect.
///
/// A click that is injected successfully but lands on something else otherwise
/// looks like success. For an editable field the observable effect is focus, so
/// a field that never takes focus is reported as a failure rather than a click.
/// Focus can lag the click slightly, so it is polled before giving up.
pub async fn verify_text_input_focus(node: &LiveNode) -> Result<()> {
    for attempt in 0..6u64 {
        let focused = inspect::read_state_set(node)
            .await
            .map(|states| states.contains(atspi::State::Focused))
            .unwrap_or(false);
        if focused {
            return Ok(());
        }
        if attempt + 1 < 6 {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        }
    }

    bail!(
        "click on {} did not take effect: the field never took focus, so the click landed on something else",
        node.line_label()
    )
}
