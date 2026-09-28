## AxonBrowser v1.1.2

Fixes for the regressions and gaps found driving the Air Peace booking widget
on a live run, plus the stability work behind them. Every change is in the
shared action layer, so Chrome, Edge, Firefox, and Camoufox all behave the same.

### The regression, fixed

- **A field parked under a sticky header is no longer clicked through to the
  page behind it.** v1.1.1 lost this. `scroll-into-view` parks a target at the
  viewport edge, which is exactly where a sticky header lives, so the click
  landed on a navigation menu and claimed success. The point is now checked
  against the page's own hit test before the click is sent, and a target that is
  covered is moved to the middle of the viewport first. On the live site the
  date field was measured at viewport y=0 under the header, moved to y=334, and
  the picker opened; v1.1.1 navigated to a different page from the same state.

### Also fixed

- **`select-option --nth` addresses the control, not the option.** On a page
  with two combo boxes, `--nth 1` looked for the *second option* with that label
  and failed outright. Verification also re-read the wrong control and reported
  a successful selection as unverified.
- **A selection that worked is no longer reported as unverified.** The value is
  now read from the page, which is authoritative, so a native select and a
  custom dropdown both verify correctly.
- **Modals are genuinely dismissed.** Detection matched roles that ordinary page
  content also has, so it either missed the popup or dismissed the page. The
  page is now asked what is actually covering it — a declared dialog or a
  full-viewport fixed layer — and the cover is confirmed gone before the action
  proceeds.
- **A click that changes nothing is no longer reported as success.** A radio on
  the live form answered "clicked" while the page still showed the other option
  selected. Checked state is read before and after, and a click that did not
  change it fails. Combo boxes are verified by their list opening, because a
  custom dropdown is often a `div` that never takes focus.
- **The accessibility tree no longer collapses.** Three causes were found and
  fixed: a repair path that killed the bus under a running browser (which never
  re-registers), a launch that did not ensure the stack was up before the
  browser started, and duplicate registries competing for the same name. The
  retry that absorbs an empty tree read also never ran, because it matched an
  error message the code no longer produced.
- **Commands can no longer hang.** Every call into the accessibility bus is a
  D-Bus round trip with no timeout of its own, so a half-dead bus made a command
  block forever with no output. Connections and the overall command are now
  bounded, and a stall reports what happened instead of hanging.
- **A dead bus is reported as such.** A browser window on screen with an empty
  tree now says the browser is not registered with the accessibility bus and
  needs restarting, instead of the unhelpful "no page matches".

### Verified

- 16/16 page-command regression on the demo site, twice in a row, where the
  same run previously collapsed the tree partway through.
- Live Air Peace run: tree stable at 595 lines across six consecutive reads,
  Round Trip confirmed selected in the page, departure date set to 2026-10-08,
  no navigation from any form click.

### Install

No Rust toolchain required.

```bash
curl -fsSL https://raw.githubusercontent.com/Axon56/AxonBrowser/main/install.sh | bash
axonbrowser install-deps
```

### Release assets

- `install.sh`
- `axonbrowser-linux-x86_64.tar.gz` and `.sha256`
- `axonbrowser-linux-aarch64.tar.gz` and `.sha256`
