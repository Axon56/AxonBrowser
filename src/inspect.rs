use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_recursion::async_recursion;
use atspi::{
    AccessibilityConnection, ObjectRefOwned,
    connection::{read_session_accessibility, set_session_accessibility},
    proxy::{
        CoordType,
        accessible::{AccessibleProxy, ObjectRefExt},
        component::ComponentProxy,
        proxy_ext::ProxyExt,
    },
};

use crate::{
    model::{LiveNode, UiNode, line_label},
    selector::Selector,
};

use tokio::time::sleep;

pub async fn resolve(query: &str, selectors: &[Selector]) -> Result<Vec<LiveNode>> {
    let root = resolve_root(query).await?;
    resolve_within_scopes(vec![root], selectors).await
}

pub async fn inspect_live(root: &LiveNode) -> Result<UiNode> {
    ensure_accessibility_enabled().await?;

    let connection = connect_accessibility().await?;
    let accessible = root
        .object_ref
        .as_accessible_proxy(connection.connection())
        .await
        .context("failed to bind live node for tree inspection")?;

    build_tree(&accessible, connection.connection()).await
}

pub async fn resolve_within_scope(
    scope: &LiveNode,
    selectors: &[Selector],
) -> Result<Vec<LiveNode>> {
    resolve_within_scopes(vec![scope.clone()], selectors).await
}

pub async fn descendants(scope: &LiveNode) -> Result<Vec<LiveNode>> {
    let connection = connect_accessibility().await?;

    let mut nodes = Vec::new();
    collect_descendants(scope, connection.connection(), &mut nodes).await?;
    Ok(nodes)
}

pub async fn clickable_point(node: &LiveNode) -> Result<(i32, i32)> {
    let component = bind_component(node).await?;
    read_clickable_point(&component).await
}

pub async fn component_extents(node: &LiveNode) -> Result<(i32, i32, i32, i32)> {
    let component = bind_component(node).await?;
    component
        .get_extents(CoordType::Screen)
        .await
        .context("failed to read component extents")
}

pub async fn read_state_set(node: &LiveNode) -> Result<atspi::StateSet> {
    let connection = connect_accessibility().await?;
    let accessible = node
        .object_ref
        .as_accessible_proxy(connection.connection())
        .await
        .context("failed to bind matched node for state lookup")?;
    accessible
        .get_state()
        .await
        .context("failed to read AT-SPI state set for matched node")
}

async fn resolve_root(query: &str) -> Result<LiveNode> {
    ensure_accessibility_enabled().await?;

    let connection = connect_accessibility().await?;

    let registry = connection
        .root_accessible_on_registry()
        .await
        .context("failed to get the AT-SPI registry root")?;

    let applications = registry
        .get_children()
        .await
        .context("failed to list desktop applications from the AT-SPI registry")?;

    let needle = normalize_query(query)?;

    for app_ref in applications {
        if app_ref.is_null() {
            continue;
        }

        let app = app_ref
            .as_accessible_proxy(connection.connection())
            .await
            .with_context(|| format!("failed to bind app proxy for {}", debug_ref(&app_ref)))?;

        if let Some(found) = find_matching_live_node(&app, &needle, connection.connection()).await?
        {
            return Ok(found);
        }
    }

    Err(anyhow!(
        "no accessible application or window matched query {:?}",
        query
    ))
}

async fn resolve_within_scopes(
    mut scopes: Vec<LiveNode>,
    selectors: &[Selector],
) -> Result<Vec<LiveNode>> {
    if selectors.is_empty() {
        return Ok(scopes);
    }

    let connection = connect_accessibility().await?;

    for selector in selectors {
        let mut next = Vec::new();
        for scope in &scopes {
            collect_descendant_matches(scope, selector, connection.connection(), &mut next).await?;
        }
        scopes = next;
        if scopes.is_empty() {
            break;
        }
    }

    Ok(scopes)
}

async fn ensure_accessibility_enabled() -> Result<()> {
    let enabled = read_session_accessibility()
        .await
        .context("failed to read AT-SPI IsEnabled status from the session bus")?;

    if !enabled {
        set_session_accessibility(true)
            .await
            .context("failed to enable AT-SPI accessibility on the session bus")?;
    }

    Ok(())
}

/// How long a single attempt to reach the accessibility bus may take.
///
/// Connecting is a D-Bus round trip, and a bus that is half-dead -- reachable but
/// not answering -- would otherwise block forever, which makes a command hang
/// with no output and no way to tell a slow page from a dead bus. Bounding the
/// attempt turns that into a prompt, reportable failure.
const ACCESSIBILITY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Connect to the accessibility bus, repairing the stack once if needed.
///
/// Public because the click paths in every browser module need the same bounded
/// behaviour; connecting directly bypasses both the timeout and the repair.
pub async fn connect_accessibility() -> Result<AccessibilityConnection> {
    match tokio::time::timeout(
        ACCESSIBILITY_CONNECT_TIMEOUT,
        AccessibilityConnection::new(),
    )
    .await
    {
        Ok(Ok(connection)) => Ok(connection),
        // A timed-out attempt is treated like a failed one, so the repair path
        // runs and the caller gets a real error rather than a stall.
        Ok(Err(_)) | Err(_) => {
            crate::runtime::repair_accessibility_stack()
                .context("failed to repair the AT-SPI accessibility stack")?;
            tokio::time::timeout(
                ACCESSIBILITY_CONNECT_TIMEOUT,
                AccessibilityConnection::new(),
            )
            .await
            .context("timed out connecting to the AT-SPI accessibility bus")?
            .context("failed to connect to the AT-SPI accessibility bus")
        }
    }
}

#[async_recursion]
async fn find_matching_live_node(
    node: &AccessibleProxy<'_>,
    needle: &str,
    conn: &atspi::zbus::Connection,
) -> Result<Option<LiveNode>> {
    let role = read_role(node).await;
    let name = read_name(node).await;
    let label = line_label(&role, name.as_deref());

    if name
        .as_deref()
        .is_some_and(|value| matches_query(value, needle))
    {
        return Ok(Some(LiveNode {
            object_ref: ObjectRefOwned::try_from(node)
                .context("failed to extract object ref for matched root")?,
            role,
            name,
            path: vec![label],
        }));
    }

    let children = node
        .get_children()
        .await
        .with_context(|| format!("failed to read children for {}", label))?;

    for child_ref in children {
        if child_ref.is_null() {
            continue;
        }

        let child = child_ref
            .as_accessible_proxy(conn)
            .await
            .with_context(|| format!("failed to bind child proxy for {}", debug_ref(&child_ref)))?;

        if let Some(found) = find_matching_live_node(&child, needle, conn).await? {
            return Ok(Some(found));
        }
    }

    Ok(None)
}

#[async_recursion]
async fn collect_descendant_matches(
    scope: &LiveNode,
    selector: &Selector,
    conn: &atspi::zbus::Connection,
    matches: &mut Vec<LiveNode>,
) -> Result<()> {
    let accessible = scope
        .object_ref
        .as_accessible_proxy(conn)
        .await
        .with_context(|| {
            format!(
                "failed to bind scope proxy while traversing descendants for {}",
                scope.line_label()
            )
        })?;

    let children = accessible
        .get_children()
        .await
        .with_context(|| format!("failed to read children for {}", scope.line_label()))?;

    for child_ref in children {
        if child_ref.is_null() {
            continue;
        }

        let child = child_ref
            .as_accessible_proxy(conn)
            .await
            .with_context(|| format!("failed to bind child proxy for {}", debug_ref(&child_ref)))?;

        let role = read_role(&child).await;
        let name = read_name(&child).await;
        let label = line_label(&role, name.as_deref());
        let path = extend_path(&scope.path, &label);

        let live = LiveNode {
            object_ref: child_ref.clone(),
            role,
            name,
            path: path.clone(),
        };

        if selector.matches_live(&live) {
            matches.push(live.clone());
        }

        collect_descendant_matches(&live, selector, conn, matches).await?;
    }

    Ok(())
}

#[async_recursion]
async fn collect_descendants(
    scope: &LiveNode,
    conn: &atspi::zbus::Connection,
    descendants: &mut Vec<LiveNode>,
) -> Result<()> {
    let accessible = scope
        .object_ref
        .as_accessible_proxy(conn)
        .await
        .with_context(|| {
            format!(
                "failed to bind scope proxy while traversing descendants for {}",
                scope.line_label()
            )
        })?;

    let children = accessible
        .get_children()
        .await
        .with_context(|| format!("failed to read children for {}", scope.line_label()))?;

    for child_ref in children {
        if child_ref.is_null() {
            continue;
        }

        let child = child_ref
            .as_accessible_proxy(conn)
            .await
            .with_context(|| format!("failed to bind child proxy for {}", debug_ref(&child_ref)))?;

        let role = read_role(&child).await;
        let name = read_name(&child).await;
        let label = line_label(&role, name.as_deref());
        let path = extend_path(&scope.path, &label);

        let live = LiveNode {
            object_ref: child_ref.clone(),
            role,
            name,
            path,
        };

        descendants.push(live.clone());
        collect_descendants(&live, conn, descendants).await?;
    }

    Ok(())
}

#[async_recursion]
async fn build_tree(node: &AccessibleProxy<'_>, conn: &atspi::zbus::Connection) -> Result<UiNode> {
    let mut budget = TreeBudget::new();
    build_tree_inner(node, conn, &mut budget).await
}

#[async_recursion]
async fn build_tree_inner(
    node: &AccessibleProxy<'_>,
    conn: &atspi::zbus::Connection,
    budget: &mut TreeBudget,
) -> Result<UiNode> {
    let role = read_role(node).await;
    let name = read_name(node).await;

    // A reference already on this branch is a true cycle. A reference seen
    // elsewhere is walked again: Chrome recycles references for virtualized and
    // re-rendered nodes, so a second appearance can have genuinely different
    // children, and skipping it is what used to drop whole subtrees.
    let key = node_key(node);
    if budget.on_path.contains(&key) || !budget.charge() {
        return Ok(UiNode::new(role, name, Vec::new()));
    }
    budget.on_path.push(key);

    let children_refs = node.get_children().await.with_context(|| {
        format!(
            "failed to read children for {}",
            line_label(&role, name.as_deref())
        )
    })?;

    let mut children = Vec::new();
    for child_ref in children_refs {
        if child_ref.is_null() {
            continue;
        }

        let child = child_ref
            .as_accessible_proxy(conn)
            .await
            .with_context(|| format!("failed to bind child proxy for {}", debug_ref(&child_ref)))?;
        children.push(build_tree_inner(&child, conn, budget).await?);
    }

    budget.on_path.pop();
    Ok(UiNode::new(role, name, children))
}

/// Bounds a tree walk without dropping nodes.
///
/// Two limits, each catching a different failure:
///
/// * `on_path` holds the references on the current branch, which detects a true
///   cycle. It is scoped to the path rather than the whole walk because Chrome
///   recycles references for virtualized and re-rendered nodes: a single global
///   visited set emitted the second appearance without its children, which is how
///   a search input inside an open dropdown went missing from the tree.
/// * `deadline` caps wall-clock time. Each node costs several D-Bus round trips,
///   so a node count alone is not a time bound, and a walk that overruns must
///   return a partial tree rather than hang the command.
///
/// There is deliberately no per-reference expansion cap. Counting visits sounds
/// safer but truncates real pages: a node that legitimately reappears through a
/// recycled reference loses its subtree on the third appearance, and a page that
/// normally exposes hundreds of controls comes back with a fraction of them. The
/// node budget and the deadline already bound the work.
struct TreeBudget {
    on_path: Vec<String>,
    remaining: usize,
    deadline: std::time::Instant,
}

impl TreeBudget {
    /// Total nodes the walk may emit. Sized so a real page is never truncated:
    /// the pages this tool drives expose a few hundred to a few thousand nodes.
    const MAX_NODES: usize = 20_000;
    /// Wall-clock budget for one walk.
    const BUDGET: Duration = Duration::from_secs(20);

    fn new() -> Self {
        Self {
            on_path: Vec::new(),
            remaining: Self::MAX_NODES,
            deadline: std::time::Instant::now() + Self::BUDGET,
        }
    }

    /// Whether another node may be walked, consuming budget when it may.
    fn charge(&mut self) -> bool {
        if self.remaining == 0 || std::time::Instant::now() >= self.deadline {
            return false;
        }
        self.remaining -= 1;
        true
    }
}

async fn read_clickable_point(component: &ComponentProxy<'_>) -> Result<(i32, i32)> {
    let (x, y, width, height) = component
        .get_extents(CoordType::Screen)
        .await
        .context("failed to read component extents")?;

    if width <= 0 || height <= 0 {
        return Err(anyhow!("matched node has non-visible extents"));
    }

    Ok((x + (width / 2), y + (height / 2)))
}

async fn read_role(node: &AccessibleProxy<'_>) -> String {
    node.get_role_name()
        .await
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(title_case_role)
        .unwrap_or_else(|| "Unknown".to_string())
}

async fn read_name(node: &AccessibleProxy<'_>) -> Option<String> {
    node.name()
        .await
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn matches_query(name: &str, needle: &str) -> bool {
    normalize_for_match(name).contains(needle)
}

fn normalize_query(query: &str) -> Result<String> {
    let needle = normalize_for_match(query);
    if needle.is_empty() {
        return Err(anyhow!("query must not be empty"));
    }
    Ok(needle)
}

/// Read component extents and wait until they stop moving.
///
/// Scrolling is asynchronous: `scroll_to` returns before the page has finished
/// settling, so extents read immediately after a scroll can still describe the
/// pre-scroll or mid-animation position. Poll until two consecutive reads agree
/// so callers act on the position the element actually occupies now.
pub async fn stable_extents(node: &LiveNode) -> Result<(i32, i32, i32, i32)> {
    let mut previous: Option<(i32, i32, i32, i32)> = None;
    let mut last: Option<(i32, i32, i32, i32)> = None;

    // A page with `scroll-behavior: smooth` animates a scroll for hundreds of
    // milliseconds, so the first reads after a scroll still describe the old
    // position. Waiting for two consecutive reads to agree is what makes the
    // coordinates trustworthy, so the loop is long enough to outlast an
    // animation rather than giving up while the target is still moving.
    for attempt in 0..12u64 {
        let extents = component_extents(node).await?;
        if previous == Some(extents) {
            return Ok(extents);
        }
        previous = Some(extents);
        last = Some(extents);
        sleep(Duration::from_millis(50 + (attempt * 25).min(200))).await;
    }

    last.ok_or_else(|| anyhow!("failed to read component extents"))
}

/// Center of the node from freshly settled extents, in screen coordinates.
pub async fn clickable_point_stable(node: &LiveNode) -> Result<(i32, i32)> {
    let (x, y, width, height) = stable_extents(node).await?;
    if width <= 0 || height <= 0 {
        return Err(anyhow!("matched node has non-visible extents"));
    }
    Ok((x + (width / 2), y + (height / 2)))
}

/// Ask the accessibility tree which element actually occupies a screen point.
///
/// Called on a high-level node (the page root or window) so the answer is the
/// topmost element at that point, which is how an overlay is detected.
pub async fn accessible_at_point(
    node: &LiveNode,
    x: i32,
    y: i32,
) -> Result<Option<ObjectRefOwned>> {
    let component = match bind_component(node).await {
        Ok(component) => component,
        Err(_) => return Ok(None),
    };

    match component
        .get_accessible_at_point(x, y, CoordType::Screen)
        .await
    {
        Ok(reference) if !reference.is_null() => Ok(Some(reference)),
        _ => Ok(None),
    }
}

/// Stable identity for an accessibility object reference.
pub fn ref_key(reference: &ObjectRefOwned) -> String {
    format!(
        "{}|{}",
        reference.name_as_str().unwrap_or_default(),
        reference.path_as_str()
    )
}

/// Accessible name of an object reference, if it has one.
pub fn ref_name(reference: &ObjectRefOwned) -> &str {
    reference.name_as_str().unwrap_or_default()
}

/// Screen extents of an accessibility object reference.
pub async fn extents_of_ref(reference: &ObjectRefOwned) -> Result<(i32, i32, i32, i32)> {
    let connection = connect_accessibility().await?;
    let accessible = reference
        .as_accessible_proxy(connection.connection())
        .await
        .context("failed to bind object reference for extents lookup")?;
    let proxies = accessible
        .proxies()
        .await
        .context("failed to inspect object reference interfaces")?;
    let component = proxies
        .component()
        .await
        .context("object reference does not expose Component interface")?;
    component
        .get_extents(CoordType::Screen)
        .await
        .context("failed to read object reference extents")
}

/// Whether a node currently occupies space on screen.
///
/// Hidden duplicates are common: a page with two date pickers exposes two
/// identical day cells, and only one is showing. Acting on the hidden one looks
/// like success while changing nothing, so callers use this to prefer the
/// element the user can actually see.
pub async fn is_showing(node: &LiveNode) -> bool {
    if let Ok((_, _, width, height)) = component_extents(node).await
        && width > 0
        && height > 0
    {
        return true;
    }

    read_state_set(node)
        .await
        .map(|states| {
            states.contains(atspi::State::Showing) || states.contains(atspi::State::Visible)
        })
        .unwrap_or(false)
}

/// Whether the accessibility tree considers this node able to be activated.
///
/// A control that cannot take focus is very likely disabled or decorative, and
/// activating it does nothing. AT-SPI is not fully reliable here -- some pages
/// report a disabled control as enabled and sensitive -- so this qualifies a
/// success report rather than refusing the click.
pub async fn is_actionable(node: &LiveNode) -> bool {
    match read_state_set(node).await {
        Ok(states) => states.contains(atspi::State::Focusable),
        // Unreadable state is not evidence of a problem.
        Err(_) => true,
    }
}

/// A caveat to append when a click could not be confirmed as having an effect.
pub async fn unverified_note(node: &LiveNode) -> Option<String> {
    if is_actionable(node).await {
        return None;
    }

    Some(format!(
        "could not verify {} is actionable: the accessibility tree does not report it as focusable, so the click may have had no effect",
        node.line_label()
    ))
}

/// Label of a modal dialog that is hiding the rest of the page, if any.
///
/// When a modal opens, pages commonly mark the content behind it `aria-hidden`
/// or `inert`. The browser then correctly removes that content from the
/// accessibility tree, so the tree exposes only the dialog and every page
/// lookup fails. Reporting the dialog turns a confusing "no page matches" into
/// an actionable message, because the dialog's own controls are still present
/// and can be used to dismiss it.
pub async fn blocking_dialog(root: &LiveNode) -> Option<BlockingDialog> {
    let children = descendant_refs(root, 2).await.ok()?;
    if children.is_empty() {
        return None;
    }

    // A covering element is not always exposed as a dialog. Site builders wrap
    // their popups in frames and panels, and one was reported covering the page
    // as a `Document Frame`. Matching every such role is not an option either:
    // ordinary content sections carry those same roles, so treating them as
    // modals dismissed the page itself and refused clicks on plain content.
    //
    // The page is asked what is actually covering it, and that answer decides.
    // It reports only a declared modal dialog or a full-viewport fixed layer, so
    // a sticky header and a content section are both correctly ignored.
    let covered = crate::dom::covering_overlay().await;
    covered.as_ref()?;

    // The page knows a cover exists; the tree says what it is and, more
    // importantly, which control closes it. The cover is matched by name so the
    // right node is picked when several dialogs are in the markup.
    let wanted = covered.as_deref().unwrap_or_default();
    let wanted_name = wanted
        .split_once('"')
        .and_then(|(_, rest)| rest.strip_suffix('"'))
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    let mut fallback: Option<BlockingDialog> = None;
    for reference in &children {
        let Some(role) = ref_role(reference).await else {
            continue;
        };
        if role != "dialog" && role != "alert dialog" && role != "document frame" && role != "frame"
        {
            continue;
        }

        // `ObjectRefOwned::name_as_str` is the D-Bus sender, not the accessible
        // name, so read the real name through the proxy.
        let name = accessible_name(reference).await;
        let candidate = BlockingDialog {
            label: line_label(&role, name.as_deref()),
            dismiss_control: dialog_dismiss_control(reference).await,
        };

        // Prefer the node the page named, when it has a name to match on.
        let name_matches = !wanted_name.is_empty()
            && name
                .as_deref()
                .map(|name| name.to_ascii_lowercase().contains(&wanted_name))
                .unwrap_or(false);
        if name_matches && candidate.dismiss_control.is_some() {
            return Some(candidate);
        }

        // Otherwise anything we can actually close will do, since the page has
        // already confirmed that something is covering it.
        if candidate.dismiss_control.is_some() {
            return Some(candidate);
        }
        if fallback.is_none() {
            fallback = Some(candidate);
        }
    }

    // Nothing in the tree carries a close control, so report the cover by the
    // name the page gave it rather than pretending there is nothing there.
    fallback.or_else(|| {
        Some(BlockingDialog {
            label: wanted.to_string(),
            dismiss_control: None,
        })
    })
}

/// A modal that is hiding the page behind it.
#[derive(Debug, Clone)]
pub struct BlockingDialog {
    pub label: String,
    /// The dialog's own close control, which stays reachable while the dialog
    /// is open even though the rest of the page does not.
    pub dismiss_control: Option<ObjectRefOwned>,
}

/// Find a control inside a dialog that dismisses it.
async fn dialog_dismiss_control(dialog: &ObjectRefOwned) -> Option<ObjectRefOwned> {
    const DISMISS_WORDS: &[&str] = &[
        "close",
        "dismiss",
        "accept",
        "reject",
        "cancel",
        "no thanks",
        "not now",
        "×",
        "✕",
    ];

    let live = LiveNode {
        object_ref: dialog.clone(),
        role: String::new(),
        name: None,
        path: Vec::new(),
    };
    let descendants = descendant_refs(&live, 4).await.ok()?;

    for reference in descendants {
        let role = ref_role(&reference).await.unwrap_or_default();
        if role != "push button" && role != "link" {
            continue;
        }
        let Some(name) = accessible_name(&reference).await else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if DISMISS_WORDS.iter().any(|word| name.contains(word)) {
            return Some(reference);
        }
    }

    None
}

/// Invoke an object reference's default accessibility action.
pub async fn invoke_action(reference: &ObjectRefOwned) -> bool {
    let Ok(connection) = connect_accessibility().await else {
        return false;
    };
    let Ok(accessible) = reference.as_accessible_proxy(connection.connection()).await else {
        return false;
    };
    let Ok(proxies) = accessible.proxies().await else {
        return false;
    };
    let Ok(action) = proxies.action().await else {
        return false;
    };
    action.do_action(0).await.unwrap_or(false)
}

/// Accessible name of an object reference, read through its proxy.
pub async fn accessible_name(reference: &ObjectRefOwned) -> Option<String> {
    let connection = connect_accessibility().await.ok()?;
    let accessible = reference
        .as_accessible_proxy(connection.connection())
        .await
        .ok()?;
    read_name(&accessible).await
}

/// Accessible role of an object reference, in the same casing as tree output.
pub async fn ref_role(reference: &ObjectRefOwned) -> Option<String> {
    let connection = connect_accessibility().await.ok()?;
    let accessible = reference
        .as_accessible_proxy(connection.connection())
        .await
        .ok()?;
    Some(read_role(&accessible).await.to_ascii_lowercase())
}

/// First match that is actually on screen, if any.
pub async fn first_showing(matches: &[LiveNode]) -> Option<LiveNode> {
    for candidate in matches {
        if is_showing(candidate).await {
            return Some(candidate.clone());
        }
    }
    None
}

/// Best-effort visible text for a node: its own name plus its descendants'.
///
/// Custom widgets often expose their value as a child `Static` node rather than
/// through the AT-SPI text interface, so verification cannot rely on
/// `read_text` alone.
pub async fn node_text(node: &LiveNode) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(name) = node.name.as_deref() {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }

    for descendant in descendant_refs(node, 3).await.unwrap_or_default() {
        if let Some(name) = descendant.name_as_str() {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Whether a descendant whose name matches `option` reports itself as selected.
///
/// Native selects report their value as an unhelpful placeholder, so selection
/// is verified from the state of the matching option node instead of from text.
pub async fn descendant_option_selected(node: &LiveNode, option: &str) -> bool {
    let option = option.trim().to_ascii_lowercase();
    if option.is_empty() {
        return true;
    }

    for descendant in descendant_refs(node, 3).await.unwrap_or_default() {
        let Some(name) = descendant.name_as_str() else {
            continue;
        };
        if !name.to_ascii_lowercase().contains(&option) {
            continue;
        }

        let live = LiveNode {
            object_ref: descendant,
            role: String::new(),
            name: None,
            path: Vec::new(),
        };
        if let Ok(states) = read_state_set(&live).await
            && (states.contains(atspi::State::Selected)
                || states.contains(atspi::State::Checked)
                || states.contains(atspi::State::Focused)
                || states.contains(atspi::State::Pressed))
        {
            return true;
        }
    }

    false
}

/// Role and name of an object reference, for describing what blocked a click.
pub async fn describe_ref(reference: &ObjectRefOwned) -> String {
    let Ok(connection) = connect_accessibility().await else {
        return "<unavailable>".to_string();
    };
    let Ok(accessible) = reference.as_accessible_proxy(connection.connection()).await else {
        return "<unavailable>".to_string();
    };

    let role = read_role(&accessible).await;
    let name = read_name(&accessible).await;
    line_label(&role, name.as_deref())
}

/// Collect the references of every descendant, bounded by depth.
pub async fn descendant_refs(node: &LiveNode, max_depth: usize) -> Result<Vec<ObjectRefOwned>> {
    let connection = connect_accessibility().await?;
    let mut refs = Vec::new();
    collect_descendant_refs(node, connection.connection(), max_depth, &mut refs).await?;
    Ok(refs)
}

#[async_recursion]
async fn collect_descendant_refs(
    node: &LiveNode,
    conn: &atspi::zbus::Connection,
    max_depth: usize,
    refs: &mut Vec<ObjectRefOwned>,
) -> Result<()> {
    if max_depth == 0 {
        return Ok(());
    }

    let Ok(accessible) = node.object_ref.as_accessible_proxy(conn).await else {
        return Ok(());
    };
    let Ok(children) = accessible.get_children().await else {
        return Ok(());
    };

    for child_ref in children {
        if child_ref.is_null() {
            continue;
        }
        refs.push(child_ref.clone());
        let child = LiveNode {
            object_ref: child_ref,
            role: String::new(),
            name: None,
            path: node.path.clone(),
        };
        collect_descendant_refs(&child, conn, max_depth - 1, refs).await?;
    }

    Ok(())
}

async fn bind_component(node: &LiveNode) -> Result<ComponentProxy<'_>> {
    let connection = connect_accessibility().await?;
    let accessible = node
        .object_ref
        .as_accessible_proxy(connection.connection())
        .await
        .context("failed to bind matched node for component lookup")?;
    let proxies = accessible
        .proxies()
        .await
        .context("failed to inspect matched node interfaces")?;
    proxies
        .component()
        .await
        .context("matched node does not expose Component interface")
}

fn normalize_for_match(input: &str) -> String {
    strip_invisible_format_chars(input)
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_invisible_format_chars(input: &str) -> String {
    input
        .chars()
        .filter(|ch| !matches!(ch, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}'))
        .collect()
}

fn title_case_role(role: String) -> String {
    role.split_whitespace()
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = String::new();
                    out.extend(first.to_uppercase());
                    out.push_str(chars.as_str());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn extend_path(path: &[String], label: &str) -> Vec<String> {
    let mut next = path.to_vec();
    next.push(label.to_string());
    next
}

fn node_key(node: &AccessibleProxy<'_>) -> String {
    match ObjectRefOwned::try_from(node) {
        Ok(object_ref) => debug_ref(&object_ref),
        Err(_) => "<unknown-object-ref>".to_string(),
    }
}

fn debug_ref(node: &ObjectRefOwned) -> String {
    format!(
        "{} {}",
        node.name_as_str().unwrap_or("<no-name>"),
        node.path_as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::normalize_for_match;

    #[test]
    fn strips_zero_width_characters_for_matching() {
        let with_hidden = "Microsoft\u{200b} Edge";
        assert_eq!(normalize_for_match(with_hidden), "microsoft edge");
    }
}
