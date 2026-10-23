# Design System

<!-- impeccable:design-schema 1 -->

## Provenance

Extracted from [canonical/lxd-ui](https://github.com/canonical/lxd-ui), whose visual
identity is Canonical's **Vanilla Framework** (v4.56.0) plus the Canonical design
tokens. This project adopts Vanilla directly: the tokens below are Vanilla's
`_settings_*.scss` source-of-truth values, re-expressed so future work reads them
without digging into `node_modules`.

<!--
THESIS: The Canonical ops console — a high-contrast, square-cornered, border-led
interface that reads as an instrument, not a brochure. It refuses the soft-shadowed
rounded card default of dashboard UI.

OWN-WORLD: Ubuntu orange as the single brand accent on a dense neutral ground.
Ubuntu (variable) for interface and headings, Ubuntu Mono for code and values.
8px spacing baseline, 1px hairline borders, near-zero corner radius, and a flat
shadow language that only deepens for overlays. Components are `.p-*` Vanilla
patterns wrapped as typed Svelte 5 components.

LIGHT/DARK: Light is the default (`.is-light` on `<html>`); `.is-dark` and
`.is-paper` are theme aliases that swap the palette through CSS custom properties.
-->

## Visual character

- **Corners:** square (radius 0). Depth comes from borders and a two-tier shadow,
  never from rounding.
- **Contrast:** near-black text on white; muted text is black at 0.6 alpha, never
  mid-gray-by-default in light mode.
- **Borders:** 1px hairlines are the primary separation device. Default border is
  `rgba(0,0,0,0.2)`; a low-contrast variant (`rgba(0,0,0,0.1)`) separates interior
  rows; high-contrast (`#707070`) marks interactive controls.
- **Brand:** Ubuntu orange is reserved for the single primary action and product
  mark, never scattered as decoration.

## Color tokens

### Primitives

| Token | Value |
| --- | --- |
| `$color-ubuntu` (brand) | `#e95420` |
| `$color-accent` | `#0f95a1` (dark: `#70bbc2`) |
| `$color-link` | `#06c` |
| `$color-link-visited` | `#7d42b8` |
| `$color-focus` | `#2e96ff` |
| `$color-negative` | `#c7162b` |
| `$color-caution` | `#cc7900` |
| `$color-positive` | `#0e8420` |
| `$color-information` | `#24598f` |
| `$color-x-light` | `#fff` |
| `$color-light` | `#f7f7f7` |
| `$color-mid-x-light` | `#e5e5e5` |
| `$color-mid-light` | `#d9d9d9` |
| `$color-mid` | `#999` |
| `$color-mid-dark` | `#666` |
| `$color-dark` | `#111` |
| `$color-x-dark` | `#000` |
| `$color-paper` | `#f3f3f3` |

### Semantic theme surfaces (light; `--vf-color-*`)

| Role | Value |
| --- | --- |
| text-default | `#000` |
| text-muted | `rgba(0,0,0,0.6)` |
| text-inactive | `rgba(0,0,0,0.75)` |
| background-default | `#fff` |
| background-alt | `#f7f7f7` |
| background-hover | `rgba(0,0,0,0.05)` |
| background-active | `rgba(0,0,0,0.08)` |
| border-default | `rgba(0,0,0,0.2)` |
| border-high-contrast | `#707070` |
| border-low-contrast | `rgba(0,0,0,0.1)` |

Dark theme (`is-dark`): background-default `#262626`, background-alt `#202020`,
text-default `#fff`, borders `rgba(255,255,255,0.2/0.1)`. The
`--vf-color-*` custom properties switch across themes; components consume those
variables, never raw hex, so theming is automatic.

## Typography

- **Interface/headings:** `"Ubuntu variable", Ubuntu, -apple-system, "Segoe UI", Roboto, …` (variable weight 100–800, self-hosted WOFF2).
- **Mono:** `"Ubuntu Mono variable", "Ubuntu Mono", Consolas, Monaco, Courier, …`.
- **Weights:** thin 300 · regular 400 · medium 500 · bold 550 · display 100.
- **Scale (rem):** display 5 · h1/h2 2.5 · h3/h4 1.5 · h5/h6/body 1 · small 0.875 · x-small 0.75. Base size scales 1.125× on large screens.
- **Line heights:** h1/h2 1.5rem × 2 · h3/h4 1rem × 2 · body 0.75rem × 2 (the Vanilla baseline-nudge system keeps text on the 0.5rem grid).

## Spacing

8px baseline (`$sp-unit: 0.5rem`). Vertical scale: `x-small 0.25rem · small 0.5rem ·
medium 0.75rem · large 1rem · x-large 1.5rem · strip-regular 4rem`. Horizontal
mirrors the same unit (`sph--*`).

## Breakpoints

`x-small 460px · small 620px · large 1090px (project override of Vanilla's 1036px) · x-large 1681px`.

## Elevation

- Default `$box-shadow`: `0 1px 1px 0 rgba(0,0,0,.15), 0 2px 2px -1px rgba(0,0,0,.15), 0 0 3px 0 rgba(0,0,0,.2)`.
- Deep `$box-shadow--deep`: `0 0 2rem 0 rgba(0,0,0,.2)` — for modals/overlays only.
- Disabled opacity: `0.33`. Form control border thickness: `1.5px`.

## Theming

Theme class on `<html>`: `is-light` (default) · `is-dark` · `is-paper`.
`is-paper` is a light variant on the `#f3f3f3` paper ground.

## Components

All in `src/lib/`, exported from `src/lib/index.ts`. Each wraps a Vanilla pattern
as a typed Svelte 5 component (snippet slots, runes state where applicable).

| Component | Vanilla pattern | Variants |
| --- | --- | --- |
| `Button` | `.p-button` | `base · positive · negative · brand · link` |
| `StatusIndicator` | `.p-status-label` | `positive · caution · negative · information` (+ neutral) |
| `Notification` | `.p-notification` | `positive · negative · caution · information` (+ `closable`, `inline`) |
| `Panel` | `.p-panel` | header `title`, `controls` snippet, `content` |
| `EmptyState` | ported `.empty-state` (lxd-ui) | `title`, message, `action` snippet |
| `Kpi` | `.kpi` (token-built stat card) | `label`, `value`, `delta`, `sub` |
| `Segmented` | `.segmented` (token-built control) | `options`, `value`, `onchange`, `ariaLabel` |
| `Chart` | inline SVG on tokens | dual-axis cost + tokens, hover cursor |
| `DataTable` | bare `<table>` (Vanilla base) | cost-sorted attribution, row select, sparkline |
| `UsageDetail` | composes `Panel` / `Chart` / table | per-model breakdown |
| `Navigation` | `.l-navigation` + `.p-side-navigation--icons` | dark sidebar, six links, mobile drawer |
| `UsageView` | composes KPI / chart / table / detail | `scope`, `dimensions`, `defaultDimension` |
| `Chip` | `.p-chip` | `neutral · positive · caution · negative · information · branded` |
| `MyBudget` | composes summary cards + table | personal monthly budget |
| `Approvals` | composes approval list | approve / deny + empty state |
| `OrgChart` | composes search/filter + recursive cards | org hierarchy |
| `Finance` | composes KPI strip + tables | org usage by service / tier |

## Surfaces

Mode: Operate. `src/App.svelte` is a six-page application behind the lxd-ui side
navigation (`.l-application` layout), hash-routed via `#/usage`, `#/team`,
`#/budget`, `#/approvals`, `#/org`, `#/finance`:

- **My usage** / **Team spend** — token-spend instruments (KPI → trend chart →
  attribution table → per-group drill), fed by the synthetic `usage.ts` service.
- **My budget** — personal monthly budget with a per-service spend table.
- **Approvals** — pending budget requests with approve/deny and an empty state.
- **Org chart** — searchable/filterable org hierarchy with per-person allocation.
- **Finance** — org-level KPIs plus usage-by-service and usage-by-tier tables.

The budget/org pages are fed by the synthetic `src/lib/budget.ts` service. Open
decisions: exact synthetic model pricing (labeled illustrative); "today" boundary
is UTC; the chart is inline SVG (no chart dependency).
