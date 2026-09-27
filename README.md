<p align="center">
  <img src="https://github.com/user-attachments/assets/a50562a8-be13-4aa9-93e7-544a2d8de61c" alt="AxonBrowser" width="900" />
</p>

# AxonBrowser

**Browser automation for real Linux browser windows.**

## Install

Download a prebuilt binary — no Rust toolchain required:

```bash
curl -fsSL https://raw.githubusercontent.com/Axon56/AxonBrowser/main/install.sh | bash
```

Then install the runtime packages it needs:

```bash
axonbrowser install-deps
```

Prebuilt bundles for Linux x86_64 and arm64 are published on the [latest release](https://github.com/Axon56/AxonBrowser/releases/latest). Each bundle contains the `axonbrowser` binary plus the install and runtime dependency scripts, so you can also unpack it directly:

```bash
tar -xzf axonbrowser-linux-x86_64.tar.gz
sudo install -m 755 bin/axonbrowser /usr/local/bin/axonbrowser
axonbrowser install-deps
```

Either way, verify the CLI works with:

```bash
axonbrowser --help
```

AxonBrowser is a Rust CLI for automating Chrome, Chromium, Microsoft Edge, Firefox, and Camoufox on Linux. It combines browser-native control with real rendered-page interaction, making it useful for AI agents, browser workflows, smoke tests, page interaction scripts, and desktop-like browser automation.

Unlike tools that only automate the DOM or only send raw desktop events, AxonBrowser works in two layers:

1. **Browser shell control**

   * Chrome and Edge use native Chromium DevTools.
   * Firefox and Camoufox use Firefox WebDriver BiDi.

2. **Rendered page interaction**

   * Page actions use real accessibility/X11 interaction through AT-SPI.
   * Clicks, typing, focus, waits, reads, uploads, and screenshots happen against the live browser window.

That means commands like `launch`, `current`, `goto`, `tabs`, `scroll`, and `scroll-to` are browser-native, while page commands still interact with the actual rendered UI.

---

## Why AxonBrowser?

AxonBrowser is built for cases where an automation system needs to operate a browser like a real desktop application.

| Tool type                | Good for                          | AxonBrowser adds                                                       |
| ------------------------ | --------------------------------- | ---------------------------------------------------------------------- |
| Playwright / Selenium    | DOM-first web automation          | Real browser-window and accessibility-tree interaction                 |
| xdotool-style scripts    | Raw keyboard and mouse automation | Browser-aware commands, selectors, tabs, screenshots, and page actions |
| Accessibility automation | UI-tree based interaction         | A browser-focused CLI with Chrome, Edge, Firefox, and Camoufox support |
| AI agents                | Browser tasks and web workflows   | Shell control plus rendered-page actions from one CLI                  |

---

## Supported browsers

| Browser           | Shell control     | Page interaction |
| ----------------- | ----------------- | ---------------- |
| Chrome / Chromium | Chromium DevTools | AT-SPI / X11     |
| Microsoft Edge    | Chromium DevTools | AT-SPI / X11     |
| Firefox           | WebDriver BiDi    | AT-SPI / X11     |
| Camoufox          | WebDriver BiDi    | AT-SPI / X11     |

---

## Runtime requirements

AxonBrowser currently targets Linux environments with X11 support.

Required runtime pieces:

* Linux with X11 available, or a headless VPS where AxonBrowser can start its own X11 session
* Chrome/Chromium, Microsoft Edge, Firefox, or Camoufox
* AT-SPI support: `at-spi2-core` and `dbus-x11`
* X11 helpers: `x11-utils`, `xdotool`, `xclip`
* ImageMagick, for screenshot capture through `import`
* `Xvfb`, for headless VPS usage

Headless sessions are created automatically under:

```text
~/.cache/axonbrowser/headless
```

No VNC server, `openbox`, `wmctrl`, or `xwd` is required.

---

## Installation details

Install from a local checkout:

```bash
scripts/install-axonbrowser.sh
```

Install runtime dependencies:

```bash
scripts/install-runtime-deps.sh
```

Optional browser installers used by this repository:

```bash
scripts/install-chrome-local.sh
scripts/install-firefox-local.sh
scripts/install-edge-local.sh
scripts/install-camoufox.sh
```

---

## Build from source

```bash
cargo build --release
```

The release binary is created at:

```text
target/release/axonbrowser
```

For local development:

```bash
cargo build
./target/debug/axonbrowser --help
```

---

## Quick start

Launch a browser:

```bash
axonbrowser chrome launch
axonbrowser edge launch
axonbrowser firefox launch
axonbrowser camoufox launch
```

Open a page:

```bash
axonbrowser chrome goto example.com
```

Read the active browser state:

```bash
axonbrowser chrome current
axonbrowser chrome current --json
```

List tabs:

```bash
axonbrowser firefox tabs list
```

Click and type into the rendered page:

```bash
axonbrowser chrome page click "Button:Submit"
axonbrowser firefox page type "Text Box:Email" "user@example.com"
```

Take a screenshot:

```bash
axonbrowser chrome screenshot artifacts/chrome.png
axonbrowser chrome page screenshot artifacts/page.png
```

---

## Headless VPS usage

```bash
scripts/install-runtime-deps.sh
cargo build --release
./target/release/axonbrowser chrome launch google.com
./target/release/axonbrowser chrome current
./target/release/axonbrowser chrome screenshot artifacts/google.png
```

AxonBrowser automatically bootstraps its own headless X11 session when needed.

---

## Core command model

All browser entry points use the same general structure:

```bash
axonbrowser <browser> <command> [options]
```

Examples:

```bash
axonbrowser chrome launch https://example.com
axonbrowser edge current --json
axonbrowser firefox tabs list
axonbrowser camoufox screenshot artifacts/camoufox.png
```

Supported browser names:

```text
chrome
edge
firefox
camoufox
```

---

## Browser commands

The core browser commands are shared across Chrome, Edge, Firefox, and Camoufox.

Verified native shell coverage in the current tree:

| Browser  | Verified commands                                                                                                        |
| -------- | ------------------------------------------------------------------------------------------------------------------------ |
| Chrome   | `launch`, `current`, `goto`, `new-tab`, `scroll`, `scroll-to`, `tabs list`, `tabs close`, `screenshot`                   |
| Edge     | `launch`, `current`, `goto`, `new-tab`, `scroll`, `scroll-to`, `tabs list`, `tabs close`, `screenshot`                   |
| Firefox  | `launch`, `current`, `goto`, `new-tab`, `scroll`, `scroll-to`, `tabs list`, `tabs close`, `screenshot`                   |
| Camoufox | `launch`, `current`, `goto`, `goto --new-tab`, `new-tab`, `scroll`, `scroll-to`, `tabs list`, `tabs close`, `screenshot` |

### Launch and attach

```bash
axonbrowser chrome launch
axonbrowser chrome launch https://example.com
axonbrowser chrome launch --profile /tmp/my-profile
axonbrowser chrome attach
axonbrowser chrome attach --json
```

### Navigate

```bash
axonbrowser chrome goto example.com
axonbrowser chrome goto https://example.com --new-tab
axonbrowser chrome back
axonbrowser chrome forward
axonbrowser chrome reload
axonbrowser chrome new-tab
```

### Current page and window state

```bash
axonbrowser chrome current
axonbrowser chrome current --json
axonbrowser chrome resize --preset desktop    # 1440x900
axonbrowser chrome resize --preset tablet     # 1024x768
axonbrowser chrome resize --preset mobile     # 430x932
axonbrowser chrome resize --width 1280 --height 900
```

### Scroll

```bash
axonbrowser chrome scroll down --amount 3
axonbrowser chrome scroll-to top
axonbrowser chrome scroll-to bottom
```

### Tabs

```bash
axonbrowser chrome tabs list
axonbrowser chrome tabs switch 1
axonbrowser chrome tabs switch --title example
axonbrowser chrome tabs close
axonbrowser chrome tabs close --index 0
```

### Browser shell interactions

```bash
axonbrowser chrome locate address-bar
axonbrowser chrome focus address-bar
axonbrowser chrome click reload
axonbrowser chrome type address-bar https://example.com
axonbrowser chrome key address-bar ctrl+l
axonbrowser chrome press-enter
axonbrowser chrome hold ctrl --duration-ms 500
axonbrowser chrome screenshot artifacts/chrome.png
axonbrowser chrome screenshot --active artifacts/active.png
```

### Browser popup options

Chrome and Edge expose the same prompt names directly. Firefox uses the same command names, but maps them onto Firefox-native labels where those prompts differ.

```bash
axonbrowser chrome option leave-site cancel
axonbrowser chrome option leave-site leave
axonbrowser chrome option save-password never
axonbrowser chrome option save-password save

axonbrowser edge option leave-site cancel
axonbrowser edge option save-password never

axonbrowser firefox option leave-site cancel
axonbrowser firefox option save-password never

axonbrowser camoufox option leave-site cancel
axonbrowser camoufox option save-password never
```

---

## Page commands

Page commands target rendered web content inside the browser window.

They are useful when you want to interact with the visible page as a user or desktop agent would, instead of only calling browser or DOM APIs.

### Selector syntax

```text
Role:Name          exact role + name    "Heading:Example Domain"
Role               role only            "Push Button"
~name              name contains        "~continue"
Role~name          role + contains      "Button~submit"
```

Common role aliases:

| Alias          | Maps to        |
| -------------- | -------------- |
| `Button`       | `Push Button`  |
| `Text Box`     | `Entry`        |
| `Input`        | `Entry`        |
| `Frame`        | frame scope    |
| `Iframe`       | frame scope    |
| `Check Box`    | checkbox role  |
| `Radio Button` | radio role     |
| `Combo Box`    | combo box role |

Use `--nth <n>` to select the nth match. Indexing is zero-based and defaults to `0`.

Use `--frame` to scope an action to a frame.

---

### Inspect

```bash
axonbrowser chrome page inspect
axonbrowser chrome page frames
axonbrowser chrome page inspect --frame "Frame:Checkout"
```

### Find, count, and read

```bash
axonbrowser chrome page find "Heading:Example Domain"
axonbrowser chrome page count "Push Button"
axonbrowser chrome page read "Heading:Example Domain"
axonbrowser chrome page read --value "Text Box:Email"
```

### Click, focus, and hover

```bash
axonbrowser chrome page focus "Text Box:Email"
axonbrowser chrome page click "Button:Submit"
axonbrowser chrome page hover "Button:Submit"
axonbrowser chrome page double-click "Button:Submit"
axonbrowser chrome page right-click "Button:Submit"
axonbrowser chrome page click --nth 2 "Button:Edit"
axonbrowser chrome page click --frame "Frame:Checkout" "Button:Pay"
```

### Type and keys

```bash
axonbrowser chrome page type "Text Box:Email" "user@example.com"
axonbrowser chrome page key "Text Box:Email" Tab
axonbrowser chrome page press-enter "Button:Submit"
```

### Form helpers

```bash
axonbrowser chrome page check "Check Box:Accept Terms"
axonbrowser chrome page uncheck "Check Box:Accept Terms"
axonbrowser chrome page check "Radio Button:Pro Plan"
axonbrowser chrome page select-option "Combo Box:Country" "Canada"
axonbrowser chrome page upload "Button~Upload File" ./resume.pdf
```

### Page scrolling

```bash
axonbrowser chrome page scroll --direction down --amount 3
axonbrowser chrome page scroll --direction up --amount 1
axonbrowser chrome page scroll "Button:Dismiss Notice" --into-view
```

### Waits

```bash
axonbrowser chrome page wait --text "Submitted"
axonbrowser chrome page wait --title-contains "Dashboard"
axonbrowser chrome page wait --url-contains "dashboard.html"
axonbrowser chrome page wait "Button:Submit"
axonbrowser chrome page wait --state focused "Text Box:Email"
axonbrowser chrome page wait --text "Loading" --disappear
```

### Action and wait flows

```bash
axonbrowser chrome page click-and-wait "Link:Continue" --title-contains "Step 2"
axonbrowser chrome page submit-and-wait "Button:Submit" --text "Submitted"
axonbrowser chrome page submit-and-wait "Button:Submit" --url-contains "done.html"
```

### Page screenshots

```bash
axonbrowser chrome page screenshot artifacts/page.png
axonbrowser chrome page screenshot artifacts/button.png "Button:Submit"
axonbrowser chrome page screenshot artifacts/frame.png --frame "Frame:Checkout" "Button:Pay"
```

---

## Example workflows

### Launch, navigate, inspect, and screenshot

```bash
axonbrowser chrome launch https://example.com
axonbrowser chrome current --json
axonbrowser chrome page read "Heading:Example Domain"
axonbrowser chrome page screenshot artifacts/example.png
```

### Fill a simple form

```bash
axonbrowser chrome launch https://example.com/login
axonbrowser chrome page type "Text Box:Email" "user@example.com"
axonbrowser chrome page type "Text Box:Password" "correct-horse-battery-staple"
axonbrowser chrome page submit-and-wait "Button:Sign in" --url-contains "dashboard"
```

### Work with multiple tabs

```bash
axonbrowser chrome launch https://example.com
axonbrowser chrome goto https://example.org --new-tab
axonbrowser chrome tabs list
axonbrowser chrome tabs switch --title example
axonbrowser chrome tabs close --index 0
```

---

## Development

Run unit tests:

```bash
cargo test
```

Run smoke tests against a live VNC desktop:

```bash
scripts/vnc-chrome-smoke.sh
scripts/vnc-page-smoke.sh
```

Run locally with automatic headless bootstrap where needed:

```bash
cargo run -- chrome launch
```

The demo site used for page smoke tests lives under:

```text
artifacts/page-test-site/
```

---

## Repository layout

```text
src/
  main.rs                        entry point and command dispatch
  bin/axonbrowse.rs              secondary binary shim
  cli.rs                         clap CLI definitions
  browser_options.rs             browser popup matching helpers
  live_access.rs                 AT-SPI read/write helpers
  model.rs                       shared UI/tree data models
  render.rs                      tree and locator rendering
  runtime.rs                     headless session/bootstrap logic
  selector.rs                    selector parsing and matching
  chrome/                        Chrome/Chromium automation
    attach.rs, current.rs, devtools.rs
    goto.rs, launch.rs, locators.rs, options.rs, resize.rs
    retry.rs, screenshot.rs, session.rs, wait.rs, window.rs, windows.rs
    tabs/                        tab listing, switching, closing
    actions/                     browser shell focus/click/type/key/hold/scroll
    page/
      find.rs, flow.rs, root.rs, screenshot.rs, wait.rs
      actions/                   click, focus, form, input, keys, physical, pointer, read, scroll, target, upload
  edge/                          Edge automation, mirrors chrome/
    discovery.rs                 shared window/profile discovery helpers
  firefox/                       Firefox/Camoufox automation, mirrors chrome/
    bidi.rs                      WebDriver BiDi session and tab handling
  inspect.rs                     AT-SPI tree inspection
  window.rs                      X11 window helpers
scripts/
  install-axonbrowser.sh
  install-camoufox.sh
  install-chrome-local.sh
  install-edge-local.sh
  install-firefox-local.sh
  install-runtime-deps.sh
  use-vnc-session.sh
  vnc-chrome-flow.sh
  vnc-chrome-smoke.sh
  vnc-page-smoke.sh
```

---

## Known limitations

* AxonBrowser is currently Linux/X11-focused.
* Wayland is not the primary target.
* Page interaction depends on the browser exposing a useful accessibility tree.
* Browser accessibility behavior can vary by browser, page, and desktop environment.
* Headless operation depends on Xvfb, D-Bus, and AT-SPI being available and working correctly.

---

## Status

AxonBrowser is an early-stage project. The command surface is already broad, but APIs, command names, and behavior may change while the project matures.

Run the full command tree with:

```bash
axonbrowser --help
```
