## AxonBrowser v1.1.3

Fixes for `select-option` on a form with two dropdowns, from a live run against
a booking flow. All four reported problems are addressed, plus the scroll-state
wobble. Every change is in the shared action layer, so Chrome, Edge, Firefox, and
Camoufox behave the same.

### `select-option` no longer touches the wrong control

This was the worst of the batch: with the destination dropdown open, selecting
a value applied it to the origin control and overwrote the value already set
there.

The cause is that a dropdown widget renders its list as a *sibling* of the
control, so every dropdown's options sit in the accessibility tree at the same
level, and both airport controls offer the same labels. Resolving the option by
label therefore matched the first list on the page rather than the open one.

The option is now chosen by geometry: only a node positioned inside the rectangle
of the list that is actually open is accepted, so an identically named option in
another control's list can never be picked. There is no page-wide fallback.

### The index addresses the control, never the option

`--nth` selects which control to act on. It used to index duplicate option
labels, which made the second combo box unaddressable and, on a dropdown with one
matching choice, failed outright. The option is always identified by its label.

### A selection that worked is no longer reported as unverified

Verification read the control's value through lookups that required an accessible
name, and both airport combos are unnamed, so a successful selection came back as
"selection could not be verified" and the caller retried. The value is now read
from the page at the control's own position, which needs no name, with the
control's own text as a second signal.

A genuine failure is now a real error that names the observed and expected
values, rather than a note attached to a success line.

### First-click aim with overlays up

Aiming happened before the overlay was dismissed, so the opening click was
resolved against a point the overlay owned and the step needed a retry it should
never have needed. Any cover is now cleared before the first coordinate is
resolved, and the coordinate is re-resolved after each dismissal.

### The scroll-state wobble

A click that did not open its dropdown is now re-resolved and retried, bounded,
instead of failing the step. A control that genuinely will not open is still
reported.

Alongside it, two page-reading defects that made the wobble look worse were
fixed: a closed dropdown kept in the markup with its height collapsed was treated
as open, because a sliver still receives hits and its clipped options still report
their own box; and a value read from a wrapper counted text belonging to hidden
options, so a selection that never happened looked like it had worked.

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
