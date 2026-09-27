---
name: axonbrowser
description: install, configure, and use axonbrowser, a linux cli for automating real browser windows across chrome/chromium, microsoft edge, firefox, and camoufox. use this skill when the user asks chatgpt to control a browser through axonbrowser, run browser workflows for an ai agent, install axonbrowser and its runtime dependencies, launch or attach to browsers, navigate pages, manage tabs, take screenshots, click/type/read rendered page elements through accessibility selectors, or troubleshoot axonbrowser cli/browser automation issues on linux or headless vps environments.
---

# AxonBrowser

Use AxonBrowser to operate real Linux browser windows from the command line. Prefer AxonBrowser when the user asks to automate Chrome/Chromium, Microsoft Edge, Firefox, or Camoufox as a live desktop application, especially for AI-agent browser workflows.

## Operating assumptions

- AxonBrowser targets Linux with X11. For headless hosts, it can bootstrap an X11 session under `~/.cache/axonbrowser/headless`.
- Chrome and Edge shell commands use Chromium DevTools.
- Firefox and Camoufox shell commands use Firefox WebDriver BiDi.
- Rendered page commands use AT-SPI/X11 accessibility actions against the live browser window.
- Do not assume Wayland-first environments work reliably. Prefer X11 or Xvfb.

## Installation workflow

First check whether `axonbrowser` is already available:

```bash
command -v axonbrowser && axonbrowser --help
```

If it is missing and the user wants setup, install with:

```bash
curl -fsSL https://raw.githubusercontent.com/Axon56/AxonBrowser/main/install.sh | bash
```

If working from a local repository checkout, prefer:

```bash
scripts/install-runtime-deps.sh
scripts/install-axonbrowser.sh
```

For source builds:

```bash
cargo build --release
./target/release/axonbrowser --help
```

If runtime packages are missing, run:

```bash
scripts/install-runtime-deps.sh
```

## Command selection workflow

1. Identify the target browser: `chrome`, `edge`, `firefox`, or `camoufox`.
2. For browser-level work, use shell commands such as `launch`, `goto`, `current`, `tabs`, `scroll`, `scroll-to`, `resize`, or `screenshot`.
3. For rendered page interaction, use `page` commands such as `page click`, `page type`, `page read`, `page wait`, or `page screenshot`.
4. After actions that navigate or mutate state, verify with `current`, `tabs list`, `page read`, `page wait`, or a screenshot.
5. When a selector fails, inspect the accessibility tree before retrying:

```bash
axonbrowser chrome page inspect
axonbrowser chrome page frames
```

## Browser shell commands

Launch or attach:

```bash
axonbrowser chrome launch
axonbrowser chrome launch https://example.com
axonbrowser chrome launch --profile /tmp/my-profile
axonbrowser chrome attach
axonbrowser chrome attach --json
```

Navigate:

```bash
axonbrowser chrome goto example.com
axonbrowser chrome goto https://example.com --new-tab
axonbrowser chrome back
axonbrowser chrome forward
axonbrowser chrome reload
axonbrowser chrome new-tab
```

Inspect current state:

```bash
axonbrowser chrome current
axonbrowser chrome current --json
```

Resize:

```bash
axonbrowser chrome resize --preset desktop
axonbrowser chrome resize --preset tablet
axonbrowser chrome resize --preset mobile
axonbrowser chrome resize --width 1280 --height 900
```

Scroll:

```bash
axonbrowser chrome scroll down --amount 3
axonbrowser chrome scroll-to top
axonbrowser chrome scroll-to bottom
```

Tabs:

```bash
axonbrowser chrome tabs list
axonbrowser chrome tabs switch 1
axonbrowser chrome tabs switch --title example
axonbrowser chrome tabs close
axonbrowser chrome tabs close --index 0
```

Browser UI interactions:

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

Browser popup options:

```bash
axonbrowser chrome option leave-site cancel
axonbrowser chrome option leave-site leave
axonbrowser chrome option save-password never
axonbrowser chrome option save-password save
```

## Page selectors

Use accessibility selectors for rendered page commands:

```text
Role:Name          exact role + name    "Heading:Example Domain"
Role               role only            "Push Button"
~name              name contains        "~continue"
Role~name          role + contains      "Button~submit"
```

Common aliases:

- `Button` -> `Push Button`
- `Text Box` or `Input` -> `Entry`
- `Frame` or `Iframe` -> frame scope
- `Check Box`, `Radio Button`, `Combo Box` map to their matching accessibility roles

Use `--nth <n>` for zero-based match selection. Use `--frame` to scope a command to a frame.

## Rendered page commands

Inspect:

```bash
axonbrowser chrome page inspect
axonbrowser chrome page frames
axonbrowser chrome page inspect --frame "Frame:Checkout"
```

Find, count, and read:

```bash
axonbrowser chrome page find "Heading:Example Domain"
axonbrowser chrome page count "Push Button"
axonbrowser chrome page read "Heading:Example Domain"
axonbrowser chrome page read --value "Text Box:Email"
```

Click, focus, and hover:

```bash
axonbrowser chrome page focus "Text Box:Email"
axonbrowser chrome page click "Button:Submit"
axonbrowser chrome page hover "Button:Submit"
axonbrowser chrome page double-click "Button:Submit"
axonbrowser chrome page right-click "Button:Submit"
axonbrowser chrome page click --nth 2 "Button:Edit"
axonbrowser chrome page click --frame "Frame:Checkout" "Button:Pay"
```

Type and keyboard:

```bash
axonbrowser chrome page type "Text Box:Email" "user@example.com"
axonbrowser chrome page key "Text Box:Email" Tab
axonbrowser chrome page press-enter "Button:Submit"
```

Forms:

```bash
axonbrowser chrome page check "Check Box:Accept Terms"
axonbrowser chrome page uncheck "Check Box:Accept Terms"
axonbrowser chrome page check "Radio Button:Pro Plan"
axonbrowser chrome page select-option "Combo Box:Country" "Canada"
axonbrowser chrome page upload "Button~Upload File" ./resume.pdf
```

Page scrolling:

```bash
axonbrowser chrome page scroll --direction down --amount 3
axonbrowser chrome page scroll --direction up --amount 1
axonbrowser chrome page scroll "Button:Dismiss Notice" --into-view
```

Waits:

```bash
axonbrowser chrome page wait --text "Submitted"
axonbrowser chrome page wait --title-contains "Dashboard"
axonbrowser chrome page wait --url-contains "dashboard.html"
axonbrowser chrome page wait "Button:Submit"
axonbrowser chrome page wait --state focused "Text Box:Email"
axonbrowser chrome page wait --text "Loading" --disappear
```

Action plus wait flows:

```bash
axonbrowser chrome page click-and-wait "Link:Continue" --title-contains "Step 2"
axonbrowser chrome page submit-and-wait "Button:Submit" --text "Submitted"
axonbrowser chrome page submit-and-wait "Button:Submit" --url-contains "done.html"
```

Screenshots:

```bash
axonbrowser chrome page screenshot artifacts/page.png
axonbrowser chrome page screenshot artifacts/button.png "Button:Submit"
axonbrowser chrome page screenshot artifacts/frame.png --frame "Frame:Checkout" "Button:Pay"
```

## Common workflows

### Open a page and summarize visible state

```bash
axonbrowser chrome launch https://example.com
axonbrowser chrome current --json
axonbrowser chrome page inspect
axonbrowser chrome page read "Heading:Example Domain"
```

### Fill and submit a form

```bash
axonbrowser chrome launch https://example.com/login
axonbrowser chrome page type "Text Box:Email" "user@example.com"
axonbrowser chrome page type "Text Box:Password" "password"
axonbrowser chrome page submit-and-wait "Button:Sign in" --url-contains "dashboard"
axonbrowser chrome current --json
```

### Debug selector failures

1. Run `page inspect` and identify the accessible role/name.
2. Try a contains selector such as `Button~submit` when exact names differ.
3. Use `--nth` only after confirming multiple matches exist.
4. For iframes, run `page frames` and use `--frame`.
5. Capture a screenshot when accessibility output is insufficient.

```bash
axonbrowser chrome page inspect
axonbrowser chrome page count "Push Button"
axonbrowser chrome page screenshot artifacts/debug.png
```

## Troubleshooting

- If no browser window is found, run `axonbrowser <browser> launch` before page commands.
- If page selectors fail, inspect the accessibility tree and prefer role/name selectors from the output.
- If headless automation fails, verify Xvfb, D-Bus, and AT-SPI packages are installed.
- If screenshots fail, verify ImageMagick is installed and the `import` command is available.
- If Firefox or Camoufox shell commands fail, check that WebDriver BiDi startup and browser launch completed successfully.
- If Chrome or Edge shell commands fail, check that the browser was launched by AxonBrowser or is attachable through DevTools.

## Safety and reliability

- Do not enter credentials, submit purchases, or perform irreversible actions unless the user explicitly instructs it.
- Prefer dry-run-style inspection before clicking destructive controls.
- Use screenshots or `current --json` to verify important browser state changes.
- Keep generated artifacts such as screenshots under an `artifacts/` directory when possible.
