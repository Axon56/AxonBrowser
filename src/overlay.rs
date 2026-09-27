use anyhow::{Result, bail};

use crate::{inspect, model::LiveNode, window};

/// What actually occupies the point a click is about to land on.
#[derive(Debug, Clone)]
pub enum PointOwner {
    /// The point belongs to the target element itself.
    Target,
    /// The point belongs to a child or an ancestor of the target, so a click
    /// there still reaches the target.
    Related,
    /// Something else owns the point; the click would never reach the target.
    Blocked { label: String },
}

/// Minimal view of a page action target needed to guard a click.
pub trait ClickTarget {
    fn node(&self) -> &LiveNode;
    fn root(&self) -> &LiveNode;
    fn label(&self) -> &str;
}

/// Classify what owns a screen point relative to `target`.
///
/// The hit test runs against the page root so the answer is the topmost element
/// at that point, which is how a modal or promo popup is detected. When the
/// relationship cannot be established the point is treated as related, so an
/// unknown tree shape never causes a click to be refused.
pub async fn classify_point(
    root: &LiveNode,
    target: &LiveNode,
    screen_x: i32,
    screen_y: i32,
) -> Result<PointOwner> {
    let Some(hit) = inspect::accessible_at_point(root, screen_x, screen_y).await? else {
        return Ok(PointOwner::Related);
    };

    let hit_key = inspect::ref_key(&hit);
    if hit_key == inspect::ref_key(&target.object_ref) {
        return Ok(PointOwner::Target);
    }

    // The point may land on a child of the target, such as the label inside a
    // button.
    let descendants = match inspect::descendant_refs(target, 4).await {
        Ok(descendants) => descendants,
        // Could not enumerate the subtree, so there is not enough information to
        // call this an overlay: never refuse a click we cannot prove is blocked.
        Err(_) => return Ok(PointOwner::Related),
    };
    if descendants
        .iter()
        .any(|reference| inspect::ref_key(reference) == hit_key)
    {
        return Ok(PointOwner::Related);
    }

    // ...or on a container the target sits inside.
    let chain = match inspect::ancestor_chain(root, &target.object_ref).await {
        Ok(chain) => chain,
        Err(_) => return Ok(PointOwner::Related),
    };
    if chain.is_empty() {
        return Ok(PointOwner::Related);
    }
    if chain
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

    Ok(PointOwner::Blocked {
        label: inspect::describe_ref(&hit).await,
    })
}

/// Resolve a click point for `target` in window-relative coordinates.
///
/// Coordinates are re-read from freshly settled extents on every attempt, so a
/// preceding scroll can never leave the caller clicking a stale position. If an
/// overlay owns the point, it is dismissed and the point is re-resolved before
/// retrying; if the target stays covered the click is refused instead of being
/// sent into whatever is on top.
pub async fn guarded_click_point<T: ClickTarget>(
    target: &T,
    browser_window: &window::WindowMatch,
) -> Result<(i32, i32, Option<String>)> {
    const ATTEMPTS: usize = 3;
    let mut blocked = None;
    let mut dismissed: Option<String> = None;

    for attempt in 0..ATTEMPTS {
        let (screen_x, screen_y) = inspect::clickable_point_stable(target.node()).await?;
        match classify_point(target.root(), target.node(), screen_x, screen_y).await? {
            PointOwner::Target | PointOwner::Related => {
                let relative_x = screen_x - browser_window.x;
                let relative_y = screen_y - browser_window.y;
                if relative_x < 0 || relative_y < 0 {
                    bail!("resolved click point landed outside the target window");
                }
                return Ok((relative_x, relative_y, dismissed));
            }
            PointOwner::Blocked { label } => {
                if attempt + 1 < ATTEMPTS {
                    dismissed = Some(label.clone());
                    let _ = window::send_key(&browser_window.id, "Escape");
                    window::settle_after_input().await;
                }
                blocked = Some(label);
            }
        }
    }

    let blocker = blocked.unwrap_or_else(|| "an unknown element".to_string());
    bail!(
        "click on {} is blocked by {}; dismissed the overlay and retried {} times without reaching the target",
        target.label(),
        blocker,
        ATTEMPTS - 1
    )
}
