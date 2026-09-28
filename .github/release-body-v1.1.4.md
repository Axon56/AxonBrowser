## AxonBrowser v1.1.4

Reliability work on the page action layer, from a live run against a large
JavaScript-heavy site with a sticky header, stacked overlays, and several
identically labelled controls. Every change is shared by Chrome, Edge, Firefox,
and Camoufox.

### Wrong control no longer selected

Selecting an option could apply it to a different control and overwrite a value
that was already set. Two dropdowns on one form routinely offer the same labels,
and a dropdown widget renders its list as a *sibling* of the control, so every
dropdown's options sit in the accessibility tree at the same level. Resolving an
option by label matched the first list on the page rather than the open one.

The option is now chosen by geometry: only a node positioned inside the rectangle
of the list that is actually open is accepted, and there is no page-wide fallback.

### A selection that worked is no longer reported as unverified

The value was read by hit-testing the control's point, and by name-based lookups
that many controls do not have. A control sitting under a sticky header read back
as whatever was drawn over it, and an unnamed control read back as nothing, so a
real selection came back as a failure and the caller retried a step that was
already done.

The control is now found by geometry -- the smallest form control whose box
contains the point -- which reads through anything drawn over it, and a wrapper's
inner control is read as well because that is where a custom widget keeps its
chosen label. A genuine failure is a real error naming the observed and expected
values, not a note attached to a success line.

### The index addresses the control, never the option

`--nth` selects which control to act on. It used to index duplicate option
labels, which made the second control unaddressable and failed outright when only
one option matched. The option is always identified by its label.

### First-click aim with overlays up

Aiming happened before the overlay was dismissed, so the opening click was
resolved against a point the overlay owned and the step needed a retry it should
never have needed. Any cover is now cleared before the first coordinate is
resolved, and the coordinate is re-resolved after each dismissal.

### A target under a sticky header is centred before the first hit test

Accessibility scroll-to parks a target at the viewport edge, which is where a
sticky header lives. The target is now moved to the middle of the viewport before
the first hit test, not only when a retry is needed, so a readonly date field
cannot click through to the navigation menu.

### A repeated accessible name no longer moves the wrong control

Two fields often share one accessible name, such as a departure and a return
date. Moving the first document match scrolled the wrong field, and the click then
landed on empty space. The control nearest the target's reported point is chosen
instead.

### Faster page queries

Chrome's DevTools HTTP endpoint can keep the socket open even when
`Connection: close` was requested, so every page query paid the full two-second
read timeout. The read now stops as soon as the declared body is complete, which
takes a page query from about two seconds to milliseconds. The overlay scan no
longer walks every node in the document either; it inspects only the element at
the viewport centre and its ancestors, which on a large page could hold the
query open long enough to stall a click.

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
