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

    if state != "enabled" {
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

/// Reports whether the element under a point is disabled.
///
/// Flatpickr and similar widgets disable an option with a CSS class rather than
/// the `disabled` attribute, so the class is checked too, up a few ancestors
/// because the point usually lands on an inner label.
const ELEMENT_AT_POINT_JS: &str = r#"(function(){
  var el = document.elementFromPoint(__X__ - window.screenX, __Y__ - window.screenY);
  if (!el) return null;
  var n = el;
  for (var i = 0; i < 4 && n; i++) {
    if (n.disabled === true) return 'disabled';
    if (n.getAttribute && n.getAttribute('aria-disabled') === 'true') return 'aria-disabled';
    var cls = (typeof n.className === 'string') ? n.className : '';
    if (cls.indexOf('disabled') >= 0 || cls.indexOf('is-disabled') >= 0) return 'disabled';
    n = n.parentElement;
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
    var isDisabled = el.disabled === true
      || el.getAttribute('aria-disabled') === 'true'
      || cls.indexOf('disabled') >= 0
      || cls.indexOf('is-disabled') >= 0;
    if (isDisabled) disabled++;
  }
  if (matches === 0) return 'enabled';
  return disabled === matches ? 'disabled' : 'enabled';
})()"#;
