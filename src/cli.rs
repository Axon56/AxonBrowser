use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "axonbrowser",
    version,
    about = "Browser automation CLI for Chrome, Edge, Firefox, and Camoufox",
    long_about = "axonbrowser — browser automation via the accessibility tree\n\
\n\
Drives real browser windows (Chrome, Edge, Firefox) through the OS\n\
accessibility bus (AT-SPI). Works with the browser shell (address bar,\n\
tabs, resize, navigate) and the rendered page (find, click, type, wait,\n\
screenshot, form helpers, iframes).\n\
\n\
QUICK START\n\
\n\
  # Launch Chrome and navigate to a URL\n\
  axonbrowser chrome launch\n\
  axonbrowser chrome goto example.com\n\
\n\
  # Interact with the page\n\
  axonbrowser chrome page find \"Heading:Example Domain\"\n\
  axonbrowser chrome page click \"Button:Sign in\"\n\
  axonbrowser chrome page type \"Text Box:Email\" \"user@example.com\"\n\
  axonbrowser chrome page screenshot artifacts/shot.png\n\
\n\
  # The core commands also work for Edge, Firefox, and Camoufox\n\
  axonbrowser edge launch\n\
  axonbrowser edge goto example.com\n\
  axonbrowser firefox launch\n\
  axonbrowser firefox goto example.com\n\
  axonbrowser camoufox launch\n\
  axonbrowser camoufox goto example.com\n\
\n\
  # Browser popup helpers for browser-owned prompts\n\
  axonbrowser chrome option leave-site cancel\n\
  axonbrowser edge option save-password never\n\
  axonbrowser firefox option leave-site cancel\n\
  axonbrowser camoufox option leave-site cancel\n\
\n\
Run `axonbrowser <BROWSER> --help` to see all commands for a browser.\n\
Run `axonbrowser <BROWSER> page --help` to see all page automation commands."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Install system/runtime dependencies and selected browsers.
    InstallDeps,
    /// Chrome/Chromium-focused automation commands.
    Chrome(ChromeArgs),
    /// Microsoft Edge-focused automation commands.
    Edge(EdgeArgs),
    /// Firefox-focused automation commands.
    #[command(alias = "camoufox")]
    Firefox(FirefoxArgs),
}

#[derive(Debug, Args)]
pub struct ChromeArgs {
    #[command(subcommand)]
    pub command: ChromeCommands,
}

#[derive(Debug, Args)]
pub struct EdgeArgs {
    #[command(subcommand)]
    pub command: EdgeCommands,
}

#[derive(Debug, Args)]
pub struct FirefoxArgs {
    #[command(subcommand)]
    pub command: FirefoxCommands,
}

#[derive(Debug, Subcommand)]
pub enum FirefoxCommands {
    /// Launch a fresh Firefox window with a clean or provided profile.
    Launch(ChromeLaunchArgs),
    /// Attach to the active visible Firefox window and print session state.
    Attach(ChromeAttachArgs),
    /// List visible Firefox windows that axonbrowser can attach to.
    Windows,
    /// Resolve a named browser-ui locator inside the first visible Firefox window.
    Locate(ChromeLocateArgs),
    /// Capture a screenshot of the first visible Firefox window.
    Screenshot(ChromeScreenshotArgs),
    /// Resize the active Firefox window.
    Resize(ChromeResizeArgs),
    /// Focus a named browser-ui locator.
    Focus(ChromeLocatorArgs),
    /// Click a named browser-ui locator.
    Click(ChromeLocatorArgs),
    /// Type text into a named browser-ui locator.
    Type(ChromeTypeArgs),
    /// Press a key after focusing a named browser-ui locator.
    Key(ChromeKeyArgs),
    /// Press Enter after optionally focusing a named browser-ui locator.
    PressEnter(ChromePressEnterArgs),
    /// Hold a key down for a short duration in the browser window.
    Hold(ChromeHoldArgs),
    /// Scroll the rendered page window.
    Scroll(BrowserScrollArgs),
    /// Jump to the top or bottom of the rendered page.
    ScrollTo(BrowserScrollToArgs),
    /// Wait for a locator, page title change, or URL change.
    Wait(ChromeWaitArgs),
    /// Navigate the current Firefox window by focusing the address bar, entering a URL, and pressing Enter.
    Goto(ChromeGotoArgs),
    /// Click the browser Back button.
    Back,
    /// Click the browser Forward button.
    Forward,
    /// Click the browser Reload button.
    Reload,
    /// Click the browser New Tab button.
    NewTab,
    /// Click a button inside a Firefox browser-owned popup, such as a leave-page or password-save prompt.
    Option(BrowserOptionArgs),
    /// Print the current Firefox window and tab state.
    Current(ChromeCurrentArgs),
    /// List currently visible browser tabs.
    Tabs(ChromeTabsArgs),
    /// Inspect and automate the rendered web page inside Firefox.
    Page(ChromePageArgs),
}

#[derive(Debug, Subcommand)]
pub enum EdgeCommands {
    /// Launch a fresh Microsoft Edge window with a clean or provided profile.
    Launch(ChromeLaunchArgs),
    /// Attach to the active visible Microsoft Edge window and print session state.
    Attach(ChromeAttachArgs),
    /// List visible Microsoft Edge windows that axonbrowser can attach to.
    Windows,
    /// Resolve a named browser-ui locator inside the first visible Microsoft Edge window.
    Locate(ChromeLocateArgs),
    /// Capture a screenshot of the first visible Microsoft Edge window.
    Screenshot(ChromeScreenshotArgs),
    /// Resize the active Microsoft Edge window.
    Resize(ChromeResizeArgs),
    /// Focus a named browser-ui locator.
    Focus(ChromeLocatorArgs),
    /// Click a named browser-ui locator.
    Click(ChromeLocatorArgs),
    /// Type text into a named browser-ui locator.
    Type(ChromeTypeArgs),
    /// Press a key after focusing a named browser-ui locator.
    Key(ChromeKeyArgs),
    /// Press Enter after optionally focusing a named browser-ui locator.
    PressEnter(ChromePressEnterArgs),
    /// Hold a key down for a short duration in the browser window.
    Hold(ChromeHoldArgs),
    /// Scroll the rendered page window.
    Scroll(BrowserScrollArgs),
    /// Jump to the top or bottom of the rendered page.
    ScrollTo(BrowserScrollToArgs),
    /// Wait for a locator, page title change, or URL change.
    Wait(ChromeWaitArgs),
    /// Navigate the current Edge window by focusing the address bar, entering a URL, and pressing Enter.
    Goto(ChromeGotoArgs),
    /// Click the browser Back button.
    Back,
    /// Click the browser Forward button.
    Forward,
    /// Click the browser Reload button.
    Reload,
    /// Click the browser New Tab button.
    NewTab,
    /// Click a button inside an Edge browser-owned popup, such as "Leave site?" or "Save password?".
    Option(BrowserOptionArgs),
    /// Print the current Microsoft Edge window and tab state.
    Current(ChromeCurrentArgs),
    /// List currently visible browser tabs.
    Tabs(ChromeTabsArgs),
    /// Inspect and automate the rendered web page inside Edge.
    Page(ChromePageArgs),
}

#[derive(Debug, Subcommand)]
pub enum ChromeCommands {
    /// Launch a fresh Chrome/Chromium window with a clean or provided profile.
    Launch(ChromeLaunchArgs),
    /// Attach to the active visible Chrome/Chromium window and print session state.
    Attach(ChromeAttachArgs),
    /// List visible Chrome/Chromium windows that axonbrowser can attach to.
    Windows,
    /// Resolve a named browser-ui locator inside the first visible Chrome/Chromium window.
    Locate(ChromeLocateArgs),
    /// Capture a screenshot of the first visible Chrome/Chromium window.
    Screenshot(ChromeScreenshotArgs),
    /// Resize the active Chrome/Chromium window.
    Resize(ChromeResizeArgs),
    /// Focus a named browser-ui locator.
    Focus(ChromeLocatorArgs),
    /// Click a named browser-ui locator.
    Click(ChromeLocatorArgs),
    /// Type text into a named browser-ui locator.
    Type(ChromeTypeArgs),
    /// Press a key after focusing a named browser-ui locator.
    Key(ChromeKeyArgs),
    /// Press Enter after optionally focusing a named browser-ui locator.
    PressEnter(ChromePressEnterArgs),
    /// Hold a key down for a short duration in the browser window.
    Hold(ChromeHoldArgs),
    /// Scroll the rendered page window.
    Scroll(BrowserScrollArgs),
    /// Jump to the top or bottom of the rendered page.
    ScrollTo(BrowserScrollToArgs),
    /// Wait for a locator, page title change, or URL change.
    Wait(ChromeWaitArgs),
    /// Navigate the current Chrome window by focusing the address bar, entering a URL, and pressing Enter.
    Goto(ChromeGotoArgs),
    /// Click the browser Back button.
    Back,
    /// Click the browser Forward button.
    Forward,
    /// Click the browser Reload button.
    Reload,
    /// Click the browser New Tab button.
    NewTab,
    /// Click a button inside a Chrome browser-owned popup, such as "Leave site?" or "Save password?".
    Option(BrowserOptionArgs),
    /// Print the current browser window, tab, and URL state.
    Current(ChromeCurrentArgs),
    /// List currently visible browser tabs.
    Tabs(ChromeTabsArgs),
    /// Inspect and automate the rendered web page inside Chrome.
    Page(ChromePageArgs),
}

#[derive(Debug, Args)]
pub struct ChromeLaunchArgs {
    /// Optional initial URL. Defaults to about:blank.
    pub url: Option<String>,

    /// Optional profile directory. If omitted, axonbrowser creates a fresh temp profile.
    #[arg(long)]
    pub profile: Option<String>,

    /// Timeout in milliseconds while waiting for the launched browser window.
    #[arg(long, default_value_t = crate::chrome::wait::default_timeout_ms())]
    pub timeout_ms: u64,

    /// Poll interval in milliseconds while waiting for the launched browser window.
    #[arg(long, default_value_t = crate::chrome::wait::default_poll_ms())]
    pub poll_ms: u64,
}

#[derive(Debug, Args)]
pub struct ChromeAttachArgs {
    /// Render the attached browser session as JSON.
    #[arg(long)]
    pub json: bool,

    /// Explicit X11 window id to attach to and remember for later commands.
    #[arg(long = "window-id")]
    pub window_id: Option<String>,
}

#[derive(Debug, Args)]
pub struct ChromeCurrentArgs {
    /// Render the current browser state as JSON.
    #[arg(long)]
    pub json: bool,

    /// Optional explicit X11 window id to target for current-state resolution.
    #[arg(long = "window-id")]
    pub window_id: Option<String>,
}

#[derive(Debug, Args)]
pub struct ChromeLocateArgs {
    /// Named browser-ui locator.
    ///
    /// Examples: address-bar, omnibox, back, forward, reload, new-tab,
    /// tab-strip, current-tab, window.
    pub locator: String,
}

#[derive(Debug, Args)]
pub struct ChromeLocatorArgs {
    /// Named browser-ui locator.
    pub locator: String,
}

#[derive(Debug, Args)]
pub struct ChromeTypeArgs {
    /// Named browser-ui locator.
    pub locator: String,

    /// Text to type into the locator.
    pub text: String,
}

#[derive(Debug, Args)]
pub struct ChromeKeyArgs {
    /// Named browser-ui locator.
    pub locator: String,

    /// Key/chord to send, for example Return, ctrl+l, Alt+F4.
    pub key: String,
}

#[derive(Debug, Args)]
pub struct ChromePressEnterArgs {
    /// Optional named browser-ui locator. Defaults to address-bar.
    #[arg(long)]
    pub locator: Option<String>,
}

#[derive(Debug, Args)]
pub struct ChromeHoldArgs {
    /// Key to hold, for example ctrl, shift, alt.
    pub key: String,

    /// Hold duration in milliseconds.
    #[arg(long, default_value_t = 1000)]
    pub duration_ms: u64,

    /// Optional named browser-ui locator to focus before holding.
    #[arg(long)]
    pub locator: Option<String>,
}

#[derive(Debug, Args)]
pub struct ChromeGotoArgs {
    /// URL to navigate to. If no scheme is provided, axonbrowser prefixes https://.
    pub url: String,

    /// Open a fresh tab before navigating.
    #[arg(long)]
    pub new_tab: bool,

    /// Timeout in milliseconds for page-state change after pressing Enter.
    #[arg(long, default_value_t = crate::chrome::wait::default_timeout_ms())]
    pub timeout_ms: u64,

    /// Poll interval in milliseconds while waiting for page-state change.
    #[arg(long, default_value_t = crate::chrome::wait::default_poll_ms())]
    pub poll_ms: u64,
}

#[derive(Debug, Args)]
pub struct BrowserScrollArgs {
    /// Scroll direction.
    #[arg(value_enum)]
    pub direction: ChromePageScrollDirectionArg,

    /// Number of wheel steps.
    #[arg(long, default_value_t = 3)]
    pub amount: u32,
}

#[derive(Debug, Args)]
pub struct BrowserScrollToArgs {
    /// Target page position.
    #[arg(value_enum)]
    pub target: BrowserScrollTargetArg,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum BrowserScrollTargetArg {
    Top,
    Bottom,
}

#[derive(Debug, Args)]
pub struct BrowserOptionArgs {
    /// Browser-owned prompt to target.
    #[arg(value_enum)]
    pub prompt: BrowserOptionPromptArg,

    /// Button/choice to press inside the prompt.
    #[arg(value_enum)]
    pub choice: BrowserOptionChoiceArg,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum BrowserOptionPromptArg {
    LeaveSite,
    SavePassword,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum BrowserOptionChoiceArg {
    Cancel,
    Leave,
    Never,
    Save,
}

#[derive(Debug, Args)]
pub struct ChromeTabsArgs {
    /// Optional explicit X11 window id to target for tab operations.
    #[arg(long = "window-id")]
    pub window_id: Option<String>,

    #[command(subcommand)]
    pub command: ChromeTabsCommand,
}

#[derive(Debug, Args)]
pub struct ChromeResizeArgs {
    /// Named preset. Use instead of explicit width/height.
    #[arg(long, value_enum)]
    pub preset: Option<ChromeResizePresetArg>,

    /// Explicit width in pixels.
    #[arg(long)]
    pub width: Option<u32>,

    /// Explicit height in pixels.
    #[arg(long)]
    pub height: Option<u32>,

    /// Optional window-title query override.
    #[arg(long)]
    pub query: Option<String>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ChromeResizePresetArg {
    Desktop,
    Tablet,
    Mobile,
}

#[derive(Debug, Args)]
pub struct ChromePageArgs {
    #[command(subcommand)]
    pub command: ChromePageCommand,
}

#[derive(Debug, Args, Clone, Default)]
pub struct ChromePageScopeArgs {
    /// Scope page commands to a matched frame/iframe inside the current page tree.
    #[arg(long = "frame")]
    pub frame_selectors: Vec<String>,
}

#[derive(Debug, Args, Clone)]
pub struct ChromePageTransitionArgs {
    /// Alternate selector chain to wait for after the action.
    #[arg(long = "wait-selector")]
    pub wait_selectors: Vec<String>,

    /// Alternate frame selector chain for the post-action wait target.
    #[arg(long = "wait-frame")]
    pub wait_frame_selectors: Vec<String>,

    /// Wait for visible page text containing this substring.
    #[arg(long)]
    pub text: Option<String>,

    /// Wait for the page title to contain this substring.
    #[arg(long = "title-contains")]
    pub title_contains: Option<String>,

    /// Wait for the page URL to contain this substring.
    #[arg(long = "url-contains")]
    pub url_contains: Option<String>,

    /// Wait for the selector/text/title/url fragment to disappear instead of appear.
    #[arg(long)]
    pub disappear: bool,

    /// Timeout in milliseconds.
    #[arg(long, default_value_t = crate::chrome::wait::default_timeout_ms())]
    pub timeout_ms: u64,

    /// Poll interval in milliseconds.
    #[arg(long, default_value_t = crate::chrome::wait::default_poll_ms())]
    pub poll_ms: u64,
}

#[derive(Debug, Subcommand)]
pub enum ChromePageCommand {
    /// Print the accessibility tree for the current page content only.
    Inspect(ChromePageInspectArgs),
    /// List frame/iframe matches inside the current page content tree.
    Frames(ChromePageInspectArgs),
    /// Find selector matches inside the current page content tree.
    Find(ChromePageFindArgs),
    /// Count selector matches inside the current page content tree.
    Count(ChromePageFindArgs),
    /// Read text/value from the first matched page node.
    Read(ChromePageReadArgs),
    /// Focus the first page node matched by the selector chain.
    Focus(ChromePageTargetArgs),
    /// Click the first node matched inside the current page content tree.
    Click(ChromePageTargetArgs),
    /// Hover the first node matched inside the current page content tree.
    Hover(ChromePageTargetArgs),
    /// Double click the first node matched inside the current page content tree.
    DoubleClick(ChromePageTargetArgs),
    /// Right click the first node matched inside the current page content tree.
    RightClick(ChromePageTargetArgs),
    /// Click a page node, then wait for navigation/text/selector state.
    ClickAndWait(ChromePageActionWaitArgs),
    /// Type text into the first page node matched by the selector chain.
    Type(ChromePageTypeArgs),
    /// Press a key after focusing the first page node matched by the selector chain.
    Key(ChromePageKeyArgs),
    /// Press Enter after optionally focusing a matched page node.
    PressEnter(ChromePagePressEnterArgs),
    /// Submit the current/focused page control with Enter, then wait for navigation/text/selector state.
    SubmitAndWait(ChromePageSubmitWaitArgs),
    /// Set a checkbox/toggle/radio/selectable control to checked/selected.
    Check(ChromePageFindArgs),
    /// Clear a checkbox/toggle/selectable control.
    Uncheck(ChromePageFindArgs),
    /// Choose an option from a combo box / select-like control.
    SelectOption(ChromePageSelectOptionArgs),
    /// Type a file path into a file-upload control.
    Upload(ChromePageUploadArgs),
    /// Scroll the page window or scroll a target into view.
    Scroll(ChromePageScrollArgs),
    /// Capture a screenshot from the page context.
    Screenshot(ChromePageScreenshotArgs),
    /// Wait for page text, selectors, title fragments, or URL fragments.
    Wait(ChromePageWaitArgs),
}

#[derive(Debug, Args)]
pub struct ChromePageInspectArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,
}

#[derive(Debug, Args)]
pub struct ChromePageFindArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,

    /// Selector chain to apply inside the page content tree.
    #[arg(required = true)]
    pub selectors: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ChromePageTargetArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,

    /// Selector chain to apply inside the page content tree.
    #[arg(required = true)]
    pub selectors: Vec<String>,

    /// Zero-based match index. Defaults to the first match.
    #[arg(long)]
    pub nth: Option<usize>,
}

#[derive(Debug, Args)]
pub struct ChromePageReadArgs {
    #[command(flatten)]
    pub target: ChromePageTargetArgs,

    /// Read the control value/text content instead of the node label text.
    #[arg(long)]
    pub value: bool,
}

#[derive(Debug, Args)]
pub struct ChromePageActionWaitArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,

    /// Selector chain to click inside the page content tree.
    #[arg(required = true)]
    pub selectors: Vec<String>,

    #[command(flatten)]
    pub wait: ChromePageTransitionArgs,
}

#[derive(Debug, Args)]
pub struct ChromePageTypeArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,

    /// Selector chain to apply inside the page content tree.
    #[arg(required = true)]
    pub selectors: Vec<String>,

    /// Text to type into the matched page node.
    pub text: String,
}

#[derive(Debug, Args)]
pub struct ChromePageKeyArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,

    /// Selector chain to apply inside the page content tree.
    #[arg(required = true)]
    pub selectors: Vec<String>,

    /// Key/chord to send, for example Return, Tab, ctrl+a.
    pub key: String,
}

#[derive(Debug, Args)]
pub struct ChromePagePressEnterArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,

    /// Optional selector chain to focus before pressing Enter.
    #[arg(required = false)]
    pub selectors: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ChromePageSubmitWaitArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,

    /// Optional selector chain to focus before pressing Enter.
    #[arg(required = false)]
    pub selectors: Vec<String>,

    #[command(flatten)]
    pub wait: ChromePageTransitionArgs,
}

#[derive(Debug, Args)]
pub struct ChromePageSelectOptionArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,

    /// Selector chain for the combo-box/select control.
    #[arg(required = true)]
    pub selectors: Vec<String>,

    /// Option label to select.
    pub option: String,

    /// Zero-based index of the control to act on when the selector matches
    /// several. The index never refers to the option: a form with two combo
    /// boxes needs the second control, not the second option with that label.
    #[arg(long)]
    pub nth: Option<usize>,
}

#[derive(Debug, Args)]
pub struct ChromePageUploadArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,

    /// Selector chain for the file upload control.
    #[arg(required = true)]
    pub selectors: Vec<String>,

    /// Local file path to upload.
    pub path: String,
}

#[derive(Debug, Args)]
pub struct ChromePageScrollArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,

    /// Selector chain to scroll into view. Omit to scroll the page window.
    #[arg(required = false)]
    pub selectors: Vec<String>,

    /// Zero-based match index. Defaults to the first match.
    #[arg(long)]
    pub nth: Option<usize>,

    /// Scroll direction for window scrolling.
    #[arg(long, value_enum, default_value_t = ChromePageScrollDirectionArg::Down)]
    pub direction: ChromePageScrollDirectionArg,

    /// Number of wheel steps.
    #[arg(long, default_value_t = 3)]
    pub amount: u32,

    /// Use selector-based scroll-into-view instead of window scrolling.
    #[arg(long)]
    pub into_view: bool,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ChromePageScrollDirectionArg {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Args)]
pub struct ChromePageScreenshotArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,

    /// Output PNG path.
    pub output: String,

    /// Selector chain to target an element screenshot. Omit for the browser window.
    #[arg(required = false)]
    pub selectors: Vec<String>,

    /// Zero-based match index. Defaults to the first match.
    #[arg(long)]
    pub nth: Option<usize>,
}

#[derive(Debug, Args)]
pub struct ChromePageWaitArgs {
    #[command(flatten)]
    pub scope: ChromePageScopeArgs,

    /// Selector chain to apply inside the page content tree.
    #[arg(required = false)]
    pub selectors: Vec<String>,

    /// Zero-based match index for selector-backed waits.
    #[arg(long)]
    pub nth: Option<usize>,

    /// Wait for visible page text containing this substring.
    #[arg(long)]
    pub text: Option<String>,

    /// Wait for the page title to contain this substring.
    #[arg(long = "title-contains")]
    pub title_contains: Option<String>,

    /// Wait for the page URL to contain this substring.
    #[arg(long = "url-contains")]
    pub url_contains: Option<String>,

    /// Wait for the selector or text to disappear instead of appear.
    #[arg(long)]
    pub disappear: bool,

    /// Wait for a specific element state.
    #[arg(long, value_enum)]
    pub state: Option<ChromePageStateArg>,

    /// Timeout in milliseconds.
    #[arg(long, default_value_t = crate::chrome::wait::default_timeout_ms())]
    pub timeout_ms: u64,

    /// Poll interval in milliseconds.
    #[arg(long, default_value_t = crate::chrome::wait::default_poll_ms())]
    pub poll_ms: u64,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ChromePageStateArg {
    Focused,
    Checked,
    Enabled,
    Disabled,
    Expanded,
    Collapsed,
}

#[derive(Debug, Subcommand)]
pub enum ChromeTabsCommand {
    /// List browser tabs and mark the current one.
    List,
    /// Switch to a tab by zero-based index or by title substring.
    Switch(ChromeTabSwitchArgs),
    /// Close a tab by zero-based index; defaults to the current tab.
    Close(ChromeTabCloseArgs),
}

#[derive(Debug, Args)]
pub struct ChromeTabSwitchArgs {
    /// Zero-based tab index. Optional when using --title.
    #[arg(long)]
    pub index: Option<usize>,

    /// Case-insensitive substring to match against visible tab titles.
    #[arg(long, conflicts_with = "index")]
    pub title: Option<String>,
}

#[derive(Debug, Args)]
pub struct ChromeTabCloseArgs {
    /// Zero-based tab index. Defaults to the current tab.
    #[arg(long)]
    pub index: Option<usize>,
}

#[derive(Debug, Args)]
pub struct ChromeWaitArgs {
    #[command(subcommand)]
    pub target: ChromeWaitTarget,
}

#[derive(Debug, Subcommand)]
pub enum ChromeWaitTarget {
    /// Wait until a named browser-ui locator resolves.
    Locator(ChromeWaitLocatorArgs),
    /// Wait until the current page title changes.
    TitleChange(ChromeWaitChangeArgs),
    /// Wait until the current page URL changes.
    UrlChange(ChromeWaitChangeArgs),
}

#[derive(Debug, Args)]
pub struct ChromeWaitLocatorArgs {
    /// Named browser-ui locator.
    pub locator: String,

    /// Timeout in milliseconds.
    #[arg(long, default_value_t = crate::chrome::wait::default_timeout_ms())]
    pub timeout_ms: u64,

    /// Poll interval in milliseconds.
    #[arg(long, default_value_t = crate::chrome::wait::default_poll_ms())]
    pub poll_ms: u64,
}

#[derive(Debug, Args)]
pub struct ChromeWaitChangeArgs {
    /// Optional previous value. If omitted, axonbrowser uses the current value as the baseline.
    #[arg(long)]
    pub from: Option<String>,

    /// Timeout in milliseconds.
    #[arg(long, default_value_t = crate::chrome::wait::default_timeout_ms())]
    pub timeout_ms: u64,

    /// Poll interval in milliseconds.
    #[arg(long, default_value_t = crate::chrome::wait::default_poll_ms())]
    pub poll_ms: u64,
}

#[derive(Debug, Args)]
pub struct ChromeScreenshotArgs {
    /// Output PNG path.
    pub output: String,

    /// Optional window-name query override when auto-detection is not enough.
    #[arg(long)]
    pub query: Option<String>,

    /// Capture the active window instead of auto-targeting Chrome.
    #[arg(long)]
    pub active: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scroll_to_for_all_browsers() {
        let chrome = Cli::try_parse_from(["axonbrowser", "chrome", "scroll-to", "top"]).unwrap();
        let edge = Cli::try_parse_from(["axonbrowser", "edge", "scroll-to", "bottom"]).unwrap();
        let firefox = Cli::try_parse_from(["axonbrowser", "firefox", "scroll-to", "top"]).unwrap();
        let camoufox =
            Cli::try_parse_from(["axonbrowser", "camoufox", "scroll-to", "bottom"]).unwrap();

        assert!(matches!(
            chrome.command,
            Commands::Chrome(ChromeArgs {
                command: ChromeCommands::ScrollTo(BrowserScrollToArgs {
                    target: BrowserScrollTargetArg::Top
                })
            })
        ));
        assert!(matches!(
            edge.command,
            Commands::Edge(EdgeArgs {
                command: EdgeCommands::ScrollTo(BrowserScrollToArgs {
                    target: BrowserScrollTargetArg::Bottom
                })
            })
        ));
        assert!(matches!(
            firefox.command,
            Commands::Firefox(FirefoxArgs {
                command: FirefoxCommands::ScrollTo(BrowserScrollToArgs {
                    target: BrowserScrollTargetArg::Top
                })
            })
        ));
        assert!(matches!(
            camoufox.command,
            Commands::Firefox(FirefoxArgs {
                command: FirefoxCommands::ScrollTo(BrowserScrollToArgs {
                    target: BrowserScrollTargetArg::Bottom
                })
            })
        ));
    }
}
