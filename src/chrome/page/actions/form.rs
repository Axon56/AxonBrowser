use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use atspi::State;
use tokio::time::sleep;

use crate::{
    chrome::{actions::click::click_target_node_with_root, page::root::PageScope},
    model::LiveNode,
    selector, window,
};

use super::target::PageActionTarget;

pub async fn check(scope: &PageScope, raw_selectors: &[String]) -> Result<String> {
    set_toggle(scope, raw_selectors, true).await
}

pub async fn uncheck(scope: &PageScope, raw_selectors: &[String]) -> Result<String> {
    set_toggle(scope, raw_selectors, false).await
}

pub async fn select_option(
    scope: &PageScope,
    raw_selectors: &[String],
    option: &str,
    nth: Option<usize>,
) -> Result<String> {
    // The index addresses the control, not the option: the common need is the
    // second combo box on a form, and a custom dropdown's control selector is what
    // matches several of them. The option is always identified by its label.
    let target = PageActionTarget::resolve_nth(scope, raw_selectors, nth).await?;
    let mut notes = Vec::new();

    // A dropdown that is already open is the control to act on, and its list is the
    // only one worth looking in. This is the case that made a selection land in the
    // wrong combo box: with the second one open, resolving the option by label found
    // the first list's copy of the same label and applied the value there. Only a
    // list belonging to the control the caller named is used, so a list left open on
    // some other control cannot be changed by mistake.
    if let Some(open) = crate::dom::open_dropdown().await
        && list_belongs_to_target(&open, &target).await
        && let Some(summary) = click_option_in_open_list(&open, option, &target, &notes).await?
    {
        return Ok(summary);
    }

    // Already showing the wanted value: say so rather than opening the list and
    // shutting it again for no reason. Only page evidence counts here, because
    // skipping a needed change is worse than doing a redundant one.
    if option_confirmed_by_page(&target, option).await {
        return Ok(format!(
            "{} already shows {:?}, so nothing was changed",
            target.label, option
        ));
    }

    if target.scroll_into_view().await? {
        notes.push("scrolled into view first".to_string());
    }

    // Record what is on screen before opening anything, so a list that does not open
    // can be told apart from one that was already showing.
    let before = crate::dom::dropdown_state(crate::dom::current_flavor(), None).await;
    let before_options = before.as_ref().map(|state| state.options.as_str());

    // Open the control, retrying with a fresh position each time. A single click that
    // does not open the list is usually the scroll-state wobble: the page moved, the
    // click was sent to where the control used to be, and nothing happened.
    // Re-resolving and clicking again makes that self-correcting, and the retry is
    // bounded so a control that genuinely will not open is still reported.
    let mut open_summary = String::new();
    let mut opened = false;
    for attempt in 0..3u64 {
        open_summary =
            click_target_node_with_root(&target.node, &target.label, &target.path, &target.root)
                .await?;
        if list_opened(before_options, &target).await {
            opened = true;
            break;
        }
        if attempt < 2 {
            let fresh = PageActionTarget::resolve_nth(scope, raw_selectors, nth).await?;
            let _ = fresh.scroll_into_view().await;
            sleep(Duration::from_millis(250)).await;
        }
    }
    if !opened {
        bail!(
            "clicked {} three times but its own list never opened, so no option was chosen; the click is landing on something else",
            target.label
        );
    }

    // Click the option in the list that is actually open, by its own position. The
    // tree cannot be trusted to say which list an option belongs to: a widget renders
    // its list as a sibling of the control, so every closed dropdown's options sit in
    // the tree at the same level as the open one's, and two dropdowns on a booking
    // form offer the same labels. Resolving by label alone therefore picks the first
    // list on the page, which is how a value meant for the second combo box was
    // applied to the first and overwrote it. The visible list is unambiguous.
    let select_summary = match crate::dom::open_dropdown().await {
        Some(open) if list_belongs_to_target(&open, &target).await => {
            match click_option_in_open_list(&open, option, &target, &notes).await? {
                Some(summary) => summary,
                None => click_option_via_tree(scope, option, &target).await?,
            }
        }
        _ => click_option_via_tree(scope, option, &target).await?,
    };

    // Verify against the control that was acted on, re-resolved so the check reads
    // current state. The index is applied again: without it the first combo box on the
    // page was checked instead, and a successful selection on the second one was
    // reported as unverified.
    let verified = match PageActionTarget::resolve_nth(scope, raw_selectors, nth).await {
        Ok(fresh) => option_selected(&fresh, option).await,
        Err(_) => false,
    };

    if !verified {
        // The option was clicked but the control does not report the new value. That
        // is a real failure, not a note: reporting it as a success line is what let a
        // missing selection pass as done.
        let observed = observed_value(&target).await;
        bail!(
            "clicked option {:?} in {} but the control does not report it as its value; observed {:?}, expected {:?} ({})",
            option,
            target.label,
            observed,
            option,
            select_summary
        );
    }

    Ok(attach_notes(
        format!(
            "selected option {:?} via {} | {}",
            option, open_summary, select_summary
        ),
        &notes,
    ))
}

/// Click an option using the accessibility tree.
///
/// Used for a control whose options are not exposed as a separate list element, such
/// as a native select that renders them inline. The search is confined to the control,
/// so an identical label in another control's list is never chosen.
async fn click_option_via_tree(
    scope: &PageScope,
    option: &str,
    target: &PageActionTarget,
) -> Result<String> {
    let option_node = resolve_option(scope, option, target).await?;
    let option_label = option_node.line_label();
    let option_path = option_node.path.join(" > ");
    click_target_node_with_root(&option_node, &option_label, &option_path, &target.root).await
}

/// Click an option in the list that is open, verifying the control took it.
///
/// The option is chosen by geometry: only a tree node whose position falls inside the
/// open list's own rectangle is accepted. That is what keeps the click on the right
/// control, because two dropdowns on one form routinely offer the same label and a
/// match by name alone picks whichever comes first on the page.
///
/// Activation prefers the accessibility action, which needs no coordinates and cannot
/// go stale, and falls back to a guarded physical click. The geometry is re-read on
/// every attempt, so a list that shifts as it opens is still hit correctly.
///
/// Returns None when the open list does not offer the option, so the caller can fall
/// back to a control whose options are not exposed as a list element.
async fn click_option_in_open_list(
    open: &crate::dom::OpenDropdown,
    option: &str,
    target: &PageActionTarget,
    notes: &[String],
) -> Result<Option<String>> {
    if !open
        .options
        .iter()
        .any(|candidate| option_matches(&candidate.text, option))
    {
        return Ok(None);
    }

    let mut last_summary = String::new();
    for attempt in 0..3u64 {
        let Some(current) = crate::dom::open_dropdown().await else {
            break;
        };
        let Some(option_point) = current
            .options
            .iter()
            .find(|candidate| option_matches(&candidate.text, option))
        else {
            return Ok(None);
        };

        let control_label = current.control.as_deref().unwrap_or(&target.label);
        match option_node_inside(option, &current, option_point, target).await? {
            // The tree describes this option, so its accessibility action is used:
            // no coordinates, so nothing can go stale.
            Some(node) => {
                let label = node.line_label();
                let path = node.path.join(" > ");
                let summary =
                    click_target_node_with_root(&node, &label, &path, &target.root).await?;
                last_summary = format!("{summary} in the open list of {control_label}");
            }
            // The tree does not describe this list's options, so the point the page
            // reported is used directly.
            None => {
                let browser_window = target.browser_window().await?;
                let relative_x = option_point.screen_x - browser_window.x;
                let relative_y = option_point.screen_y - browser_window.y;
                if relative_x < 0 || relative_y < 0 {
                    bail!(
                        "option {:?} in {} resolved to {},{} which is outside window {}",
                        option,
                        target.label,
                        option_point.screen_x,
                        option_point.screen_y,
                        browser_window.id
                    );
                }
                window::mousemove_click(&browser_window.id, relative_x, relative_y)?;
                window::settle_after_input().await;
                last_summary = format!(
                    "clicked {:?} at {},{} in the open list of {control_label}",
                    option_point.text, relative_x, relative_y
                );
            }
        }

        if control_shows(current.control_point, target, option).await {
            let mut summary = format!("selected option {option:?} via {last_summary}");
            if !notes.is_empty() {
                summary = format!("{summary} | {}", notes.join(", "));
            }
            return Ok(Some(summary));
        }

        if attempt + 1 < 3 {
            sleep(Duration::from_millis(200)).await;
        }
    }

    let observed = observed_value(target).await;
    bail!(
        "clicked option {:?} in the open list of {} but the control does not report it as its value; observed {:?}, expected {:?} ({})",
        option,
        target.label,
        observed,
        option,
        last_summary
    )
}

/// The tree node for an option that lies inside the open list.
///
/// Two dropdowns on a form offer the same labels, so the label alone cannot say which
/// node belongs to the list that is open. The list's own rectangle is what decides:
/// only a node positioned inside it is that list's option. Candidates are searched
/// inside the control first and across the page second, because a widget may render its
/// list as a sibling of the control rather than inside it.
async fn option_node_inside(
    option: &str,
    open: &crate::dom::OpenDropdown,
    option_point: &crate::dom::OpenOption,
    target: &PageActionTarget,
) -> Result<Option<LiveNode>> {
    const OPTION_ROLES: &[&str] = &["List Item", "Menu Item", "Option", "Table Cell"];

    let mut candidates = Vec::new();
    for role in OPTION_ROLES {
        let raw = format!("{}~{}", role, option);
        let Ok(parsed) = selector::Selector::parse(&raw) else {
            continue;
        };
        if let Ok(matches) =
            crate::inspect::resolve_within_scope(&target.node, std::slice::from_ref(&parsed)).await
        {
            candidates.extend(matches);
        }
        if let Ok(matches) =
            crate::inspect::resolve_within_scope(&target.root, std::slice::from_ref(&parsed)).await
        {
            candidates.extend(matches);
        }
    }

    let (list_x, list_y, list_w, list_h) = open.list_rect;
    const TOLERANCE: i32 = 8;
    for node in candidates {
        let Ok((x, y, width, height)) = crate::inspect::stable_extents(&node).await else {
            continue;
        };
        let centre_x = x + width / 2;
        let centre_y = y + height / 2;
        let inside_list = list_w > 0
            && centre_x >= list_x - TOLERANCE
            && centre_x <= list_x + list_w + TOLERANCE
            && centre_y >= list_y - TOLERANCE
            && centre_y <= list_y + list_h + TOLERANCE;
        let near_point = (centre_x - option_point.screen_x).abs() <= 24
            && (centre_y - option_point.screen_y).abs() <= 24;
        if inside_list || near_point {
            return Ok(Some(node));
        }
    }

    Ok(None)
}

/// Whether the list that is open belongs to the control the caller named.
///
/// A name is the strongest evidence, since it is what the caller selected by. When
/// the control has no accessible name -- the common case for an airport combo box --
/// position decides: an open list sits at its own control, and the tolerance is
/// generous enough for a list that is wider than its control while still far smaller
/// than the distance between two controls on one form.
async fn list_belongs_to_target(
    open: &crate::dom::OpenDropdown,
    target: &PageActionTarget,
) -> bool {
    if let (Some(list_control), Some(target_name)) =
        (open.control.as_deref(), target.node.name.as_deref())
        && !target_name.trim().is_empty()
        && option_matches(list_control, target_name)
    {
        return true;
    }

    let (Some((list_x, list_y)), Ok((target_x, target_y))) = (
        open.control_point,
        crate::inspect::clickable_point_stable(&target.node).await,
    ) else {
        // Nothing to compare, so the caller's own selector is the only guide and the
        // open list is accepted: refusing would break the case this exists for.
        return true;
    };

    const TOLERANCE: i32 = 80;
    (list_x - target_x).abs() <= TOLERANCE && (list_y - target_y).abs() <= TOLERANCE
}

/// Whether the control's own list opened, polling for the options to change.
///
/// One page query per poll: the visible option labels and the control's own expanded
/// state come back together, so a poll costs a single round trip. Only a positive
/// "closed" is treated as failure, because a native select exposes no expanded state
/// and refusing every selection on such a control would be worse than the problem
/// this guards against.
async fn list_opened(before: Option<&str>, target: &PageActionTarget) -> bool {
    let point = crate::inspect::clickable_point_stable(&target.node)
        .await
        .ok();
    let flavor = crate::dom::current_flavor();
    let mut said_closed = false;

    for attempt in 0..6u64 {
        let Some(state) = crate::dom::dropdown_state(flavor, point).await else {
            // The page could not be queried, so there is nothing to judge.
            return true;
        };

        let changed = match before {
            Some(before) => before != state.options,
            None => !state.options.is_empty(),
        };
        if changed && !state.options.is_empty() {
            return true;
        }

        match state.expanded {
            Some(true) => return true,
            Some(false) => said_closed = true,
            None => {}
        }

        if attempt + 1 < 6 {
            sleep(Duration::from_millis(120)).await;
        }
    }

    !said_closed
}

/// Find the option to click inside a control.
///
/// Dropdowns render their choices with different accessibility roles: a native select
/// exposes Menu Item, while a custom autocomplete usually exposes List Item. The roles
/// are tried in turn and the first with a match wins.
///
/// The search never leaves the control. A page keeps every dropdown's options in the
/// markup, so a match anywhere on the page is not evidence that the option belongs to
/// the control being acted on, and with two combo boxes offering the same labels a
/// page-wide search picks the first list and changes the wrong control.
async fn resolve_option(
    _scope: &PageScope,
    option: &str,
    target: &PageActionTarget,
) -> Result<LiveNode> {
    const OPTION_ROLES: &[&str] = &["List Item", "Menu Item", "Option", "Table Cell"];

    for role in OPTION_ROLES {
        let raw = format!("{}~{}", role, option);
        let Ok(parsed) = selector::Selector::parse(&raw) else {
            continue;
        };
        if let Ok(matches) = crate::inspect::resolve_within_scope(&target.node, &[parsed]).await
            && let Some(node) = matches.into_iter().next()
        {
            return Ok(node);
        }
    }

    Err(anyhow!(
        "no option matched {:?} inside {}; the list that is open does not offer it",
        option,
        target.label
    ))
}

/// Confirm the control now reports the option as its value.
async fn option_selected(target: &PageActionTarget, option: &str) -> bool {
    let option = option.trim().to_ascii_lowercase();
    if option.is_empty() {
        return true;
    }

    if option_confirmed_by_page(target, &option).await {
        return true;
    }

    // The control's own text, read over the accessibility tree. This is what covers a
    // control with no accessible name and no name-based page lookup: on a real booking
    // form both airport combos are unnamed, so the page lookups return nothing even
    // though the selection worked.
    if let Ok(Some(value)) = crate::live_access::read_text(&target.node).await
        && value.to_ascii_lowercase().contains(&option)
    {
        return true;
    }
    if let Some(value) = crate::inspect::node_text(&target.node).await
        && value.to_ascii_lowercase().contains(&option)
    {
        return true;
    }

    // The tree is consulted last and only as a secondary signal. It reports a radio
    // group and a custom dropdown loosely, so on its own it produced both false
    // negatives, a selection that worked reported as unverified, and false positives.
    crate::inspect::descendant_option_selected(&target.node, &option).await
}

/// Whether the page itself shows the option as the control's value.
///
/// This is the authoritative check, and the stricter one: it accepts only evidence
/// read from the page, so it is safe both for verifying a selection and for deciding
/// that a control already holds the wanted value.
///
/// Three lookups are tried because a control exposes its value differently depending
/// on how it is built: its accessible name, which a custom dropdown updates to the
/// chosen label; a page lookup by that name, which reads a native select's value and
/// selected option; and a page lookup at the control's own point, which is what covers
/// a control with no accessible name at all.
async fn option_confirmed_by_page(target: &PageActionTarget, option: &str) -> bool {
    if target
        .node
        .name
        .as_deref()
        .map(|name| name.to_ascii_lowercase().contains(option))
        .unwrap_or(false)
    {
        return true;
    }

    if let Some(value) = crate::dom::control_value(&target.node).await
        && value.to_ascii_lowercase().contains(option)
    {
        return true;
    }

    if let Ok((screen_x, screen_y)) = crate::inspect::clickable_point_stable(&target.node).await
        && let Some(value) =
            crate::dom::value_at_point(crate::dom::current_flavor(), screen_x, screen_y).await
        && value.to_ascii_lowercase().contains(option)
    {
        return true;
    }

    false
}

/// Whether the control now shows the option, read at the control itself.
///
/// The control's own point is what makes this work for an unnamed control, which is
/// the common case: both airport combos on a real booking form have no accessible
/// name, so a name-based lookup finds nothing even after a selection that worked.
async fn control_shows(
    control_point: Option<(i32, i32)>,
    target: &PageActionTarget,
    option: &str,
) -> bool {
    if option_selected(target, option).await {
        return true;
    }

    let Some((screen_x, screen_y)) = control_point else {
        return false;
    };
    crate::dom::value_at_point(crate::dom::current_flavor(), screen_x, screen_y)
        .await
        .map(|value| option_matches(&value, option))
        .unwrap_or(false)
}

/// Whether an option label answers to what the caller asked for.
///
/// An option is often shown with more than the caller typed -- an airline lists
/// "Lagos LOS" where the caller says "Lagos" -- so a case-insensitive containment in
/// either direction is accepted.
fn option_matches(candidate: &str, wanted: &str) -> bool {
    let candidate = candidate.trim().to_ascii_lowercase();
    let wanted = wanted.trim().to_ascii_lowercase();
    if wanted.is_empty() {
        return false;
    }
    candidate == wanted || candidate.contains(&wanted) || wanted.contains(&candidate)
}

/// Whatever the control currently reports as its value, for error reporting.
///
/// A message that only says "could not be verified" leaves the caller unable to tell a
/// wrong value from an unreadable one, so the observed value is read the same way
/// verification reads it.
async fn observed_value(target: &PageActionTarget) -> String {
    if let Some(value) = crate::dom::control_value(&target.node).await {
        return value;
    }
    if let Ok(Some(value)) = crate::live_access::read_text(&target.node).await {
        return value;
    }
    crate::inspect::node_text(&target.node)
        .await
        .unwrap_or_else(|| "<unreadable>".to_string())
}

async fn set_toggle(
    scope: &PageScope,
    raw_selectors: &[String],
    desired_checked: bool,
) -> Result<String> {
    let target = PageActionTarget::resolve(scope, raw_selectors).await?;
    let mut notes = Vec::new();
    if target.scroll_into_view().await? {
        notes.push("scrolled into view first".to_string());
    }

    let states = target.state_set().await?;
    let currently_checked = is_checked(states);
    if currently_checked == desired_checked {
        let state = if desired_checked {
            "already checked"
        } else {
            "already unchecked"
        };
        return Ok(attach_notes(
            format!("{} {} ({})", state, target.label, target.path),
            &notes,
        ));
    }

    let action_summary =
        click_target_node_with_root(&target.node, &target.label, &target.path, &target.root)
            .await?;
    if !wait_for_checked_state(scope, raw_selectors, desired_checked).await? {
        bail!(
            "toggle state for {} did not change to {}",
            target.label,
            if desired_checked {
                "checked"
            } else {
                "unchecked"
            }
        );
    }

    Ok(attach_notes(
        format!(
            "set {} {} ({}) | {}",
            if desired_checked {
                "checked"
            } else {
                "unchecked"
            },
            target.label,
            target.path,
            action_summary
        ),
        &notes,
    ))
}

fn is_checked(states: atspi::StateSet) -> bool {
    states.contains(State::Checked)
        || states.contains(State::Selected)
        || states.contains(State::Pressed)
}

async fn wait_for_checked_state(
    scope: &PageScope,
    raw_selectors: &[String],
    desired_checked: bool,
) -> Result<bool> {
    for _ in 0..15 {
        let target = PageActionTarget::resolve(scope, raw_selectors).await?;
        if is_checked(target.state_set().await?) == desired_checked {
            return Ok(true);
        }
        sleep(Duration::from_millis(100)).await;
    }
    Ok(false)
}

fn attach_notes(summary: String, notes: &[String]) -> String {
    if notes.is_empty() {
        summary
    } else {
        format!("{} | {}", summary, notes.join(", "))
    }
}
