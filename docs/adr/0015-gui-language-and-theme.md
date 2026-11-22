# ADR 0015: Panel language selection and the follow-system light/dark theme

- **Status:** accepted
- **Date:** 2026-11-22

## Context

The desktop panel shipped in Chinese only, with a fixed light content palette and
a dark header. Two requests followed:

1. **Simplified Chinese and English.** The operator asked for English support
   without a language picker: the panel should read the system language at
   startup and use English for every language it does not support.
2. **Light and dark mode that follows the system.** The settings dialog should
   offer *follow the system*, *light* and *dark*. This is a reversal of the
   earlier assumption that a light-only panel was enough, and it interacts with
   the existing hand-built palette in `ui/theme.slint`.

Constraints from the repository:

- No WebView and no new GUI toolkit; the panel stays Slint + software renderer.
- Static UI text lives in `.slint` files; a large part of the operator-facing
  text is built in Rust (log lines, validation errors, native dialog titles), so
  translating only the `.slint` files would leave the panel half-translated.
- State files must stay advisory, per-user and separate from `ServerConfig`.
- The theme probe must not block the UI thread and must not leave helper
  processes behind.

## Decision

### Language: decided once at startup, English is the source language

- `src/i18n.rs` owns a two-value `Language` (`En`, `ZhCn`) and maps the system
  locale (`sys-locale`, already a transitive Slint dependency) to one of them:
  - `zh`, `zh-CN`, `zh_CN`, `zh-Hans*`, `zh-SG` → Simplified Chinese;
  - Traditional Chinese (`zh-TW`, `zh-HK`, `zh-MO`, `zh-Hant*`) and every other
    language → English, the fallback;
  - no locale at all → English.
- The decision is taken **once**, before the window is created. There is no
  language picker and no runtime switch, so the activity log does not have to
  store message IDs and no already-written line can end up in the "wrong"
  language.
- Static strings use `@tr(...)` in the `.slint` files with **English source
  strings**. `translations/zh_CN/LC_MESSAGES/tftpd-r-gui-slint.po` is bundled
  into the executable by `slint-build`
  (`with_bundled_translations` + `with_default_translation_context(None)`, so
  the catalogue has no `msgctxt`). At startup the panel calls
  `slint::select_bundled_translation("zh_CN" | "en")`, which also overrides
  Slint's own locale guess — that guess would treat `zh-TW` as Chinese because
  it matches on the language part only.
- The Rust side uses `i18n::Text` for everything it formats itself: log lines,
  validation errors, status text, address labels, the export header and the
  native dialog titles. It renders in the same language as the catalogue, so the
  panel never mixes the two.
- Native dialogs receive their title and file-type filter through environment
  variables (Windows) or fixed arguments (Unix). Message text is never
  interpolated into a PowerShell script.
- `TFTPD_R_GUI_LANGUAGE` is a development-only override (no UI): on a machine
  whose Windows UI language is English, looking at the Chinese layout otherwise
  requires changing the OS language. It is documented as a development aid and
  obeys the same mapping (so `zh-TW` still yields English).

### Theme: three choices, custom tokens, and a detected system value

- `settings.slint` gains an **Appearance** row with three mutually exclusive
  buttons: *Follow system*, *Light*, *Dark*. The choice applies on click (it is
  a display preference) and is persisted in `ui-prefs.txt`
  (`theme-mode=system|light|dark`), a new advisory per-user state file next to
  `recent-dirs.txt`, `last-root.txt` and `ui-state.txt`.
- `ui/theme.slint` now holds **both palettes** and exposes `Theme.dark`. Every
  colour token is a function of `dark`, so no component keeps a hard-coded
  colour that would break in the other scheme: surfaces, borders, text levels,
  accents, the tinted level chips, disabled/hover/pressed states, selection
  colours and the scrim all moved into the palette. The header block stays dark
  in both schemes, with a slightly different shade per scheme.
- The window copies the resolved value into `Palette.color-scheme`, so the
  standard widgets the panel embeds (the `ScrollView` scrollbars) follow the
  same choice. This has to happen in `init`/`changed` handlers: assigning a
  global's property at component scope is a parse error, and the window property
  is what Rust sets.
- The system's own preference is read from the platform, not from Slint:
  - Windows: `reg.exe query HKCU\...\Themes\Personalize /v AppsUseLightTheme`
    (`0` = dark);
  - Unix: `gsettings get org.gnome.desktop.interface color-scheme`, then
    `GTK_THEME` as a fallback;
  - anything unparsable or missing means light.
  `SystemThemeWatcher` reads the value synchronously once (so the first frame is
  correct) and then every 30 s on a worker thread while the panel follows the
  system. The probe is disabled in light/dark mode, and stopped when the panel
  exits.

## Alternatives considered

- **Runtime language switch in the settings dialog.** Rejected for now: it
  requires structured log storage (message IDs and arguments) so existing lines
  can be re-rendered, and it doubles the translation surface under test. The
  request was a startup decision, and that is what shipped.
- **gettext `.mo` files next to the executable.** Rejected: it adds a runtime
  dependency and deployable files (and gettext is not available on Windows by
  default). Bundled `.po` catalogues need nothing at run time.
- **Slint's `Palette` as the only source of truth.** Rejected: reading
  `Palette.color-scheme` gives `unknown` whenever the style resolves the system
  scheme itself, so the custom palette could not tell light from dark; forcing
  it from Rust permanently replaces the platform binding, so switching back to
  "follow the system" would stop tracking the OS. Reading the platform value in
  Rust keeps both behaviours exact.
- **Detecting the system scheme through `StyleMetrics`/`Palette` and forcing
  only in explicit mode.** Same binding-loss problem, plus an `unknown` case
  that silently looks light. Rejected.
- **Polling the system scheme every few hundred milliseconds.** Rejected: each
  Windows probe starts a process (~40 ms), which is far too much for the UI
  timer; 30 s on a worker thread is imperceptible for a theme change.
- **A language picker in the settings dialog instead of the system locale.**
  Rejected by the request: unsupported languages must fall back to English
  automatically, and an explicit picker would be a second source of truth.

## Consequences

- The panel is bilingual without a WebView, without a new runtime dependency and
  without untranslated Rusk-built strings. Adding a language means a new
  `Language` variant, a locale mapping, a `.po` catalogue and the matching
  `Text` methods.
- English is the **source** language: the `.slint` files must stay English, and
  a unit test enforces that (plus catalogue coverage, no unused entries and
  matching `{}` placeholders).
- Chinese text is no longer visible in the source files, so reviewing a
  translation change means reading the `.po` file.
- Two palettes must be kept in sync by hand; the token list is the single place
  to change a colour, and a component that hard-codes a colour is now a bug.
- Following the system costs one `reg.exe`/`gsettings` call at startup and one
  every 30 s while that mode is active. A system theme change is picked up
  within 30 s instead of instantly.
- `ui-prefs.txt` is one more advisory state file. It stores a single word, is
  never read by the TFTP server and a failed write only means the choice is not
  remembered (logged as a WARN).
- The development override `TFTPD_R_GUI_LANGUAGE` is not reachable from the UI;
  `--smoke-test` now prints the resolved language, the translated tagline (proof
  that the catalogue was applied), the theme choice and the resolved darkness.
