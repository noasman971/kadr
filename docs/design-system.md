# Design system

The visual language of the Kadr desktop app: tokens, components and the rules that
keep screens consistent. The flows are in [ux.md](ux.md); a living picture of all of
this is in [mockups/index.html](mockups/index.html).

## Rules

- **Tokens only.** Components use the CSS variables below, never a raw colour, size or
  duration. A new value is a new token, added here first.
- **Dark theme by default** (the picture stands out better), light theme available.
  Both themes define the same tokens.
- **One icon family:** [Lucide](https://lucide.dev) (ISC license), 16 px, stroke 2.
  Every icon button has a tooltip with its shortcut and an `aria-label`.
- **One typeface:** Inter, bundled with the app (SIL Open Font License), with
  tabular figures for timecodes and numbers.
- **Inspired by modern editors, copying none.** Kadr's own accent is violet; the
  playhead is coral.

## Colour tokens

| Token | Dark | Light | Used for |
|---|---|---|---|
| `--color-bg-app` | `#0e1014` | `#e9ebf0` | Behind panels (the 1 px gaps between them) |
| `--color-bg-panel` | `#15181e` | `#f7f8fa` | Panels |
| `--color-bg-raised` | `#1c2028` | `#ffffff` | Buttons, cards, popovers, toasts |
| `--color-bg-hover` | `#242933` | `#eceef3` | Hover |
| `--color-bg-active` | `#2d3340` | `#e0e3ea` | Pressed, selected rows, empty slider track |
| `--color-bg-inset` | `#0a0c0f` | `#e3e6ec` | Inputs, preview background |
| `--color-border` | `#262b35` | `#d9dce3` | Separators |
| `--color-border-strong` | `#363d4a` | `#c3c8d2` | Control outlines |
| `--color-text` | `#e9ebf0` | `#171a21` | Main text |
| `--color-text-secondary` | `#a8aebb` | `#4d5463` | Labels |
| `--color-text-muted` | `#8a91a0` | `#636a79` | Hints, units |
| `--color-accent` | `#6a5af9` | `#5847e8` | Primary buttons, selection, sliders |
| `--color-accent-text` | `#b4abff` | `#4a39d8` | Accent text and icons on panels, focus ring |
| `--color-playhead` | `#ff6a55` | `#e5483a` | Playhead |
| `--color-snap-guide` | `#ffd166` | `#c98a00` | Snapping guide line |
| `--color-success` / `warning` / `danger` | `#3ccf91` / `#f5b544` / `#ff6464` | `#0e7a4e` / `#8a5800` / `#c4302f` | States, always with an icon or words |
| `--color-clip-video` / `audio` / `text` / `image` | `#2b5a8a` / `#22664f` / `#5f3a92` / `#1f6870` | `#3d6fa6` / `#2f7f62` / `#7650ab` / `#2c7f88` | Clip fills (each kind also has an `-edge` colour and an icon) |

### Contrast (WCAG 2.1)

Measured with the WCAG relative-luminance formula. AA asks for 4.5 for normal text,
3 for large text and interface parts.

| Pair | Dark | Light |
|---|---|---|
| Text on panel | 14.9 | 16.4 |
| Secondary text on panel | 8.0 | 7.2 |
| Muted text on panel / on inputs | 5.6 / 6.2 | 5.1 / 4.3 ¹ |
| Accent text on panel | 8.6 | 6.9 |
| White on accent (primary button) | 4.7 | 6.0 |
| White on clip colours (lowest) | 6.4 | 4.7 |
| Success / warning / danger on panel | 8.9 / 9.8 / 6.1 | 5.1 / 5.7 / 5.2 |
| Playhead on panel | 6.3 | 3.7 (interface part: AA) |

¹ Muted text inside light inputs is only used for units next to a value, which
itself has full contrast.

## Spacing, size and shape

| Tokens | Values |
|---|---|
| Spacing (`--space-1` … `--space-12`) | 4, 8, 12, 16, 20, 24, 32, 40, 48 px (4 px scale) |
| Radius (`--radius-sm` … `--radius-xl`, `--radius-full`) | 4, 6, 10, 14 px, pill |
| Control height | 30 px (28 px in dense rows, 40 px for large buttons) |
| Layout | top bar 44 px, library 280 px, inspector 300 px, timeline 300 px, track headers 168 px (side panels shrink below 1360 px wide) |

## Typography

| Token | Size | Use |
|---|---|---|
| `--font-size-xs` | 11 px | Badges, units, hints |
| `--font-size-sm` | 12 px | Labels, secondary text |
| `--font-size-md` | 13 px | Body (default) |
| `--font-size-lg` | 15 px | Timecode, emphasised text |
| `--font-size-xl` | 18 px | Dialog titles |
| `--font-size-2xl` / `3xl` | 24 / 32 px | Home screen |

Weights: 400, 500 (buttons, names), 600 (titles). Timecodes and numbers use
`font-variant-numeric: tabular-nums` so they do not jitter while playing.

## Motion

| Token | Value | Use |
|---|---|---|
| `--duration-fast` | 120 ms | Hover, toggles |
| `--duration-normal` | 200 ms | Panels, dialogs, toasts |
| `--easing-standard` | `cubic-bezier(0.2, 0, 0, 1)` | Everything |

With `prefers-reduced-motion`, transitions and animations are turned off.

## Components

Each component has these states where they apply: normal, hover, pressed, focus
(2 px accent-text ring, offset 2 px), disabled (45 % opacity), loading (spinner),
error (danger outline and message).

| Component | Notes |
|---|---|
| Button | Default, primary (one per screen area), ghost, danger, large |
| Icon button | 30 px square; `aria-pressed` for toggles (snapping, mute, hide) |
| Number field | Label or axis, value, unit; type, arrow keys (Shift = ×10), or drag sideways |
| Slider | Paired with a number field; the filled part shows the value |
| Segmented control | Small exclusive choices (speed, alignment, theme) |
| Select, text input, text area | 30 px, inset background |
| Toggle switch | On/off settings |
| Section | Folding inspector group with a *Reset* action |
| Tabs | Library *Media* / *Text* |
| Tooltip | Label + shortcut key caps |
| Dialog | Header, body, footer with the primary action on the right; Esc closes |
| Toast | Bottom of the preview, icon + message + optional action (*Undo*) |
| Progress bar | Determinate; with percentage and remaining time nearby |
| Badge, chip, key cap | Small labels; format chip in the top bar; shortcut keys |
| Media tile | Thumbnail, kind icon, length, used-in-timeline check, add button; reading and error states |
| Clip | Kind colour + icon + name, thumbnails and/or waveform, badges (speed, volume), selected outline and trim handles |
