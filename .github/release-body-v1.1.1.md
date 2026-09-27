## AxonBrowser v1.1.1

Four fixes from driving an airline booking flow, specifically its airport
dropdown and date picker.

### Fixed

- **`select-option` now works on custom dropdowns.** It only matched `Option`
  and `Menu Item`, so a dropdown whose choices are `List Item` reported "no page
  option matched" and the only way through was clicking by name. Option roles
  are now tried in turn, and `--nth` is available so duplicate labels are
  resolvable.
- **Subtrees no longer vanish from the tree.** Cycle detection used a single
  visited set for the whole walk, so when a browser recycled an object reference
  — which it does for virtualized and re-rendered nodes — the second appearance
  was emitted without its children and everything underneath disappeared. A
  search input inside an open dropdown went missing this way even though the
  browser's own accessibility tree exposed it.
- **`readonly` is no longer treated as `disabled`.** Date pickers are readonly
  by design and clicking one is how the picker opens. The actionability check
  walked up to four ancestors looking for a disabled marker, so a wrapper
  carrying such a class vetoed the click and the field was wrongly refused.
- **Modals are dismissed rather than clicked through.** Clicking a backdrop
  happens to work on some sites and fails on any site that traps pointer events
  or focus. A modal covering the page is now closed via its own close control
  before the action runs, and the target re-resolved afterwards.

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
