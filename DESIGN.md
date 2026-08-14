---
version: alpha
name: Task Server / Sumi
description: >
  Self-contained design contract for Task Server — household task
  control plane (Task Card + worker claim/report) in the Sumi family.
  Dark theme is Sumi (the CSS default), light theme is Kinari; Washi
  is deliberately not adopted. Accent is teal #2f6f7e; storage-key
  prefix is task-server. Bootstrapped from the Sumi family starter
  (consulted 2026-08-07), then copied and owned. This file is the sole
  ongoing styling authority for this repository.
colors:
  # Kinari (light) palette — the set designmd validates. designmd has no
  # theme concept, so the Sumi (dark) counterpart of every token lives in
  # the Colors section below (Kinari / Sumi pairs) and is implemented in
  # client/src/global.sass. `primary` duplicates `accent` because designmd
  # requires a key color named primary; the family vocabulary is "accent".
  # Product identity is teal (Kinari #2f6f7e / Sumi #5eb8c7).
  primary: "#2f6f7e"
  accent: "#2f6f7e"
  accent-subtle: "rgba(47, 111, 126, 0.10)"
  surface: "#faf6ef"
  surface-raised: "#fffdf8"
  on-surface: "#3a2f28"
  muted: "#6f6257"
  border: "#e3d9c9"
  scrim: "rgba(58, 47, 40, 0.4)"
  link: "#14506e"
  danger: "#9c2b1d"
  danger-subtle: "#f9e9e4"
  # Sprinkle indirection hooks (see Colors): neutral in Sumi, accent wash
  # in Kinari. Components consume these, never accent-subtle directly,
  # for band/hover jobs.
  wash-base: "#e6eef0"
  wash-raised: "#eef4f5"
  hover-1: "rgba(47, 111, 126, 0.10)"
  hover-2: "rgba(47, 111, 126, 0.16)"
typography:
  title:
    fontFamily: system-ui
    fontSize: 17px
    fontWeight: 600
    lineHeight: 1.3
  body:
    fontFamily: system-ui
    fontSize: 16px
    fontWeight: 400
    lineHeight: 1.6
  body-sm:
    fontFamily: system-ui
    fontSize: 14px
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: system-ui
    fontSize: 15px
    fontWeight: 500
    lineHeight: 1.2
  caption:
    fontFamily: system-ui
    fontSize: 12px
    fontWeight: 400
    lineHeight: 1.4
rounded:
  sm: 6px
  md: 8px
  lg: 12px
  full: 9999px
spacing:
  sp-1: 4px
  sp-2: 8px
  sp-3: 12px
  sp-4: 16px
  sp-5: 24px
components:
  # Quiet controls (button-quiet, icon-button, badge) render with a
  # transparent background at runtime; the backgroundColor below is the
  # backdrop they typically sit on, so contrast is checked against it.
  app-header:
    backgroundColor: "{colors.wash-base}"
    textColor: "{colors.on-surface}"
    height: 48px
  sub-header:
    backgroundColor: "{colors.wash-raised}"
    textColor: "{colors.on-surface}"
    height: 40px
  hairline:
    backgroundColor: "{colors.border}"
    height: 1px
  card:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.md}"
    padding: 10px
  button:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: 8px
  button-hover:
    backgroundColor: "{colors.hover-1}"
  button-pressed:
    backgroundColor: "{colors.hover-2}"
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.surface-raised}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: 8px
  button-quiet:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.muted}"
    rounded: "{rounded.sm}"
  icon-button:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.sm}"
    size: 36px
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.on-surface}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    padding: 8px
  modal:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.lg}"
    padding: 16px
  modal-scrim:
    backgroundColor: "{colors.scrim}"
  radio-selected:
    backgroundColor: "{colors.accent-subtle}"
    rounded: "{rounded.sm}"
  link:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.link}"
  error-banner:
    backgroundColor: "{colors.danger-subtle}"
    textColor: "{colors.danger}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.sm}"
    padding: 8px
  spinner:
    textColor: "{colors.accent}"
    size: 18px
  badge:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.muted}"
    typography: "{typography.caption}"
    rounded: "{rounded.full}"
    padding: 4px
---

# Task Server — Sumi Family Control Plane

## Overview

Task Server is the household **task control plane**. It ships the Sumi
app shell — header, menu, theme system, a task-list top page and a Task
Card detail page — so a human can read a card (body, verification,
commit) and press an action, while workers claim and report over HTTP.

The personality is **calm, quiet, and tool-like**: content first, chrome
recedes into neutral ink tones, color only where it means something. The
audience is one professional web engineer who uses these tools daily next
to a terminal; density is welcome, onboarding is not.

Two named themes with fixed jobs:

- **Sumi (墨) — dark, the default.** `:root` IS Sumi. Design here first.
- **Kinari (生成り) — light, for screens.** Warm cream surfaces, sepia
  ink, and a limited license to decorate with faint accent washes.

**Washi is deliberately not adopted.** This product targets ordinary
screens; a later e-paper surface would replace Kinari with Washi in this
file and re-audit contrast — it does not layer Washi on top of this
contract.

This document is **self-contained**: it was bootstrapped from the
Sumi family starter (consulted 2026-08-07) and is now owned here.
All styling rules for Task Server are stated in this file.

### Ownership

This file is the product authority. Accent is teal (`#2f6f7e` / Sumi
`#5eb8c7`). Theme storage key is `task-server:theme`. Do not re-point
rules at the starter template.

## Colors

Every color is a CSS custom property (`--c-*`); components never hardcode
hex. The frontmatter carries the Kinari (light) palette; the Sumi (dark)
counterpart of every token is listed below as a Kinari / Sumi pair and
implemented in `client/src/global.sass`.

- **Surface (#faf6ef / #191919):** page background. Warm cream / ink
  off-black — never pure white or pure black.
- **Surface Raised (#fffdf8 / #232323):** cards, modals, bands.
- **On-Surface (#3a2f28 / #e6e6e6):** primary text. ~11:1 on Kinari
  surface, comfortably AA+ on Sumi.
- **Muted (#6f6257 / #9a9a9a):** secondary text, captions, metadata,
  quiet icons. ≥ 4.5:1 (AA) against surface in both themes.
- **Border (#e3d9c9 / #333333):** 1px hairlines — the primary separation
  tool of this flat system.
- **Accent (#2f6f7e / #5eb8c7):** the project identity color (teal).
  Marks the primary action, the focus ring, the selected state, the
  spinner — "you are here / this is the main move". One accent-filled
  element per screen region. The selected-state tint is `accent-subtle`
  (rgba(47,111,126,.10) / rgba(94,184,199,.15)).
- **Link (#14506e / #7fdbff)**, **Danger (#9c2b1d / #ff6b6b)** with
  `danger-subtle` tints (#f9e9e4 / #3a1a1a) for error banners.
- **Scrim (rgba(58,47,40,.4) / rgba(0,0,0,.6)):** modal backdrop.

**Sprinkle indirection (the Kinari license, made mechanical).** Four
semantic hooks decouple "where warmth appears" from component code:

| Hook              | Job                                      | Sumi resolves to        | Kinari resolves to     |
| ----------------- | ---------------------------------------- | ----------------------- | ---------------------- |
| `--c-wash-base`   | app-header band background               | `#232323` (raised)      | `#e6eef0` (teal wash) |
| `--c-wash-raised` | sub-header / sticky band background      | `#191919` (page surface — the band sits flush with the page, separated by its hairline alone) | `#eef4f5` (faint teal wash) |
| `--c-hover-1`     | hover fill (buttons, rows, menu items)   | `#333333` (border gray) | `rgba(47,111,126,.10)`  |
| `--c-hover-2`     | pressed / active-row fill                | `#3d3d3d`               | `rgba(47,111,126,.16)`  |

Components consume the hook, never `accent-subtle` directly, for these
jobs. Sumi stays strictly neutral; Kinari warms up with zero
per-component branching. Washes are decoration only — every meaning they
touch must also be carried by text or shape.

**Theme mechanism.** `:root` carries the Sumi values and
`color-scheme: dark`. Kinari is applied by two equivalent blocks (kept
identical via one Sass mixin), each also setting `color-scheme: light`:

- `:root[data-theme="light"]` — explicit user choice;
- `@media (prefers-color-scheme: light)` → `:root:not([data-theme="dark"])`
  — OS decides when no explicit choice is set.

`data-theme` on `<html>` takes `"dark"` or `"light"`; the auto setting
**removes the attribute** (and the storage key) so the OS rules.
Preference persists in `localStorage` under `task-server:theme` and is applied before first paint.

The primary button sets its text with the `surface-raised` token, so it
is dark-on-teal in Sumi (≈ 8:1) and warm-white-on-teal in Kinari
(≥ 4.5:1) with no extra token. All text keeps WCAG AA in both themes.

## Typography

One typeface — the platform `system-ui` stack. No webfonts. Exactly five
roles, exposed as font-size tokens `--fs-xs..xl` (12/14/15/16/17px):

- **Title (`--fs-xl` 17px / 600 / 1.3):** screen and item titles, modal
  headers. Single line, ellipsized.
- **Body (`--fs-lg` 16px / 400 / 1.6):** main reading text. Never smaller.
- **Body Small (`--fs-sm` 14px / 400 / 1.5):** summaries, list subtitles,
  state messages.
- **Label (`--fs-md` 15px / 500 / 1.2):** buttons, menu items, the app
  title.
- **Caption (`--fs-xs` 12px / 400 / 1.4):** timestamps, statuses,
  metadata — always `muted` unless carrying a data color.

If a new size feels needed, use weight or muted color instead.

## Layout

The shell stacks three rows:

1. **App header — invariant on every page.** Sticky, 48px, full width,
   `--c-wash-base` background, 1px bottom hairline. Contents are exactly
   two: the app title as a home link (`<a href="/">`, label type,
   on-surface ink, no underline — left) and the hamburger icon-button
   (right). **The title is the header's only navigation link**; all
   other navigation lives inside the menu, so phone widths never crowd.
2. **Sub-header — detail screens only.** 40px, `--c-wash-raised`, 1px
   bottom hairline, holding only the current item's title (label,
   single line, ellipsized). No back button — going back is the header
   title link or the browser itself.
3. **Main content**, the only scrolling region.

One breakpoint: **768px**. Below it, a single column with `--sp-3` side
gutters; at and above, the content column centers at max-width 720px with
`--sp-5` gutters. Bands stay full-width at all widths. The page never
scrolls horizontally at 320px and up.

Spacing snaps to the 4px scale `--sp-1..5` (4/8/12/16/24px). Default
rhythm: 8px gap between cards, 10px card padding, 16px modal padding.
No off-scale values.

## Elevation & Depth

The system is **flat**. Hierarchy comes from tonal layers (surface →
surface-raised → wash bands) plus 1px hairlines. Exactly one shadow
exists: floating modals/menus cast `0 8px 32px rgba(0, 0, 0, 0.25)` over
the scrim. No other `box-shadow` anywhere.

**Focus ring:** defined once globally on `:focus-visible` —
`outline: 2px solid var(--c-accent); outline-offset: 2px`. The UA
default ring is suppressed only because this replaces it; focus
indication is never removed outright.

## Shapes

Soft-rectangle language, tokens `--radius-sm/md/lg/full` (6/8/12/9999px):

- **sm (6px):** buttons, inputs, all small controls.
- **md (8px):** cards and list rows.
- **lg (12px):** modals and floating menus.
- **full:** count pills and the status badge only.

Never mix radii within one composite control. No circular buttons.

## Iconography

All icons come from **one dictionary component**,
`client/src/lib/Icon.svelte`: `<Icon name="menu" />` renders inline SVG
on a 24×24 grid — `fill="none" stroke="currentColor" stroke-width="2"
stroke-linecap="round" stroke-linejoin="round"` (Lucide style), default
size `1.2em`, baseline-aligned, inheriting the text color of its context.

Current dictionary: `menu`, `x`, `sun`, `moon`, `monitor`,
`chevron-left`, `trash`, `megaphone`, `megaphone-off`, `pencil`,
`refresh-cw`, `check-check`, `mail`, `book`, `search`, `star`,
`star-filled`.

Outline is the unnamed default: a `-filled` variant shares its outline
sibling's geometry and overrides `fill` to `currentColor` on the shape
itself — the root svg stays `fill="none"` for every entry. A filled
variant is a visual state only; the control using it must still carry
that state accessibly (e.g. `aria-pressed`), never through color alone.

`Icon.svelte` also exports `ICON_NAMES`, the canonical array of every
dictionary entry. Anything that enumerates the dictionary — the
アイコン辞書 fixture's specimen page — renders from that export, never
from a hand-copied list. The dictionary is a vocabulary, not a usage
report: an entry (e.g. `chevron-left`) stays even while no screen
currently uses it.

- **Emoji are banned as UI icons**, and so are text glyphs standing in
  for icons (▲ ▼ × ☰ ▶ …) — always an SVG entry in the dictionary.
- **Adoption rule:** this template's dictionary is the family's
  canonical copy source. A derived project adds new icons to its own
  `Icon.svelte`; icons that prove generally useful are normalized to the
  24×24 Lucide grammar above and adopted into this dictionary first.
  After adoption, each project receives an explicit, separate delivery
  that replaces its local or inline SVGs with the template's
  name-and-geometry entry — no automatic sync, no submodule, no runtime
  dependency; every project's DESIGN.md and build stay self-contained.

## Components

- **App header:** per Layout. The title link keeps on-surface ink with
  no underline (chrome, not content — the `link` token is for body
  links). The hamburger is a 36px quiet icon-button with `aria-label`
  and `aria-expanded`.
- **Menu (from the hamburger):** a dropdown panel spatially anchored to
  the hamburger, not a modal — absolutely positioned at `top: 100%` /
  `right: 0` within the header's positioned right slot, `min-width`
  180px, surface-raised background, 1px hairline border, lg radius with
  `overflow: hidden`, and the single floating shadow. There is **no
  scrim**; a transparent `position: fixed` full-viewport close button
  sits behind the panel so any outside click closes it. Esc also
  closes; closing always returns focus to the hamburger, and
  `aria-expanded` mirrors the open state. Items are full-width
  borderless rows — label type, `--sp-2`/`--sp-3` padding, left
  aligned, transparent background, hover `--c-hover-1`, square corners
  clipped by the panel's lg radius. **Item 1 is always テーマ設定**,
  which opens the centered theme settings modal; page-navigation links
  of derived projects follow it. There is no トップ/home item — the
  header title already is the home link.
- **Theme settings modal:** opened from the menu's テーマ設定 item; the
  centered modal (lg radius, 16px padding, scrim + shadow) holding a
  `role="radiogroup"` with three radios — 自動 (`monitor`), ライト
  (`sun`), ダーク (`moon`). Selecting applies immediately (attribute +
  storage) and **does not close the modal** — the user watches the
  theme change live. Close via ×, Esc, or scrim; focus returns to the
  hamburger.
- **Top page — task list:** cards per the family recipe
  (surface-raised, 1px hairline, 8px radius, 10px padding) in a single
  column with 8px gaps; each card links to its Task Card and shows the
  task title (label) and status (caption muted). The list container
  exposes `data-state="loading|empty|error|success"`:
  - _loading:_ centered muted body-sm text with the accent spinner
    (1.5px-stroke circle, 1.1rem);
  - _empty:_ centered muted body-sm message;
  - _error:_ danger-colored body-sm message plus a default retry button;
  - _success:_ the cards.
- **Detail page — Task Card:** sub-header (title only) over a content
  column showing status (an outline badge — caption type, 1px border,
  muted text; neutral chrome, not a data color), `commit_sha` and
  `verification` as muted captions when present, body text (body, 1.6,
  pre-line), and `available_actions` as a row of default buttons. The
  `done` action, when present, is the single primary (accent-filled)
  button. `bump-tag` expands to three buttons posting `bump` =
  patch / minor / major. After a successful action the card reloads so
  status and actions update. There is no icon-dictionary fixture page.
- **Buttons:** default = surface-raised bg, 1px hairline, label type,
  sm radius, 8×14px padding, hover fills `--c-hover-1`. Primary =
  accent bg, `surface-raised`-token text — at most one per screen.
  Quiet = transparent, for icon-buttons in bars. Disabled = 50%
  opacity, no pointer.
- **Inputs:** surface bg (one layer below their container), 1px
  hairline, sm radius, body type; focus swaps border to accent under
  the shared focus ring. Labels are caption muted above the field.
- **Modals:** centered, lg radius, 16px padding, scrim + the single
  permitted shadow; close via ×, Esc, scrim; content scrolls
  internally, max-height 80dvh.
- **Motion:** utilitarian only — height/opacity transitions ≤ 150ms and
  the spinner. Honor `prefers-reduced-motion: reduce` by disabling both.
- **Navigation state:** every page has a router-backed URL; reloads
  restore the same view. The chosen theme is never held only in
  component state.

## Implementation Mapping

- Styling is **Sass indented syntax (`.sass`)** with **normalize.css**
  imported first.
- All tokens live in `client/src/global.sass` on `:root` (Sumi values);
  the two equivalent Kinari blocks are emitted from a single Sass mixin
  so they cannot drift.
- Canonical custom-property names: colors `--c-<token>`
  (`--c-surface`, `--c-on-surface`, `--c-accent`, `--c-wash-base`, …),
  spacing `--sp-1..--sp-5`, font sizes `--fs-xs..--fs-xl`, radii
  `--radius-sm/md/lg/full`. Components consume variables only.
- Theme bootstrap script: read `task-server:theme`; `"light"` /
  `"dark"` set `data-theme` on `<html>` before first paint; absent key
  (auto) leaves the attribute off. `Icon.svelte` is the sole icon
  source.

## Verification

- `designmd lint` validates the frontmatter structure.
- UI claims in this document are verified **in a real browser** against
  DOM, computed styles, geometry, and operations — never by reading
  source alone. The standing invariants:
  1. Default (no `data-theme`): `color-scheme` is `dark`, body
     background computes to `rgb(25, 25, 25)`.
  2. Choosing ライト in the theme modal sets `data-theme="light"`,
     turns the body `rgb(250, 246, 239)`, writes the storage key, and
     leaves the modal open.
  3. At 375px the header contains exactly two interactive elements —
     the title `<a href="/">` and the hamburger `<button>` — and
     `document.documentElement.scrollWidth` never exceeds the
     viewport, with the menu closed or open.
  4. Cards compute to 1px border / 8px radius / 10px padding / 8px gap;
     the list's `data-state` reflects loading, empty, error, success.
  5. Chrome icons are all inline SVG on the 24×24 viewBox grid, stroked
     with `currentColor` and rendered at 1.2em; no emoji or glyph icons
     anywhere.
  6. `:focus-visible` on any control shows the 2px accent outline with
     2px offset.
  7. Clicking the hamburger opens the dropdown: the panel's top edge
     meets the header's bottom edge and its right edge aligns with the
     hamburger's right edge (±1px); computed `min-width` 180px, 12px
     radius, 1px border, the single floating shadow; no scrim element
     exists and `aria-expanded` is `true`. Esc closes it and focus
     returns to the hamburger; a click outside the panel also closes
     it. Item 1 reads テーマ設定 and opens the centered theme modal.
  8. A Task Card sub-header contains the task title and zero
     buttons or links.
  9. A Task Card shows body, verification, commit_sha, status, and one
     control per available action (`bump-tag` appears as three bump
     buttons). After a successful POST the displayed status and actions
     match the reloaded card.

## Do's and Don'ts

- Do source every color from a `--c-*` variable; don't hardcode hex in
  components.
- Do consume `--c-wash-*` / `--c-hover-*` for bands and hovers; don't
  reach for `accent-subtle` directly in those jobs.
- Do keep exactly one accent-filled primary action per screen.
- Do present the menu as a hamburger-anchored dropdown; centered
  modals are for dialogs (theme settings), never for navigation.
- Don't use emoji or text glyphs as icons; every icon is an
  `Icon.svelte` dictionary entry.
- Don't introduce font sizes, radii, spacing values, or shadows outside
  the defined scales — the modal shadow is the only shadow.
- Do give the list every one of its four states; don't ship a page where
  error or empty renders as blank.
- Do maintain WCAG AA (4.5:1) for all text in both themes; verify in
  the browser, not by eye.
- Do design in Sumi first, then verify Kinari as a warm sibling — never
  as an inverted afterthought.
- Don't re-point any rule at the canonical templates; adapt changes into
  this file explicitly.
- Do keep the theme storage key `task-server:theme` and the teal
  accent pair; don't ship this product wearing the starter amber.
