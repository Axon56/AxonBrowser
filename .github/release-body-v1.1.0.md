## AxonBrowser v1.1.0

Reliability release. This version is about the tool telling the truth: it no
longer reports clicks or typing that did not happen, and it recovers instead of
dead-ending when a page hides itself.

Found by driving a real airline booking flow, where the booking widget sits
under a sticky header, behind popups, and inside a collapsed tab.

### Fixed

- **Clicks no longer land on the wrong element.** A click point is checked
  against the page before the click is sent. If a sticky header or a modal owns
  that point, the click is refused and names what blocked it, instead of being
  reported as a successful click on your target. Applies to the page layer and
  to the shell click fallback, on Chrome, Edge, and Firefox.
- **Targets are repositioned out of the header.** Scroll-into-view parks an
  element at the nearest viewport edge, which is exactly where sticky headers
  live. Targets are now moved into a safe band of the window before clicking.
- **Typing fails instead of lying.** `page type` used to print success after its
  own verification had failed twice. It now retries once and then reports the
  observed value.
- **Disabled controls are refused.** A disabled control looks identical to an
  enabled one in the accessibility tree, and activating it reports success
  while nothing changes. Disabled state is now confirmed against the page.
- **Modals no longer collapse the page.** A modal marks the content behind it
  `aria-hidden`, so the accessibility tree legitimately contains only the
  dialog and every lookup failed with "no page matches". The dialog is now
  detected, dismissed through its own close control, and the lookup retried.
- **Selectors match roles exactly.** `entry` and `combo box` were treated as
  interchangeable, so `Combo Box --nth 2` could land on a date field and the
  indices shifted as a form changed state.
- **Firefox launch works on a fresh profile.** A privileged `about:home`
  context aborted BiDi context enumeration and launch timed out even though
  Firefox was running.
- **`resize` works.** It shelled out to `wmctrl`, which is not installed and
  cannot drive windows without a window manager. It now uses `xdotool`.
- **Chrome no longer segfaults after synthetic input.** Sending a key could
  kill the browser; a short settle after input removes the race.
- **Unintended navigation is reported.** A click on a form control that changes
  the URL now fails loudly, because it means the form state is about to be
  lost.
- **Unreadable counts.** An out-of-range `--nth` reported one more match than
  existed.

### Install

Download a prebuilt binary. No Rust toolchain required.

```bash
curl -fsSL https://raw.githubusercontent.com/Axon56/AxonBrowser/main/install.sh | bash
axonbrowser install-deps
```

### Release assets

- `install.sh`
- `axonbrowser-linux-x86_64.tar.gz` and `.sha256`
- `axonbrowser-linux-aarch64.tar.gz` and `.sha256`

### Notes

- Targets Linux with X11. Headless hosts get an X11 session bootstrapped
  automatically.
- `axonbrowser install-deps` installs the runtime packages and a browser.
