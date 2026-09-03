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
The top page shows the stretch of pipeline the server carries by itself —
review, then merge, then release — and the open work grouped by status.
Review, merge and release are all issued automatically (a release is issued
the moment work lands, at the level the work was filed with), so **the top
page asks a human for no decision and holds no primary button**. The one
decision this UI still asks — promoting a draft to `ready` — lives on the
Task Card.

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
   three, left to right: the app title as a home link (`<a href="/">`,
   label type, on-surface ink, no underline), the closed link
   (`<a href="/closed">`, label type, same ink, grouped beside the title
   with `--sp-2` gap), and the hamburger icon-button (right, unchanged).
   **The title and the closed link are the header's only navigation
   links**; every other destination lives inside the menu, so phone
   widths never crowd. The closed link is a plain page-navigation link,
   not a primary action — it never takes the accent-filled button
   treatment, so it never competes with a page's one accent-filled
   control (see Colors, Buttons). Its only states are default and
   selected (Components); it never disables.
2. **Sub-header — detail screens only.** 40px, `--c-wash-raised`, 1px
   bottom hairline, holding only the current item's title (label,
   single line, ellipsized). No back button — going back is the header
   title link or the browser itself.
3. **Main content**, the only scrolling region.

**Screen-level controls are content, not chrome.** A screen that offers
actions on the whole screen's subject places them as the **first block
inside the content column** (the top page's panel sits there even though
it now holds readouts and no control). It is never a third band, never
sticky, and never full-width: the two bands above stay the only bands, and
main content stays the only scrolling region, so the controls scroll away
with the work they act on.

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
- **full:** count pills, the status badge, and the spinner only.

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
  Japanese UI already uses untranslated ("review が発行されていない task"),
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
  links). Beside it, the **closed link** is the header's one
  page-navigation link: label type, `var(--sp-1) var(--sp-2)` padding,
  `--radius-sm`, inline-flex, 36px min height (the same hit target as
  the hamburger). Default state matches the title — on-surface ink,
  transparent background, hover fills `--c-hover-1`. On `/closed` it
  carries `aria-current="page"` plus the **selected-radio treatment
  already used in the release modal** (`--c-accent-subtle` background,
  `--radius-sm`) and switches its text to `--c-accent` ink — the tint
  this document already names "the selected-state tint" in Colors,
  never a solid accent fill, because the header is chrome present on
  every page and a nav link is not the primary action a page's one
  accent fill marks. The hamburger stays a 36px quiet icon-button with
  `aria-label` and `aria-expanded`, unchanged.
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
  screen: one panel over the open work grouped by status. It reads two
  sources — the control plane and the task list — and **each region
  carries its own `data-state`**, so a failure or an emptiness on one
  side never masks the other.

  **What the panel is.** Review, merge and release are all issued by the
  server, so the panel is not "the screen's controls" and holds none. It
  is **the automated stretch of the pipeline**: what the machine is
  carrying — reviews waiting, the merge trains, the releases — and
  anything the automation failed to carry. The task list below it is the
  work still waiting for a human or a worker to pick up.

  The panel is the first block of the content column, built from the card
  recipe (surface-raised, 1px hairline, md radius, 10px padding), `--sp-3`
  between its blocks, and it is read top-down in pipeline order.

  **The top page has no primary button.** The earlier rule — the page's
  one accent marks the human decision point — has nothing left to mark:
  the release button died the way the merge button did, when the server
  took the decision over. A release is issued at the landing, at the
  `release_level` the work was filed with, and a worker cuts the tag. So
  no `<button>` on the top page is accent-filled, and no control on it is
  named for release or merge. Where a human decision still exists — the
  `ready` transition on a Task Card — the accent stays with it. A move
  the machine makes never takes the accent.

  **There is no toast, no timer, and no auto-dismissing message anywhere
  in this product.** The panel reports no action either, because it makes
  none; the reloaded readouts are the receipt for whatever the server did.

  **Readouts, and when they exist.** The panel draws what the server is
  carrying, in pipeline order: the review queue, the merge trains, the
  releases, then reconciliation. Each is a muted caption with its count
  pill beside it over the ordinary card list, exactly like a status-group
  heading, and **a readout holding nothing is not rendered at all**,
  caption included — the status-group rule, for the same reason. When
  every readout is empty the panel says so in one muted line instead: a
  quiet pipeline is a state worth naming, not an empty box.

  A readout has no status heading over it, so **its cards wear the
  neutral outline status badge** beside the product id — the same badge
  a status-group card wears as its last line, so a card reads the same
  wherever it sits.

  **Review queue.** The pending `review` tasks, under the caption "review
  待ち". This is normal running: in a healthy pipeline the queue is
  usually non-empty and it means nothing is wrong. It is drawn here
  because a review waiting for a reviewer is the machine's stretch of the
  pipeline, not the operator's work.

  **Merge trains.** The outstanding merge tasks, **grouped by product**,
  because the train is per product — one product's jam never holds
  another's, and a single flat list would say the opposite. Each group is
  captioned by its product id with its count pill. **The panel writes no
  caption that duplicates a status** — every card wears its status badge
  like every other readout card — so the old "merge 進行中" caption is
  gone. It was true only while nothing was blocked.

  **Position says state, not sequence.** The server no longer promises
  which of a product's outstanding merges is handed out next: any merge
  that is `ready` may be the one a claim takes, and the decision is not
  made until a worker asks. So a group is drawn **the merges that are
  holding it up first — the ones that are `wip` or `blocked` — then the
  rest**. What is left is a **set, not a line**: its order is stable, so
  the list does not shuffle under the eye between reloads, and that
  stability is a property of the drawing, never a claim about
  distribution. Nothing in a train may say otherwise: **no ordinals, no
  "次", no arrow, no card marked as coming first**. The old head — "the
  first card is the merge that is or will be claimed" — is retired with
  the ordering it read off, and the word "head" with it. A screen that
  keeps a promise the server dropped is worse than one that never made
  it.

  **The holder** is the merge that is stopping the group: at most one per
  product, `wip` while it runs, `blocked` when it stopped and is waiting
  for a human. A holder in `wip` is an ordinary running train and says
  nothing extra — the badge already says it. **A holder in `blocked` is
  the one thing that must not read as mere slowness.** Its card carries,
  under the title, **the reason as body-sm text** with `white-space:
  pre-line` — on that card, wherever the card sits, because the reason
  belongs to the merge that has it and not to a position. The group adds
  **one muted caption naming how many of that product's merges are
  waiting on it** ("他 2 件が待機中"), rendered only when at least one
  other merge is there to wait. A named cause and a named cost are what
  separate a stuck train from a quiet one. It stays **neutral**: a rebase
  conflict or a failing check is an ordinary outcome of landing work, not
  a failure of the app — exactly as a review's `request_changes` is — so
  the danger tokens do not enter here.

  **Releases.** The outstanding `instant:release` tasks, under the caption
  "release": at most one per product, issued by the landing and finished
  by the tag a worker cut. Drawn like a train's cards — the product id as
  the name, the **level** (`patch` / `minor` / `major`) and the status as
  outline badges, and on a `blocked` release its reason under the title
  as body-sm text with `white-space: pre-line`, neutral like a jammed
  merge. A stopped release holds every later landing of its product back
  the way a jammed merge holds its train, which is why it earns the same
  legibility: a named cause on the card that has it. No control sits
  here; calling a stopped release off happens on its Task Card, and the
  next one is issued by hand over the API.

  **Reconciliation.** The work the automation should be carrying and is
  not: tasks that are `done` with no live review, tasks that are
  `approved` with no live merge, and products whose landed work has no
  live release. Every set is empty in a healthy pipeline, which is what
  makes them a different kind of thing from the queues above and forbids
  giving them the same look. This is the panel's
  **one danger-framed block**: the error-banner recipe (danger text on
  `danger-subtle`, sm radius, 8px padding, body-sm) carrying
  `role="status"` — a standing state the operator has to notice, not the
  outcome of a request they just made, so never `role="alert"`. It holds
  one captioned line with its count pill per non-empty set — "review が
  発行されていない task" / "merge が発行されていない task" / "release が
  発行されていない product" — and under each the ordinary card list (for
  the release set, one row per product carrying the count of tasks it
  would ship), whose cards keep the neutral card recipe:
  **the danger tint frames the fact that the pipeline is holding work,
  and never tints the tasks themselves**, which are ordinary work. This
  is the single extension of the danger tokens past "a request failed",
  and it is earned: the automation dropping work silently is the only
  other thing in this product that must never go unread.

  **Stuck, inside reconciliation.** The server also measures waiting:
  `GET /api/control` carries `stuck`, one row per task that has sat past
  a threshold (a task never appears under two reasons), each with a fixed `reason` (`unclaimed`, `lease-expired`,
  `no-subtask`, `subtask-unclaimed`, `blocked`, `release-stalled`). The
  block renders it as its last readout under the caption "動いていない
  task" with the count pill, one ordinary card per row linking to the
  task, wearing the reason and the status as outline badges and the
  `since` timestamp as the muted tail. The judgment is the server's
  (a clock and a threshold), never the screen's: the readout states the
  rows, sorts nothing, holds no button, and follows the same rule as
  `releasable` — issued by nobody, pressed by nobody. It is absent from
  the DOM while `stuck` is empty.

  Reconciliation is also where the **cancelled blocked merge** surfaces.
  Cancelling a blocked merge frees the rest of that product's train, but
  its target is not re-issued — it falls back to `approved` with no merge
  and appears here until a human moves it. **The panel states that as
  state, never as instruction**: no sentence on this screen tells the
  operator what to do next. Procedure belongs to the README; the screen's
  only duty is that the stranded task cannot be missed.

  **Panel states.** The panel exposes
  `data-state="loading|empty|error|success"` on the same discipline as
  the list: _loading_ is the centered spinner line; _error_ is the danger
  body-sm line plus a default retry button — the one button the panel can
  ever hold; _empty_ — nothing pending, nothing stranded — is one muted
  line saying so ("運んでいるものはありません"); _success_ is the panel
  above.

  **Task list.** Below the panel, the open tasks grouped by status.
  Groups follow the status vocabulary order — the main line `draft`,
  `ready`, `wip`, `done`, `approved`, `merged`, then the sidetrack
  `blocked` — and **a group holding nothing is not rendered at all**,
  heading included. `cancelled` and `dropped` are never grouped: a task
  that was called off leaves this page the way a released one does, and
  the closed page is where a cancelled task keeps being readable. The order is the pipeline
  read from its start, so `approved` sits between `done` and `merged`:
  that is where the work stands — finished, carried past review, not
  yet landed. `released`, `cancelled` and `dropped` are never shown —
  shipped and called-off work leaves this page.

  **What the list hides, and why.** The list shows `normal` tasks only,
  and hides two things. A `normal` task hides while another region of
  this same page already renders it — the panel draws stranded tasks in
  reconciliation — because one object drawn twice on one screen reads as
  two; it falls back into its status group the moment the panel stops
  drawing it. A task whose `kind` is not `normal` hides always, whatever
  its status: a `review`, `instant:merge` or `rework` task exists on this
  page only through the panel's own readout — the review queue or the
  merge trains — while it is in flight, and once it finishes it leaves the
  page rather than falling into a status group. A `rework` is the pass a
  verdict or a conflicted merge sent the work back for; its target stays
  in the `wip` group meanwhile, which is where that work is. A review's verdict already lives on
  its target's `latest_review`, and a landed merge's target already
  carries the fact, so a finished subtask left behind here would be a
  husk telling nothing the target's own card does not already tell. The
  rule decides this; taste does not re-open it.

  Each group is a section carrying its status as a data attribute,
  headed by a label-type heading naming the status with its count pill
  beside it; under the heading, cards per the family recipe
  (surface-raised, 1px hairline, 8px radius, 10px padding) in a single
  column with 8px gaps. A card links to its Task Card and **reads from
  the forest to the tree to its state, top to bottom, in three lines**:
  first the product id in body-sm on-surface text (not the muted
  caption — it is the first thing read, not an afterthought), then the
  title (label), wrapping onto further lines rather than being clipped
  to one, then the neutral outline status badge. The heading groups;
  the badge lets a single card be read on its own, and it keeps every
  card on the page — group or readout — in one shape. A card whose
  kind is not `normal` would add the kind badge after the status
  badge, but every card here is a `normal` task, so in practice none
  does. The card is one link and contains no other focusable element.
  The list container keeps `data-state="loading|empty|error|success"`:
  - _loading:_ centered muted body-sm text with the accent spinner
    (1.5px-stroke circle, 1.1rem);
  - _empty:_ centered muted body-sm message;
  - _error:_ danger-colored body-sm message plus a default retry button;
  - _success:_ the groups.

- **Detail page — Task Card:** sub-header (title only) over a content
  column whose head reads in the same order as a list card: **the
  product id first** (body-sm, on-surface), then a row of outline badges
  (caption type, 1px border, muted text; neutral chrome, not a data
  color) — the status, and for a task whose kind is not `normal`
  (`instant:merge`, `review`, `rework`) a second badge naming that kind. Below
  the head, `commit_sha` and `verification` as muted captions when
  present, body text (body, 1.6, pre-line), and `available_transitions`
  as a row of default buttons, one per reachable status. The `ready` transition, when present,
  is the single primary (accent-filled) button — it is the human
  decision the screen exists for. After a successful transition the
  card reloads so status and buttons update. There is no
  icon-dictionary fixture page.

  **Status is worn, never tinted.** Every status reads through the same
  neutral outline badge and its group heading; no status earns a color,
  an icon, or a weight of its own. `approved` is told apart from `done`
  by where it sits in the vocabulary order — past review, before the
  landing — and that position is the whole of its signal. `approved` is
  also a status this screen never *produces*: a review report alone
  enters it, so no control anywhere in this UI is labelled `approved`.

  **Dependency.** A task that waits for another (`depends_on`) carries
  one more muted caption in the caption row, labelled `depends_on`, whose
  value is a link to that task; while the dependency has not landed the
  caption adds the dependency's status as an outline badge, and once it
  has landed (or when there is no dependency) the badge — and, with no
  dependency, the whole caption — is absent. That is the whole reason a
  draft is still a draft, so it is said where the other facts about the
  task are said, in the same voice: no color, no icon, no instruction.
  On the top page a status-group card of a waiting task shows the
  dependency's id as a second muted caption beside the product id,
  marked `←`; nothing else on the list changes, and no control is added
  anywhere for it — the landing promotes the task, and a person who
  wants to skip the order clears the dependency.

  **Review block.** A task the card payload carries a review outcome
  for (`review_verdict` and `review_findings`, however the server
  sources them) renders one block **between the caption row and the
  body**: a muted caption heading レビュー carrying the verdict as an
  outline badge, then the findings as body-sm text with `white-space:
  pre-line`, the whole block built from the card recipe
  (surface-raised, 1px hairline, md radius, 10px padding). It sits
  above the body because a worker reopening a task that came back to
  `ready` has to read the correction before the instruction it
  corrects; a reader who has to scroll past the brief to find out why
  it reappeared has been told too late. It stays **neutral**:
  `request_changes` is an ordinary outcome of review, not a failure of
  the app, so the danger tokens stay reserved for requests that failed.
  A task with no review outcome renders no block — never an empty one.
  A `review` task's own card adds its subject commit as a muted caption
  in the caption row, so the operator can see which commit the verdict
  was passed on.
- **Closed page — finished and called-off task list.** Reached from the
  header's closed link (`/closed`; `/done` is the old address and is
  rewritten to `/closed` in place); no sub-header, per Layout item 2 —
  like the top page, the content column opens directly on the list and
  current location is carried by the header's selected state alone, with
  no page heading of its own. It lists every `kind: normal` task whose
  status is `done`, `approved`, `merged`, `released` or `cancelled` —
  non-`normal` tasks (`review`, `instant:merge`, `rework`) never appear,
  and `dropped` never appears anywhere: it is the status of a subtask
  folded for a rebuild, whose target keeps the history. (An operator may
  also drop a `normal` task by hand; it then sits on no page until the
  retention sweep deletes it, reachable only by its URL — `cancelled` is
  the word for calling off normal work, and it stays readable here.) `released` and `cancelled`
  are shown here on purpose: the top page drops a task the moment it
  ships or is called off, and this one list is where closed work keeps
  being readable, told apart by the status badge alone. Rows sort by the
  moment they closed (`closed_at`: completion for finished work, the
  cancelling for called-off work), most recent first.

  **Deleting is not a screen.** A closed task can be deleted
  (`DELETE /api/tasks/{id}`, the MCP `task_delete`, or the retention
  sweep), but no page holds a delete control: the top page issues
  nothing, and the closed page and the Task Card state what is, in the
  same voice as everything else. Deleting is an operator's act over the
  API, like issuing a release by hand.

  Each row reuses the **card recipe** (`.cards` / `.card`:
  surface-raised, 1px hairline, `--radius-md`, 10px padding, 8px gaps
  between rows), stacked exactly like a blocked merge-train card — a
  `.name`/`.tail` header line, then one optional line under it — so the
  whole row is **one link to the Task Card and never a second focus
  stop**. The header line holds the title (`.name`) on the left; the
  tail (right, baseline-aligned) holds, in order, the **status badge**
  (outline badge recipe — neutral, never tinted, per Status is worn,
  never tinted), the **release tag** when present (the same outline
  badge recipe, a second chip), the `.product` id, then the completion
  timestamp as a muted caption. Under the header line, when
  `verification` is non-empty, an excerpt line renders its **first one
  or two source lines** (split on `\n`, not CSS-clamped) as muted
  body-sm text with `white-space: pre-line` and `overflow-wrap: anywhere`
  — a task with empty or absent verification renders no excerpt line,
  never an empty one, the same rule the review block already keeps.

  The list container carries `data-state="loading|empty|error|success"`
  on the task list's own discipline: _loading_ is the centered accent
  spinner line; _empty_ is one centered muted `.state` line reading
  "閉じたタスクがありません" and nothing else — no heading, no zero
  pill; _error_ is the `.state.error` line plus a default "再試行"
  button; _success_ is the rows. There is no group heading and no status
  grouping here — unlike the top page's task list, the closed page is
  already filtered to one purpose and reads faster as one flat,
  time-ordered list than as separate single-status groups.
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
  3. At 375px the header contains exactly three interactive elements —
     the title `<a href="/">`, the closed link `<a href="/closed">`, and
     the hamburger `<button>` — and
     `document.documentElement.scrollWidth` never exceeds the
     viewport, with the menu closed or open, on both `/` and `/closed`.
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
     button when it is offered. Its head opens with the product id: in
     document order the product precedes the status badge, and the kind
     badge (when the kind is not `normal`) follows the status badge.
     After a successful POST the displayed status and buttons match the
     reloaded card.
  10. On the top page the content column's first element child is the
      control panel and the task list follows it. Each carries its own
      `data-state`; forcing the list request to fail leaves the panel at
      `success` with its readouts drawn, and forcing the control request to
      fail leaves the list rendering its groups.
  11. The top page has no primary button. With the control plane carrying
      pending reviews, merges and releases and reporting stranded work of
      every kind, the page region contains no `<button>` whose computed
      background equals the accent (`rgb(94, 184, 199)` in Sumi,
      `rgb(47, 111, 126)` in Kinari), and outside the panel's error state
      the page contains no `<button>` at all. No `<dialog>` or
      `role="dialog"` element exists on the top page.
  12. The top page issues nothing. No control on the page is named for
      release or merge; loading the page against a control plane that
      reports mergeable, unreviewed and releasable work sends no request
      with a method other than GET.
  13. With at least one pending release the panel renders the releases
      readout: a muted caption whose count pill number equals the number
      of cards under it, each card linking to its `instant:release` task,
      naming its product id, and wearing two outline badges — the level
      (`patch`, `minor` or `major`) and the status. A `blocked` release
      carries its reason as body-sm text preserving newlines
      (`white-space` is `pre-line`) on its own card; a `ready` or `wip`
      one renders no reason element. No text color or background in the
      readout computes to the danger pair. With no pending release the
      readout is absent from the DOM entirely.
  14. With a product in `releasable`, the reconciliation block renders one
      row per stranded product carrying its id and a count pill equal to
      the number of tasks it would ship, under the caption "release が
      発行されていない product", with no button in the row. With
      `releasable` empty the row set is absent.
  15. With nothing pending and nothing stranded the panel's `data-state`
      is `empty` and it renders exactly one muted body-sm line and no
      button; with anything carried it is `success`.
  16. Status groups exist in the DOM only when non-empty; their document
      order is draft, ready, wip, done, approved, merged, blocked,
      cancelled, dropped; no group for `released` ever exists; each
      heading's count pill number equals the number of cards under it;
      `9999px` radius is computed only on count pills, outline badges,
      and the spinner.
  17. At 375px the panel's captions and readouts wrap without
      `document.documentElement.scrollWidth` exceeding the viewport,
      every card's hit box is at least 36px tall, and no two blocks of
      the panel — review queue, merge trains, releases, reconciliation —
      overlap (bounding boxes disjoint). At 900px the
      content column computes to 720px wide with 12px left and right
      padding and equal left/right margins (±1px), and the panel — its
      child — computes to 696px.
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
  22. Non-`normal` work never has a home in a status group, only in the
      panel's own readout. While a `review` task is pending, a card for
      it exists in the review queue and in no status group; once it is no
      longer pending it leaves the top page entirely — no status group,
      no card, whatever its status. No status group ever contains a card
      for an `instant:merge` task either, pending or finished. A status
      group card reads product, then title, then state: in document
      order its product id precedes its title and its status badge
      follows the title, and the status badge is the card's only badge,
      because every card in a status group is a `normal` task. The
      product id is not clipped, and a title longer than the card wraps
      (the card's height grows; `document.documentElement.scrollWidth`
      stays within a 375px viewport). Tabbing through any card — in a
      status group, the review queue, a merge train, or the
      reconciliation block — reaches exactly one focusable element, the
      card link itself.
  23. `approved` is a status the UI reads and never writes: on a `done`
      task's Task Card no transition button's text is `approved`, and
      across the top page and any open modal no control's text is
      `approved`. An `approved` group renders like every other group —
      neutral heading, count pill, and no color, icon, or weight telling
      it apart from `done`.
  24. A task carrying a review outcome renders the review block on its
      Task Card, and the block precedes the body element in document
      order (`compareDocumentPosition`). The block computes the card
      geometry (1px border, 8px radius, 10px padding); its text colors
      are the muted and on-surface tokens and never the danger pair; its
      findings preserve newlines (`white-space` is `pre-line`). A task
      with no review outcome renders no such block, empty or otherwise.
      At 375px, findings holding a 40-character unbroken token still
      leave `document.documentElement.scrollWidth` within the viewport.
      A `review` task's card shows its subject commit as a muted
      caption.
  25. Readouts exist only when they hold something, controls always.
      With at least one pending `review` task the panel renders the
      review queue: a muted caption whose count pill number equals the
      number of cards under it, each card linking to its review task and
      wearing the neutral outline status badge that a status-group card
      does not. With no pending review the queue is absent from the DOM
      entirely — no caption, no empty message.
  26. Merge trains are per product and a jam is legible. With merges
      outstanding on two products the panel renders one group per
      product, each captioned by its product id with a count pill equal
      to its cards. A `blocked` merge carries the neutral outline status
      badge and its reason as body-sm text preserving newlines
      (`white-space` is `pre-line`) on its own card; no other card in
      that group renders a reason element. When the group holds at least
      one `ready` merge besides it, the group also renders exactly one
      muted caption naming how many wait, whose number equals the count
      of `ready` cards in that group; with none the caption is absent. A
      group whose merges are all `ready`, and one whose holder is `wip`,
      render no reason and no waiting caption. Blocking one product's
      merge leaves the other product's group unchanged. No text color or
      background inside a train computes to the danger pair, and no
      caption in a train has a status name as its text. At 375px a reason
      holding a 40-character unbroken token still leaves
      `document.documentElement.scrollWidth` within the viewport.
  27. A train's order says state and promises no sequence. In every
      product group, each card whose status is `wip` or `blocked`
      precedes every card that is neither
      (`compareDocumentPosition`). No card in a train carries an
      ordinal, a position number, an arrow, or the text 次: within a
      group the only numeral rendered outside a card's own title and
      status badge is the caption's count pill. Loading the page twice
      against unchanged server state yields the same card order in
      every group, and moving a group's `blocked` merge to `cancelled`
      leaves the remaining cards in the order they already had.
  28. Reconciliation is framed as the anomaly it is. With nothing
      stranded the block is absent from the DOM. With a `done` task
      holding no live review, or an `approved` task holding no live
      merge, it renders once, computes danger text on a `danger-subtle`
      background, carries `role="status"` and never `role="alert"`, and
      holds one captioned line per non-empty set whose count pill equals
      the cards under it; those cards compute the ordinary card recipe
      (surface-raised background, 1px border, 8px radius) and appear in
      no status group. Cancelling a `blocked` merge and reloading leaves
      that merge out of every train and its target inside this block.
  29. A Task Card whose task carries `depends_on` renders a caption
      labelled `depends_on` whose link resolves to that task's card, and
      while the card payload carries `dependency_status` the caption holds
      an outline badge with that status; without `dependency_status` no
      badge exists, and without `depends_on` no such caption exists. On
      the top page a waiting task's status-group card carries the
      dependency id as a muted caption beside the product id, and a task
      without a dependency carries none. No button anywhere is added for
      dependencies.
  30. Header closed link states, computed. Off `/closed`, the closed
      link's computed `background-color` is transparent and it carries no
      `aria-current` attribute. On `/closed`, it carries
      `aria-current="page"`, its computed `background-color` equals
      `--c-accent-subtle` (`rgba(94, 184, 199, 0.15)` in Sumi,
      `rgba(47, 111, 126, 0.1)` in Kinari), its computed `color` equals
      `--c-accent`, and its computed `border-radius` equals 6px. On no
      page does the header itself contain a solid-accent-background
      element — the closed link never computes a solid `--c-accent`
      background in either state, so it never becomes a second
      accent-filled control alongside a page's own primary button.
      `:focus-visible` on the closed link shows the same 2px accent
      outline as every other control, and clicking it or activating it
      with Enter navigates to `/closed` without a full page reload.
      Loading `/done` lands on the closed page with the address rewritten
      to `/closed` and no extra history entry.
  31. The closed page shows only closed work, correctly typed. Every
      row links to a task whose `kind` is `normal`; no `review`,
      `instant:merge` or `rework` task ever appears. Every row's status is
      one of `done`, `approved`, `merged`, `released`, `cancelled`; no
      other status appears, and `dropped` appears nowhere. Loading the
      page twice against unchanged server state yields rows in the same
      order, and that order is non-increasing by `closed_at` (each row's
      timestamp is at or after the next row's). The top page's status
      groups hold no `cancelled` or `dropped` task.
  32. Closed row composition and single focus stop. Each row computes the
      card recipe (1px border, 8px radius, 10px padding) and contains
      exactly one focusable element — the row's own link — regardless
      of whether a release-tag chip or a verification excerpt is
      present. The status badge and any release-tag chip both compute
      `border-radius` 9999px and sit in the row's tail alongside the
      product id and the completion timestamp. A row whose task has
      empty or absent `verification` renders no excerpt element at all
      — not an empty one — and a row whose `verification` holds more
      than two lines shows only its first two, joined by a single
      `\n`, in the rendered `textContent`.
  33. Closed page states. With no closed tasks the list renders
      exactly one `.state` element reading "閉じたタスクがありません"
      and no row, no heading, and no pill. While loading it renders the
      centered accent spinner and no rows. On a failed fetch it renders
      the `.state.error` line plus a default-styled "再試行" button
      that re-fires the request on click. At 375px, a row whose
      verification excerpt holds a 40-character unbroken token still
      leaves `document.documentElement.scrollWidth` within the
      viewport.
  34. Stuck work is stated, not handled. With `stuck` non-empty the
      reconciliation block renders a readout captioned "動いていない task"
      whose count pill equals its rows; each row is an ordinary card link
      to `/tasks/<task_id>` carrying `data-reason` and two outline badges
      (reason, status), and the block holds no `<button>`. With `stuck`
      empty the readout is absent from the DOM.

## Do's and Don'ts

- Do source every color from a `--c-*` variable; don't hardcode hex in
  components.
- Do consume `--c-wash-*` / `--c-hover-*` for bands and hovers; don't
  reach for `accent-subtle` directly in those jobs.
- Do keep exactly one accent-filled primary action per region — one on
  the page, one inside an open modal; don't dim an accent fill to say
  "disabled", drop it to the default treatment instead.
- Do let the page's one accent mark the point where the pipeline stops and
  asks a human; don't hand it to whichever move happens most often, and
  don't leave it on a control the operator no longer presses.
- Do render a readout only while it holds something, and a control always,
  with its reason when it can do nothing; don't ship an empty readout with
  a zero pill, and don't let a control disappear when it is idle.
- Do reserve the danger tokens for a request that failed and for the
  pipeline holding work the automation should have carried; don't tint a
  `blocked` merge or a `request_changes` review — those are ordinary
  outcomes of the work.
- Do show a stalled merge train's cause and what it is holding — the
  reason on the stopped merge's own card and the count of the merges
  waiting on it; don't let a jam read as ordinary slowness.
- Do let a train's order say which merge is holding it up and nothing
  more, drawing the holder first; don't number the cards, mark one as
  next, or otherwise promise an order the server stopped keeping.
- Do let the panel say what is true; don't put procedure on the screen —
  what an operator should do about a stranded task belongs in the README.
- Do name in text, beside the control, why a disabled control is
  disabled, and point at that text with `aria-describedby`; don't ship a
  control whose only signal is 50% opacity, and don't put an
  unexplainable control out of the keyboard's reach.
- Do report the outcome of a control action in place and reload the data
  behind it; don't add a toast, a timer, or a message that dismisses
  itself.
- Do let a background reload (tab-visible return, the recurring interval)
  swap in fresh data only on success; don't let it clear a drawn card or
  list first, close an open modal, or reset a form or focus while it runs.
- Do drop a status group entirely when it holds nothing; don't render a
  heading with a zero pill under it.
- Do let a status say what it means through the vocabulary order and the
  neutral outline badge; don't give one status a color, an icon, or a
  weight the others don't have.
- Do keep a task in its status group whenever it is waiting for someone
  to claim it; hide it only when another region of the same page already
  draws it.
- Do show a review's findings on the reviewed task's own card, above the
  body; don't make the worker who was sent back navigate elsewhere to
  learn why.
- Do present the menu as a hamburger-anchored dropdown; centered
  modals are for dialogs (theme settings), never for navigation.
- Do give the closed link the selected-radio tint (`--c-accent-subtle`
  background, `--c-accent` text) and `aria-current="page"` when active;
  don't give a page-navigation link a solid accent-filled background —
  that treatment stays reserved for a region's one primary action.
- Do keep a done row a single link with one focus stop, its status and
  release tag riding the existing outline badge recipe; don't add a
  second link or button inside a row for the tag or the status.
- Do drop a done row's verification excerpt entirely when there is
  nothing to show; don't render an empty excerpt line.
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
