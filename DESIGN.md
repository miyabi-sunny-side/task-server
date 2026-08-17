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
app shell — header, menu, theme system, an operator top page and a Task
Card detail page — so a human can read a card (body, verification,
commit) and press an action, while workers claim and report over HTTP.
The top page is where the two screen-level moves live: **merge** (issue
the instant tasks that land finished work) and **release** (ship the
landed work of one product under a tag), over the open work grouped by
status.

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

**Screen-level controls are content, not chrome.** A screen that offers
actions on the whole screen's subject — the top page's merge and release
— places them as the **first block inside the content column**. It is
never a third band, never sticky, and never full-width: the two bands
above stay the only bands, and main content stays the only scrolling
region, so the controls scroll away with the work they act on.

One breakpoint: **768px**, and it moves the **vertical** rhythm only.
The content column is `max-width: 720px`, centered at every width, with
`--sp-3` (12px) side gutters that never change — the max-width already
does the horizontal work, so widening the gutter on a wide screen would
only shorten the reading measure for nothing. Vertical padding is
`--sp-4` below the breakpoint and `--sp-5` at and above it. Every box is
`border-box`, so on a viewport wide enough for the whole column the
column measures 720px and its children 696px. Bands stay full-width at
all widths. The page never scrolls horizontally at 320px and up.

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

## App Icon & Install Manifest

The app icon is a **separate register from the `Icon.svelte` dictionary**,
and the two never trade places. A dictionary entry is a 24×24 stroked
glyph that inherits `currentColor` from the text beside it; the app icon
is a standalone tile an OS draws with no context, no text, and no theme,
so it carries its own ground and its own fixed colors. Neither is
authored from the other, and the dictionary gains no entry from this
chapter.

**One source, tracked rasters.** `client/public/icon.svg` is the original
drawing; every raster is rendered from it with `rsvg-convert`, and the
rendered PNGs are tracked as well — `client/dist/` is gitignored and Vite
copies `client/public/` through untouched, so `public/` is the only place
a shipped asset can live. No second drawing exists.

**The mark.** A 512×512 viewBox holding exactly two shapes:

- **Ground:** the full square in `#191919` (Sumi surface), corner radius
  64 — 12.5% of the edge, which lands on `--radius-sm` (6px) at the ~48px
  a launcher actually draws. This is the one radius outside the CSS radius
  scale, permitted because the tile is not a CSS box: every OS re-masks it
  anyway.
- **Mark:** one path, `M112 264 200 352 392 160`, `fill="none"`,
  `stroke="#5eb8c7"` (Sumi accent), `stroke-width="56"`, round cap and
  round join. Both arms are exact 45° diagonals in an 88 : 192 ratio — a
  geometric check, not a drawn one. The stroke is heavier than the
  dictionary's 2/24 because this mark stands alone at 48px with no label
  beside it.
- **Flat, per Elevation & Depth:** two flat fills, no gradient, no
  shadow, no third color. No emoji, no text glyph, no lettering — the
  Iconography bans hold here too.

**Teal on Sumi, never the inverse.** `:root` IS Sumi, so the product's
self-portrait is dark and matches the `theme-color` the launcher paints
behind it. It also measures better: `#5eb8c7` on `#191919` is 7.67:1,
while a dark mark on a `#2f6f7e` flood is 3.09:1 and loses its shape at
launcher scale. And a neutral tile carrying one accent stroke is the UI's
own grammar — one accent-filled element per region — where an
accent-flooded tile would make the accent the ground.

**One geometry, two corner treatments.** All ink stays inside the central
80% circle (max ink radius 194.2 against the 204.8 the Android maskable
safe zone allows), so nothing meaningful can be cropped and no second,
tighter drawing is needed. The purposes differ in the corners alone:

| File                            | Render                        | Job                                                                                                                     |
| ------------------------------- | ----------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `icon.svg`                      | the source                    | document favicon (`rel="icon"`)                                                                                         |
| `icon-192.png` / `icon-512.png` | `rsvg-convert -w N -h N`      | manifest `purpose: "any"` — transparent corners, so the tile keeps its own radius where nothing masks it                 |
| `icon-maskable-512.png`         | the same, plus `-b '#191919'` | manifest `purpose: "maskable"` — corners flattened to the ground so a square or squircle mask finds no transparent notch |
| `apple-touch-icon.png`          | 180px, `-b '#191919'`         | iOS, which never reads manifest icons and composites transparency on black                                              |

**Manifest.** `client/public/manifest.webmanifest`, linked from
`client/index.html`. `id`, `start_url`, and `scope` are all `/`, so the
app resolves against whatever host and port serves it — the tailnet URL
carries a non-default port and installability must not depend on it.
`display` is `standalone`; `lang` is `ja`. Icons are **PNG only**: the
tracked SVG is the origin of the rasters, not a delivery format for
launchers.

- **`name` is `Task Server`** — the same string as `<title>`.
- **`short_name` is `Task`**, the launcher label. `task` is the noun this
  Japanese UI already uses untranslated ("merge 可能な task はありません"),
  so the label speaks the UI's register without inventing a katakana
  translation, and four characters never truncate or wrap where
  `Task Server` sits at the 12-character ceiling. The full name still
  shows in the install prompt.
- **`description` is Japanese**, matching `lang="ja"` and the UI's voice —
  it is install-UI copy read by the operator. `<meta name="description">`
  is document metadata for a different audience and is not governed here.
- **`theme_color` and `background_color` are both `#191919`.** The
  manifest is theme-blind and therefore states Sumi, the default theme.
  A Kinari user's splash is dark for the frame before the bootstrap
  script paints — the same trade the existing
  `<meta name="theme-color" content="#191919">` already makes, not a new
  regression. Naming Kinari's `#faf6ef` instead would be wrong for the
  default theme in order to be right for the override, and no
  `prefers-color-scheme` variant can see an explicit `data-theme` choice
  anyway.

**Installability stops at manifest and icons.** There is no service
worker and no offline behaviour; adding either is its own delivery, not a
consequence of this chapter.

**No new tokens.** `#191919` and `#5eb8c7` are the Sumi counterparts of
`surface` and `accent`, already documented in Colors. The frontmatter
carries the Kinari palette, and a theme-blind asset has no Kinari
counterpart to carry, so it earns no token pair.

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
- **Top page — control panel over the task list.** The operator's
  screen: one panel of screen-level controls, then the open work grouped
  by status. It reads two sources — the control plane (what is
  mergeable, what merges are already in flight, what is releasable) and
  the task list — and **each region carries its own
  `data-state`**, so a failure or an emptiness on one side never masks
  the other.

  **Control panel.** The first block of the content column, built from
  the card recipe (surface-raised, 1px hairline, md radius, 10px
  padding), `--sp-3` between its rows. It holds two control rows, each a
  button beside a note that explains the button's current standing:

  - **merge** — issues a merge task for every currently mergeable task,
    one request per task, with **no confirmation step**: nothing is
    chosen, so nothing is asked. It is the top page's **single primary
    (accent-filled) button, and only while it is enabled**; with nothing
    to merge it wears the default treatment plus the disabled one.
  - **release** — a **default button, never accent-filled**, that opens
    the release modal. Merge is the routine daily move and release is
    the occasional one, so merge takes the page's one accent fill and
    release takes its own inside the modal it opens.

  Each note takes one of exactly two shapes: the **count pill** (badge
  recipe, full radius) of what the button would act on when that count
  is above zero, or the **muted caption naming the reason** when it is
  zero — "merge 可能な task はありません" / "release 可能な product は
  ありません". A disabled control always says why in text beside it and
  points at that text with `aria-describedby`; opacity alone never
  carries the reason.

  When merges are in flight, the panel carries them below the rows: a
  muted caption ("merge 進行中") over the ordinary card list of those
  merge tasks. **A merge in flight is the state of the control, not open
  work** — it appears here and nowhere in the status groups.

  The panel exposes `data-state="loading|empty|error|success"` on the
  same discipline as the list: _loading_ is the centered spinner line
  with no control rows rendered (never a button that might be showing
  the wrong standing); _error_ is the danger body-sm line plus a default
  retry button; _empty_ — nothing mergeable, nothing pending, nothing
  releasable — still renders **both buttons, both disabled, both with
  their reason**, because a control that vanishes when idle teaches
  nothing about what would bring it back; _success_ is the panel above.

  **Result line.** One live region (`aria-live="polite"`) directly under
  the control rows reports the outcome of the last control action: the
  spinner line while an action is in flight, a muted caption on success
  ("merge task を 3 件発行しました"), and the error-banner recipe with
  `role="alert"` on failure. It persists until the next action —
  **there is no toast, no timer, and no auto-dismissing message
  anywhere in this product**. Every successful action reloads both
  regions, so the changed counts and lists are the real receipt.

  **Task list.** Below the panel, the open tasks grouped by status.
  `released` is never shown, and `instant:merge` tasks never appear
  (they belong to the panel). Groups follow the status vocabulary order
  — `draft`, `ready`, `wip`, `done`, `merged`, then the sidetracks
  `blocked`, `cancelled`, `dropped` — and **a group holding nothing is
  not rendered at all**, heading included. Each group is a section
  carrying its status as a data attribute, headed by a label-type
  heading naming the status with its count pill beside it; under the
  heading, cards per the family recipe (surface-raised, 1px hairline,
  8px radius, 10px padding) in a single column with 8px gaps. A card
  links to its Task Card and shows the task title (label) with the
  product id as a muted caption — the group heading already carries the
  status, so the card does not repeat it. The list container keeps
  `data-state="loading|empty|error|success"`:
  - _loading:_ centered muted body-sm text with the accent spinner
    (1.5px-stroke circle, 1.1rem);
  - _empty:_ centered muted body-sm message;
  - _error:_ danger-colored body-sm message plus a default retry button;
  - _success:_ the groups.

- **Release modal:** opened by the release button; the standard centered
  modal (lg radius, 16px padding, scrim + shadow; ×, Esc, or scrim
  closes it and focus returns to the release button). Contents in
  order — the product choice, the tag field, the action row:
  - **One releasable product:** no chooser. A muted caption names the
    product id and how many merged tasks would ship.
  - **Several:** a `role="radiogroup"` of product rows reusing the
    selected-radio treatment (accent-subtle fill, sm radius), each row
    showing the product id (label) and its count pill; the first row is
    selected when the modal opens, so the field below is always
    meaningful.
  - **The tag is required.** One input per the Input recipe with its
    caption-muted label above it, focused when the modal opens. The
    confirm button is disabled while the field is blank or whitespace,
    with the same rule as everywhere: the reason sits in text beside it.
  - The action row is キャンセル (default) and the confirm
    (accent-filled). **An open modal is its own primary region**, so
    this confirm is the modal's single accent fill and does not compete
    with the merge button behind the scrim.
  - A refused release keeps the modal open **with the typed tag
    intact** and shows the server's message in the error-banner recipe
    inside the modal — a rejected tag is corrected where it was typed.
    A successful one closes the modal and reports on the panel's result
    line.
- **Detail page — Task Card:** sub-header (title only) over a content
  column showing status (an outline badge — caption type, 1px border,
  muted text; neutral chrome, not a data color), `commit_sha` and
  `verification` as muted captions when present, body text (body, 1.6,
  pre-line), and `available_transitions` as a row of default buttons,
  one per reachable status. An `instant:merge` task carries a second
  outline badge next to the status. The `ready` transition, when
  present, is the single primary (accent-filled) button — it is the
  human decision the screen exists for. After a successful transition
  the card reloads so status and buttons update. There is no
  icon-dictionary fixture page.
- **Buttons:** default = surface-raised bg, 1px hairline, label type,
  sm radius, 8×14px padding, hover fills `--c-hover-1`. Primary =
  accent bg, `surface-raised`-token text — **at most one per primary
  region**, and a screen has exactly two kinds of region: the page and
  an open modal. So the page shows at most one accent fill and an open
  modal shows at most one of its own. A control that is primary only
  while it is available **drops to the default treatment when it is
  not** — a dimmed accent fill is never the disabled look.
  Quiet = transparent, for icon-buttons in bars.
  **Disabled** = 50% opacity, `cursor: not-allowed`, and activation does
  nothing. A control disabled **by circumstance** (there is nothing to
  act on, a required field is blank) stays **focusable** —
  `aria-disabled="true"` rather than the `disabled` attribute — so a
  keyboard reaches it and hears the reason its `aria-describedby` names.
  The `disabled` attribute proper is for the moment an action is in
  flight, when there is nothing to explain. Disabled text is the one
  exemption from the AA floor, as an inactive component; nothing else
  is.
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
     control per available transition, with `ready` as the only primary
     button when it is offered. After a successful POST the displayed
     status and buttons match the reloaded card.
  10. On the top page the content column's first element child is the
      control panel and the task list follows it. Each carries its own
      `data-state`; forcing the list request to fail leaves the panel at
      `success` with working buttons, and forcing the control request to
      fail leaves the list rendering its groups.
  11. With at least one mergeable task, the page region contains exactly
      one accent-filled control and it is the merge button: its computed
      background equals the accent (`rgb(94, 184, 199)` in Sumi,
      `rgb(47, 111, 126)` in Kinari). Its note is a pill whose text is
      the mergeable count and whose computed `border-radius` is 9999px.
  12. With nothing mergeable, the merge button's computed background
      equals surface-raised rather than the accent, `aria-disabled` is
      `"true"`, `opacity` computes to 0.5, `cursor` is `not-allowed`,
      Tab still lands on it and shows the 2px accent focus ring, the
      element its `aria-describedby` names is visible with non-empty
      text, and pressing it fires no request and changes no count.
  13. Pressing an enabled merge sends exactly one POST per mergeable
      task; afterwards the mergeable count has dropped to zero, the
      pending-merge card list has grown by that same number, and the
      result line (`aria-live="polite"`) is non-empty. At no point does
      a card for an `instant:merge` task appear inside any status group.
  14. The release modal opens centered per the modal geometry with the
      tag input holding focus; its confirm button is `aria-disabled`
      while the tag is blank or whitespace and drops the attribute once
      a non-blank tag is typed. With several releasable products a
      `role="radiogroup"` is present, its first row selected with an
      accent-subtle background; with exactly one there is no radiogroup
      and the product id appears as a caption. The open modal contains
      exactly one accent-filled control. ×, Esc, and the scrim each
      close it and return focus to the release button.
  15. A refused release leaves the modal open with the typed tag still
      in the field and the server's message rendered in the error-banner
      colors (danger text on danger-subtle) inside the modal. A
      successful one closes the modal, leaves a non-empty result line on
      the panel, and the released tasks are gone from the `merged` group
      on reload.
  16. Status groups exist in the DOM only when non-empty; their document
      order is draft, ready, wip, done, merged, blocked, cancelled,
      dropped; no group for `released` ever exists; each heading's count
      pill number equals the number of cards under it; `9999px` radius
      is computed only on count pills and status badges.
  17. At 375px the panel's buttons and notes wrap without
      `document.documentElement.scrollWidth` exceeding the viewport,
      every control's hit box is at least 36px tall, and no control row
      overlaps another (bounding boxes disjoint). At 900px the content
      column computes to 720px wide with 12px left and right padding and
      equal left/right margins (±1px), and the panel — its child —
      computes to 696px.
  18. Every install asset is really served, not swallowed by the SPA
      fallback: the static server answers unknown paths with
      `index.html`, so a missing file still returns 200. `GET
      /manifest.webmanifest` therefore has to answer
      `content-type: application/manifest+json` with a body that
      `JSON.parse`s, and each icon path has to answer `image/png`
      (`image/svg+xml` for `/icon.svg`). A `text/html` content type on
      any of them is a failure even at status 200.
  19. In the loaded document there is exactly one
      `link[rel="manifest"]`, one `link[rel="icon"]`, and one
      `link[rel="apple-touch-icon"]`, each resolving to a URL that
      passes invariant 18. Over the real `https://<host>:8443/` origin
      the browser's manifest inspector reports no manifest or icon
      error and shows `name` `Task Server`, `short_name` `Task`,
      `display` `standalone`, `theme_color` and `background_color`
      `#191919`, and `id` / `start_url` / `scope` all resolving to the
      origin root with the non-default port intact — the port never
      appears as a scope mismatch. Switching to ライト changes none of
      those manifest values.
  20. Measured on the files themselves: `icon-192.png` is 192×192,
      `icon-512.png` and `icon-maskable-512.png` are 512×512,
      `apple-touch-icon.png` is 180×180. The maskable and apple-touch
      rasters have alpha 255 at all four corner pixels; the `any`
      rasters have alpha 0 there. No accent-ink pixel of
      `icon-maskable-512.png` lies farther than 0.4 × 512 from the
      center, so a circular launcher mask crops ground only. Re-running
      the documented render commands reproduces byte-identical files.
  21. The mark survives launcher scale: downscale
      `icon-maskable-512.png` to 48×48 and clip it to a centered circle
      of 80% diameter — the accent-ink pixel count is unchanged by the
      clip (no ink lost) and the ink bounding box is at least 28px wide
      of the 48, so what remains is the whole check rather than a
      fragment.

## Do's and Don'ts

- Do source every color from a `--c-*` variable; don't hardcode hex in
  components.
- Do consume `--c-wash-*` / `--c-hover-*` for bands and hovers; don't
  reach for `accent-subtle` directly in those jobs.
- Do keep exactly one accent-filled primary action per region — one on
  the page, one inside an open modal; don't dim an accent fill to say
  "disabled", drop it to the default treatment instead.
- Do name in text, beside the control, why a disabled control is
  disabled, and point at that text with `aria-describedby`; don't ship a
  control whose only signal is 50% opacity, and don't put an
  unexplainable control out of the keyboard's reach.
- Do report the outcome of a control action in place and reload the data
  behind it; don't add a toast, a timer, or a message that dismisses
  itself.
- Do drop a status group entirely when it holds nothing; don't render a
  heading with a zero pill under it.
- Do present the menu as a hamburger-anchored dropdown; centered
  modals are for dialogs (theme settings), never for navigation.
- Don't use emoji or text glyphs as icons; every UI icon is an
  `Icon.svelte` dictionary entry.
- Do render every app-icon raster from `client/public/icon.svg` and track
  the output; don't hand-edit a PNG, and don't let the app icon and the
  `Icon.svelte` dictionary borrow each other's artwork.
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
