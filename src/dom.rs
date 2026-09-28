//! DOM-level checks for facts the accessibility tree cannot express.
//!
//! The accessibility tree is the tool's primary interface, but some widget
//! state never reaches it. A disabled calendar day, for example, is reported as
//! `Enabled | Sensitive | Showing | Visible` exactly like a selectable one, and
//! its action interface still reports success when invoked, so an AT-SPI check
//! cannot tell them apart and the click is reported as a success that changed
//! nothing.
//!
//! These helpers query the page directly, through Chromium DevTools for Chrome
//! and Edge and through WebDriver BiDi for Firefox, and are used only to
//! *verify* an action that the accessibility tree could not confirm.

pub use crate::browser_flavor::BrowserFlavor as Flavor;

/// Which browser the current session is driving.
///
/// Firefox records a BiDi port, and Edge is flagged by the same environment
/// variable the window queries already use; otherwise this is Chrome. A
/// remembered port is only trusted while something is listening on it, because
/// a stale file from an earlier session would otherwise make a Chrome session
/// answer DOM questions over Firefox's transport.
pub fn current_flavor() -> Flavor {
    if let Some(port) = crate::firefox::session::read_browser_port()
        && std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
    {
        return Flavor::Firefox;
    }
    if std::env::var("GUIBOT_BROWSER_WINDOW_MODE").ok().as_deref() == Some("edge") {
        return Flavor::Edge;
    }
    Flavor::Chrome
}

/// Whether the element under a screen point is disabled, or `None` when the
/// page cannot be queried.
///
/// Screen coordinates are converted to viewport coordinates inside the page, so
/// callers can pass the same point they are about to click.
pub async fn element_at_point(flavor: Flavor, screen_x: i32, screen_y: i32) -> Option<String> {
    // Built by substitution rather than a format string, because a format string
    // makes every JavaScript brace need escaping and a mistake there silently
    // produces invalid JavaScript.
    let expression = ELEMENT_AT_POINT_JS
        .replace("__X__", &screen_x.to_string())
        .replace("__Y__", &screen_y.to_string());

    let value = match flavor {
        Flavor::Chrome => crate::chrome::devtools::evaluate(&expression).await,
        Flavor::Edge => crate::edge::devtools::evaluate(&expression).await,
        Flavor::Firefox => crate::firefox::bidi::evaluate(&expression).await,
    }
    .ok()?;

    value.as_str().map(str::to_string)
}

/// Refuse to click an element the page reports as disabled.
///
/// The accessibility tree exposes a disabled control exactly like an enabled
/// one, and its action interface still reports success when invoked, so a
/// disabled target has to be caught in the page itself.
///
/// The element is located by accessible name rather than by coordinates: the
/// geometry reported for these controls can be stale, and a name lookup is
/// stable and also works for the accessibility action path, which never uses
/// coordinates at all. When several elements share the name, the click is only
/// refused if every one of them is disabled.
pub async fn refuse_if_disabled(node: &crate::model::LiveNode, label: &str) -> anyhow::Result<()> {
    let Some(name) = node
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    else {
        return Ok(());
    };

    let expression = DISABLED_BY_NAME_JS.replace("__NAME__", &js_string(name));
    let state = match current_flavor() {
        Flavor::Chrome => crate::chrome::devtools::evaluate(&expression).await,
        Flavor::Edge => crate::edge::devtools::evaluate(&expression).await,
        Flavor::Firefox => crate::firefox::bidi::evaluate(&expression).await,
    };
    let Some(state) = state
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
    else {
        return Ok(());
    };

    if state_blocks_click(&state) {
        anyhow::bail!(
            "{} is {state} in the page, so clicking it would have no effect",
            label
        );
    }

    Ok(())
}

/// Quote a string for embedding in JavaScript source.
fn js_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

/// Whether the named control is checked, read from the page.
///
/// A radio button's state is the observable effect of clicking it, and the
/// accessibility action interface reports success whether or not anything
/// happened: on a real booking form, activating the Round Trip radio answered
/// "clicked" while the page still showed the other option selected. Reading the
/// control's own state is what turns that into an honest answer.
///
/// Returns `None` when no control matches the name, so an unknown name is not
/// mistaken for "unchecked".
pub async fn control_checked(node: &crate::model::LiveNode) -> Option<bool> {
    let name = node
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())?;
    let expression = CHECKED_BY_NAME_JS.replace("__NAME__", &js_string(name));
    evaluate(current_flavor(), &expression).await?.as_bool()
}

/// Report whether the control with the given accessible name is checked.
///
/// The name is resolved the way the browser builds it, because a radio is
/// usually named by the label beside it rather than by an attribute, and the
/// state is read from whichever element in the group actually carries it.
const CHECKED_BY_NAME_JS: &str = r#"(function(){
  var want = __NAME__.trim().toLowerCase();
  if (!want) return null;

  function labelledBy(el) {
    var ids = (el.getAttribute('aria-labelledby') || '').trim();
    if (ids) {
      var parts = [];
      ids.split(/\s+/).forEach(function (id) {
        var target = document.getElementById(id);
        if (target) parts.push((target.textContent || '').trim());
      });
      if (parts.length) return parts.join(' ');
    }
    if (el.id) {
      var forLabel = document.querySelector('label[for="' + el.id.replace(/"/g, '\\"') + '"]');
      if (forLabel) return (forLabel.textContent || '').trim();
    }
    var wrapped = el.closest ? el.closest('label') : null;
    if (wrapped) return (wrapped.textContent || '').trim();
    return '';
  }

  function nameOf(el) {
    return (el.getAttribute('aria-label') || labelledBy(el) || '').trim();
  }

  var nodes = document.querySelectorAll('input, [role=radio], [role=checkbox], [role=switch], button');
  for (var i = 0; i < nodes.length; i++) {
    if (nameOf(nodes[i]).toLowerCase() !== want) continue;
    if (nodes[i].getClientRects().length === 0) continue;
    if (nodes[i].checked !== undefined && nodes[i].checked !== null) return !!nodes[i].checked;
    var aria = nodes[i].getAttribute('aria-checked');
    if (aria === 'true') return true;
    if (aria === 'false') return false;
  }
  return null;
})()"#;

/// Whether a control's dropdown is open, read from the page.
///
/// Opening the list is the observable effect of clicking a combo box, and a
/// custom one is often a `div` that never takes focus, so focus cannot be used
/// to confirm the click. The page is asked whether the control reports itself
/// expanded, or whether a listbox tied to it is showing.
///
/// Returns `None` when the control cannot be found, so an unknown name is not
/// read as "closed".
pub async fn control_expanded(node: &crate::model::LiveNode) -> Option<bool> {
    let name = node
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())?;
    let expression = EXPANDED_BY_NAME_JS.replace("__NAME__", &js_string(name));
    evaluate(current_flavor(), &expression).await?.as_bool()
}

/// Report whether the control with the given accessible name has its list open.
///
/// Three signals are checked because dropdowns are built differently: an
/// `aria-expanded` attribute, an open `<select>` whose size was raised, and a
/// visible listbox that the control points at with `aria-controls` or that sits
/// inside the same wrapper.
const EXPANDED_BY_NAME_JS: &str = r#"(function(){
  var want = __NAME__.trim().toLowerCase();
  if (!want) return null;

  function labelledBy(el) {
    var ids = (el.getAttribute('aria-labelledby') || '').trim();
    if (ids) {
      var parts = [];
      ids.split(/\s+/).forEach(function (id) {
        var target = document.getElementById(id);
        if (target) parts.push((target.textContent || '').trim());
      });
      if (parts.length) return parts.join(' ');
    }
    if (el.id) {
      var forLabel = document.querySelector('label[for="' + el.id.replace(/"/g, '\\"') + '"]');
      if (forLabel) return (forLabel.textContent || '').trim();
    }
    var wrapped = el.closest ? el.closest('label') : null;
    if (wrapped) return (wrapped.textContent || '').trim();
    return '';
  }

  function nameOf(el) {
    return (el.getAttribute('aria-label') || labelledBy(el) || '').trim();
  }

  function shows(el) {
    return !!el && el.getClientRects().length > 0;
  }

  var nodes = document.querySelectorAll('[role=combobox], [aria-haspopup], [aria-expanded], input, select');
  for (var i = 0; i < nodes.length; i++) {
    var el = nodes[i];
    if (nameOf(el).toLowerCase() !== want) continue;
    if (!shows(el)) continue;

    var expanded = el.getAttribute('aria-expanded');
    if (expanded === 'true') return true;
    if (expanded === 'false') return false;

    // A native select with its size raised renders its options inline.
    if (el.tagName === 'SELECT' && el.size > 1) return true;

    // A listbox the control names, or one beside it in the same wrapper.
    var controlled = el.getAttribute('aria-controls');
    if (controlled) {
      var list = document.getElementById(controlled);
      if (shows(list)) return true;
    }
    var scope = el.closest ? el.closest('div, section, form') : null;
    if (scope) {
      var inner = scope.querySelector('[role=listbox], [role=menu], ul.options, .choices__list--dropdown');
      if (shows(inner)) return true;
    }
    return false;
  }
  return null;
})()"#;

/// Signature of the dropdown lists currently showing, for change detection.
///
/// Opening a list is what clicking a combo box is for, and the list that appears
/// is the effect that can be observed regardless of how the control is built.
/// Matching by the control's name is not enough on a real page: the name often
/// comes from a sibling label rather than an attribute, so no element matches it.
/// Comparing what is visible before and after the click needs no name at all.
///
/// Returns `None` when the page cannot be queried, so a caller can tell "no list"
/// apart from "could not ask".
pub async fn visible_option_lists() -> Option<String> {
    let value = evaluate(current_flavor(), VISIBLE_OPTION_LISTS_JS).await?;
    Some(value.as_str().unwrap_or_default().to_string())
}

/// Concatenate the opening text of every visible dropdown list on the page.
const VISIBLE_OPTION_LISTS_JS: &str = r#"(function(){
  var nodes = document.querySelectorAll(
    '[role=listbox], [role=menu], [role=option], ul.options, .choices__list--dropdown, .flatpickr-calendar.open, [role=dialog]');
  var parts = [];
  for (var i = 0; i < nodes.length; i++) {
    var el = nodes[i];
    if (el.getClientRects().length === 0) continue;
    var style = window.getComputedStyle(el);
    if (!style || style.visibility === 'hidden' || style.display === 'none') continue;
    if (parseFloat(style.opacity || '1') <= 0.01) continue;
    parts.push((el.textContent || '').trim().slice(0, 120));
  }
  return parts.join(' || ');
})()"#;

#[cfg(test)]
mod tests {
    use super::state_blocks_click;

    #[test]
    fn readonly_does_not_block_a_click() {
        // A readonly field is what a date picker is built from, so refusing the
        // click would make the picker impossible to open.
        assert!(!state_blocks_click("readonly"));
        assert!(!state_blocks_click("enabled"));
    }

    #[test]
    fn disabled_and_aria_disabled_block_a_click() {
        assert!(state_blocks_click("disabled"));
        assert!(state_blocks_click("aria-disabled"));
    }
}

/// Read what a control currently holds, straight from the page.
///
/// AT-SPI does not always expose a control's value: a custom dropdown keeps its
/// chosen label as plain text inside a `div`, and a native select can report a
/// stale placeholder through the text interface while its value has already
/// changed. Both cases made a successful selection look unverified, so the page
/// is asked directly. The control is found by its accessible name, which is the
/// name the caller already resolved.
///
/// Returns the value, the selected option's label, and the element's text, so a
/// caller can match against whichever one carries the answer.
pub async fn control_value(node: &crate::model::LiveNode) -> Option<String> {
    let name = node
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())?;
    let expression = VALUE_BY_NAME_JS.replace("__NAME__", &js_string(name));
    let value = evaluate(current_flavor(), &expression).await?;
    let value = value.as_str()?.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

/// Report everything a control exposes about its current choice.
///
/// A native select answers with `value` and the selected option's label, while a
/// custom dropdown answers with its visible text, so all three are returned and
/// joined. Only visible elements are considered: a page often keeps a second,
/// hidden copy of a control, and reading that one would describe a widget the
/// user cannot see.
///
/// The name is resolved the way the browser builds it, because a control's
/// accessible name does not always come from `aria-label`: a native select is
/// usually named by its `<label for>` element, which is why a lookup that only
/// checked `aria-label` found nothing and reported a real selection as
/// unverified.
const VALUE_BY_NAME_JS: &str = r#"(function(){
  var want = __NAME__.toLowerCase();

  function labelledBy(el) {
    var ids = (el.getAttribute('aria-labelledby') || '').trim();
    if (ids) {
      var parts = [];
      ids.split(/\s+/).forEach(function (id) {
        var target = document.getElementById(id);
        if (target) parts.push((target.textContent || '').trim());
      });
      if (parts.length) return parts.join(' ');
    }
    if (el.id) {
      var forLabel = document.querySelector('label[for="' + el.id.replace(/"/g, '\\"') + '"]');
      if (forLabel) return (forLabel.textContent || '').trim();
    }
    var wrapped = el.closest ? el.closest('label') : null;
    if (wrapped) return (wrapped.textContent || '').trim();
    return '';
  }

  function nameOf(el) {
    return (el.getAttribute('aria-label') || labelledBy(el) || '').trim();
  }

  var nodes = document.querySelectorAll('input, select, textarea, [aria-label], [role], div, span, li, button');
  for (var i = 0; i < nodes.length; i++) {
    var el = nodes[i];
    if (el.getClientRects().length === 0) continue;
    if (nameOf(el).toLowerCase() !== want) continue;

    var parts = [];
    if (el.value !== undefined && el.value !== null && String(el.value).trim() !== '') {
      parts.push(String(el.value).trim());
    }
    if (el.selectedIndex >= 0 && el.options && el.options[el.selectedIndex]) {
      parts.push((el.options[el.selectedIndex].textContent || '').trim());
    }
    var text = (el.textContent || '').trim();
    if (text) parts.push(text);
    if (parts.length) return parts.join(' | ');
  }
  return null;
})()"#;

/// Whether a reported state should stop a click.
///
/// `readonly` is deliberately allowed: a readonly field is exactly what a date
/// picker uses, and clicking it is how the picker opens. Only genuinely
/// non-interactive states block.
pub fn state_blocks_click(state: &str) -> bool {
    !matches!(state, "enabled" | "readonly")
}

/// What the page itself says occupies a screen point.
#[derive(Debug, Clone, Default)]
pub struct PointHit {
    /// The point is inside a link that would navigate.
    pub link: bool,
    /// The point is on inert page furniture that a header pinned to the top of
    /// the viewport has drawn over the target.
    pub overlay: bool,
    /// The point is inside the viewport rectangle.
    pub inside: bool,
}

/// Ask the page what is actually under a screen point.
///
/// The accessibility tree can say a target owns a point while the page's own
/// hit test resolves it to a sticky header on top: scroll-into-view parks a
/// target at the viewport edge, which is exactly where a fixed header lives,
/// and the click then opens a navigation menu instead of reaching the field.
/// This is the page's answer to "what would I actually hit", so a click is only
/// allowed when the page agrees the target is reachable.
///
/// The window's own coordinates are not enough to convert a screen point to a
/// viewport point: the page area starts below the browser chrome. The offset
/// between the outer and inner height is that chrome, so it is subtracted from
/// the y coordinate. Both values are CSS pixels, so no scale factor is needed.
pub async fn point_hit(flavor: Flavor, screen_x: i32, screen_y: i32) -> Option<PointHit> {
    let expression = POINT_HIT_JS
        .replace("__X__", &screen_x.to_string())
        .replace("__Y__", &screen_y.to_string());

    let value = evaluate(flavor, &expression).await?;
    if value.is_null() {
        return None;
    }

    Some(PointHit {
        link: value
            .get("link")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false),
        overlay: value
            .get("overlay")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false),
        inside: value
            .get("inside")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false),
    })
}

/// Reports what occupies a screen point, and whether a click there would reach
/// a form control instead of a navigation link or a fixed overlay.
///
/// Walks up a few ancestors because a point usually lands on an inner label or
/// icon.
///
/// The overlay test is deliberately narrow. It only fires when the point lands
/// on something that is not itself interactive *and* an ancestor is pinned to
/// the top of the viewport. A form inside a fixed modal container is full of
/// interactive controls, so those keep working; a form parked underneath a
/// sticky header has inert header markup on top of it, so that is refused.
const POINT_HIT_JS: &str = r#"(function(){
  var chrome = Math.max(0, window.outerHeight - window.innerHeight);
  var x = __X__ - window.screenX;
  var y = __Y__ - window.screenY - chrome;
  var inside = x >= 0 && y >= 0 && x < window.innerWidth && y < window.innerHeight;
  var el = document.elementFromPoint(x, y);
  if (!el) return {link: false, overlay: false, inside: inside};

  var link = false;
  var pinned = false;
  var n = el;
  for (var i = 0; i < 6 && n; i++) {
    if (n.nodeType !== 1) { n = n.parentElement; continue; }
    if (n.tagName === 'A' && n.getAttribute('href')) link = true;
    var view = n.ownerDocument.defaultView;
    var style = view ? view.getComputedStyle(n) : null;
    var pos = style ? style.position : '';
    if ((pos === 'fixed' || pos === 'sticky') && n.getBoundingClientRect().top <= 4) {
      pinned = true;
    }
    n = n.parentElement;
  }

  // A control the user can actually operate stays clickable even inside a fixed
  // container: a modal's own form is the target in that case, not an occluder.
  // A link only counts when it would navigate -- a dropdown toggle is an `<a>`
  // with no `href`, and treating it as operable is how a menu sitting over a
  // form field was accepted as the thing to click.
  var topIsInteractive = el.tagName === 'INPUT' || el.tagName === 'SELECT'
    || el.tagName === 'TEXTAREA' || el.tagName === 'BUTTON'
    || (el.tagName === 'A' && el.getAttribute('href'))
    || el.isContentEditable === true
    || (el.getAttribute && el.getAttribute('role') && el.getAttribute('role') !== 'presentation');
  var overlay = pinned && !topIsInteractive;
  return {link: link, overlay: overlay, inside: inside};
})()"#;

/// Move a control to the middle of the viewport, away from sticky chrome.
///
/// Scroll-into-view aligns a target with the nearest viewport edge, which is
/// exactly where a sticky header lives, so the target ends up hidden under one.
/// Centring it instead puts it in the clear, and the browser's own scrolling
/// handles whatever chain of scrollable ancestors the form sits in.
///
/// The control is preferred by accessible name, because the point is precisely
/// what the occluder has taken over: asking the page what is under it returns the
/// header, whose ancestors do not scroll the form at all. When the control has no
/// name -- a combobox with an empty label is common -- the correction is computed
/// from the target's own screen position, which the accessibility tree reports
/// even while it is covered.
///
/// Every scroll is applied instantly. A page with `scroll-behavior: smooth`
/// animates a scroll over hundreds of milliseconds, so a position read straight
/// afterwards still describes the old place and the click is sent to coordinates
/// the target has already left. That is the stale-coordinate failure, and forcing
/// instant scrolling is what removes it.
///
/// Returns `false` when nothing moved, so callers know the nudge did not happen.
pub async fn bring_into_view(flavor: Flavor, name: &str, screen_x: i32, screen_y: i32) -> bool {
    let expression = CENTER_NAMED_JS
        .replace("__NAME__", &js_string(name))
        .replace("__X__", &screen_x.to_string())
        .replace("__Y__", &screen_y.to_string());

    evaluate(flavor, &expression)
        .await
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Centre the control with the given accessible name, or the point when unnamed.
///
/// The name is resolved the way the browser builds it -- `aria-label`, then
/// `aria-labelledby`, then a `<label for>` -- because a form control is usually
/// named by its label element rather than by an attribute.
const CENTER_NAMED_JS: &str = r#"(function(){
  var want = __NAME__.trim().toLowerCase();

  function labelledBy(el) {
    var ids = (el.getAttribute('aria-labelledby') || '').trim();
    if (ids) {
      var parts = [];
      ids.split(/\s+/).forEach(function (id) {
        var target = document.getElementById(id);
        if (target) parts.push((target.textContent || '').trim());
      });
      if (parts.length) return parts.join(' ');
    }
    if (el.id) {
      var forLabel = document.querySelector('label[for="' + el.id.replace(/"/g, '\\"') + '"]');
      if (forLabel) return (forLabel.textContent || '').trim();
    }
    var wrapped = el.closest ? el.closest('label') : null;
    if (wrapped) return (wrapped.textContent || '').trim();
    return '';
  }

  function nameOf(el) {
    return (el.getAttribute('aria-label') || labelledBy(el) || '').trim();
  }

  var nodes = document.querySelectorAll('input, select, textarea, button, a, [role], [aria-label]');
  var el = null;
  if (want) {
    for (var i = 0; i < nodes.length; i++) {
      if (nameOf(nodes[i]).toLowerCase() !== want) continue;
      // Prefer something the user can see; a hidden duplicate must not be moved.
      if (nodes[i].getClientRects().length === 0) continue;
      el = nodes[i];
      break;
    }
  }

  // The browser's own scroll handles any chain of scrollable ancestors, which
  // manual arithmetic does not: the form may sit in a panel, a tab, or the page,
  // and the sticky header may make the nearest scroll container ambiguous.
  if (el) {
    var beforeTop = el.getBoundingClientRect().top;
    el.scrollIntoView({block: 'center', inline: 'nearest', behavior: 'instant'});
    return Math.abs(el.getBoundingClientRect().top - beforeTop) > 1;
  }

  // No name to match on, so correct from the target's own screen position. The
  // page area starts below the browser chrome, which is the difference between
  // the outer and inner height.
  var chrome = Math.max(0, window.outerHeight - window.innerHeight);
  var y = __Y__ - window.screenY - chrome;
  var delta = y - (window.innerHeight / 2);
  if (Math.abs(delta) < 8) return false;
  var before = window.scrollY;
  // `behavior: instant` is required, not cosmetic: a page with
  // `scroll-behavior: smooth` animates this scroll, so the position read on the
  // next line still describes the old place and the nudge looks like it did
  // nothing.
  window.scrollTo({top: before + delta, left: window.scrollX, behavior: 'instant'});
  return window.scrollY !== before;
})()"#;

/// Run a page expression on whichever transport the current browser uses.
async fn evaluate(flavor: Flavor, expression: &str) -> Option<serde_json::Value> {
    let value = match flavor {
        Flavor::Chrome => crate::chrome::devtools::evaluate(expression).await,
        Flavor::Edge => crate::edge::devtools::evaluate(expression).await,
        Flavor::Firefox => crate::firefox::bidi::evaluate(expression).await,
    };
    value.ok()
}

/// Describe the modal that is covering the page, if one is.
///
/// The accessibility tree alone cannot decide this: a site-builder popup is
/// sometimes exposed as a plain `Document Frame` or `Panel`, and a page section
/// is exposed with the very same role, so role matching alone either misses the
/// popup or refuses clicks on ordinary content. The page is asked instead, and
/// only the two structural signatures of a modal count: something marked as a
/// dialog, or a full-viewport fixed overlay. A sticky header is neither, so it
/// is not mistaken for one.
pub async fn covering_overlay() -> Option<String> {
    let value = evaluate(current_flavor(), COVERING_OVERLAY_JS).await?;
    if value.is_null() {
        return None;
    }

    let role = value
        .get("role")
        .and_then(|role| role.as_str())
        .unwrap_or("overlay");
    let name = value
        .get("name")
        .and_then(|name| name.as_str())
        .unwrap_or("");
    let name = name.trim();
    if name.is_empty() {
        Some(role.to_string())
    } else {
        Some(format!("{role} \"{name}\""))
    }
}

/// Find the element that is covering the page, if any.
///
/// Only two signatures are reported, because both are things ordinary page
/// content never has: an element the page declares as a modal dialog, and a
/// fixed layer that covers almost the whole viewport with a stacking order that
/// puts it above the page. A sticky header is neither -- it covers a band, not
/// the viewport -- so it is not mistaken for a modal. Visibility is checked too,
/// because a dialog left hidden in the markup covers nothing.
const COVERING_OVERLAY_JS: &str = r#"(function(){
  var viewport = window.innerWidth * window.innerHeight;

  function visible(el) {
    if (!el || el.getClientRects().length === 0) return false;
    var style = window.getComputedStyle(el);
    if (!style || style.visibility === 'hidden' || style.display === 'none') return false;
    return parseFloat(style.opacity || '1') > 0.01;
  }

  function describe(el) {
    var role = el.getAttribute('role') || el.tagName.toLowerCase();
    var name = el.getAttribute('aria-label') || '';
    if (!name) {
      var heading = el.querySelector('h1, h2, h3, [role=heading]');
      if (heading) name = (heading.textContent || '').trim().slice(0, 80);
    }
    return {role: role, name: name};
  }

  // A dialog the page itself declares. This is the strongest signal, and it is
  // what an ordinary content section never has.
  var declared = document.querySelectorAll(
    '[aria-modal="true"], [role="dialog"], [role="alertdialog"], dialog[open]');
  for (var i = 0; i < declared.length; i++) {
    if (visible(declared[i])) return describe(declared[i]);
  }

  // A full-viewport fixed layer stacked above the page. The height threshold
  // excludes a sticky header, and the stacking requirement excludes the page's
  // own fixed wrappers, which sit at the default stacking level.
  var all = document.querySelectorAll('body *');
  for (var j = 0; j < all.length; j++) {
    var el = all[j];
    if (el === document.body || el === document.documentElement) continue;
    if (!visible(el)) continue;
    var style = window.getComputedStyle(el);
    if (style.position !== 'fixed' && style.position !== 'absolute') continue;
    var z = parseFloat(style.zIndex);
    if (!isFinite(z) || z < 10) continue;
    var rect = el.getBoundingClientRect();
    if (rect.width * rect.height < viewport * 0.7) continue;
    if (rect.height < window.innerHeight * 0.7) continue;
    return describe(el);
  }

  return null;
})()"#;

/// Reports whether the element under a point is disabled.
///
/// Flatpickr and similar widgets disable an option with a CSS class rather than
/// the `disabled` attribute, so the class is checked too, up a few ancestors
/// because the point usually lands on an inner label.
const ELEMENT_AT_POINT_JS: &str = r#"(function(){
  var el = document.elementFromPoint(__X__ - window.screenX, __Y__ - window.screenY);
  if (!el) return null;

  // A readonly control is not a disabled one. Date pickers are readonly by
  // design: the field cannot be typed into, but clicking it opens the picker,
  // so readonly must stay clickable.
  if (el.readOnly === true) return 'readonly';

  // Only the control itself decides. Walking ancestors would let any wrapper
  // that happens to carry a "disabled" class veto a click on a control inside
  // it, which is how a readonly date field was refused.
  if (el.disabled === true) return 'disabled';
  if (el.getAttribute && el.getAttribute('aria-disabled') === 'true') return 'aria-disabled';

  var cls = (typeof el.className === 'string') ? el.className : '';
  var tokens = cls.split(/\s+/);
  for (var i = 0; i < tokens.length; i++) {
    var token = tokens[i].toLowerCase();
    if (token === 'disabled' || token === 'is-disabled' || token === 'flatpickr-disabled') {
      return 'disabled';
    }
  }

  return 'enabled';
})()"#;

/// Reports whether every element with the given accessible name is disabled.
///
/// Returns `'enabled'` when at least one match is usable, so a name shared by a
/// disabled and an enabled control is not treated as blocked. Flatpickr and
/// similar widgets mark a disabled option with a CSS class rather than the
/// `disabled` attribute, so the class is checked too.
const DISABLED_BY_NAME_JS: &str = r#"(function(){
  var want = __NAME__.toLowerCase();
  var matches = 0;
  var disabled = 0;
  var nodes = document.querySelectorAll('[aria-label], [role], button, a, input, select, option, li, span, div');
  for (var i = 0; i < nodes.length; i++) {
    var el = nodes[i];
    // Pages often hold a hidden duplicate, such as a second date picker's day
    // cells. Only the visible one is what the user would be clicking.
    if (el.getClientRects().length === 0) continue;
    var label = (el.getAttribute('aria-label') || '').trim().toLowerCase();
    var text = (el.textContent || '').trim().toLowerCase();
    if (label !== want && text !== want) continue;
    matches++;
    var cls = (typeof el.className === 'string') ? el.className : '';
    // A readonly control is not a disabled one: date pickers are readonly by
    // design and must stay clickable.
    var isDisabled = el.readOnly !== true && (el.disabled === true
      || el.getAttribute('aria-disabled') === 'true'
      || cls.indexOf('disabled') >= 0
      || cls.indexOf('is-disabled') >= 0);
    if (isDisabled) disabled++;
  }
  if (matches === 0) return 'enabled';
  return disabled === matches ? 'disabled' : 'enabled';
})()"#;
