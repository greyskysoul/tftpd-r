# ADR 0015: Panel language selection and the follow-system light/dark theme

- **Status:** accepted
- **Date:** 2026-11-22
- **Revised:** 2026-11-23 (settings navigation, persisted language preference,
  root-history drop-down in the settings form)

## Context

The desktop panel shipped in Chinese only, with a fixed light content palette and
a dark header. Two requests followed:

1. **Simplified Chinese and English.** The operator asked for English support.
   The first implementation read the system language at startup and used English
   for every language it did not support.
2. **Light and dark mode that follows the system.** The settings dialog should
   offer *follow the system*, *light* and *dark*. This is a reversal of the
   earlier assumption that a light-only panel was enough, and it interacts with
   the existing hand-built palette in `ui/theme.slint`.

A later request added three related changes:

3. **A language setting.** The operator wants an explicit choice instead of
   relying only on the operating-system locale: *follow the system*, *English*
   or *Simplified Chinese*.
4. **A settings dialog that separates the two kinds of setting.** Server values
   (port, bind addresses, root, transfer limits) must not sit next to display
   preferences (appearance, language) as if they had the same lifetime and the
   same "Save" semantics.
5. **Root-directory history in the settings form.** The status-bar menu already
   offers recently used roots; the root field should offer the same history
   without leaving the dialog.

Constraints from the repository:

- No WebView and no new GUI toolkit; the panel stays Slint + software renderer.
- Static UI text lives in `.slint` files; a large part of the operator-facing
  text is built in Rust (log lines, validation errors, native dialog titles), so
  translating only the `.slint` files would leave the panel half-translated.
- State files must stay advisory, per-user and separate from `ServerConfig`.
- The theme probe must not block the UI thread and must not leave helper
  processes behind.
- A language change must not leave already-rendered log lines or native dialog
  titles in a different language from the rest of the panel.

## Decision

### Language: one process, one language; the preference is selected in the settings dialog

- `src/i18n.rs` owns a two-value `Language` (`En`, `ZhCn`) and a
  `LanguagePreference` (`System`, `En`, `ZhCn`) plus the persisted spelling
  (`system`, `en`, `zh-CN`).
- The preference is stored in `ui-prefs.txt` next to the theme choice. At
  startup it is resolved **once**, before the window is created:
  - `System` maps the system locale: `zh`, `zh-CN`, `zh_CN`, `zh-Hans*`,
    `zh-SG` → Simplified Chinese; Traditional Chinese (`zh-TW`, `zh-HK`,
    `zh-MO`, `zh-Hant*`) and every other language → English, the fallback;
  - `En` and `ZhCn` override the system locale explicitly;
  - a missing, unreadable or corrupt `language=` entry means `System`.
- The settings dialog's **Appearance** page offers the three choices. Selecting
  one stores the preference immediately and logs the result. The running process
  keeps its current language: the in-memory activity log stores rendered
  strings, and the native folder/save dialogs captured their titles at startup.
  The inline hint says the change applies after a restart. This keeps
  every rendered line and every native dialog in one language; a runtime switch
  would require structured log storage (message IDs and arguments) and a way to
  rebuild the helper titles, which is deliberately out of scope.
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
- `TFTPD_R_GUI_LANGUAGE` is a development-only override (no UI). It still wins
  over the persisted preference for that run, so a machine whose Windows UI
  language is English can be used to inspect the Chinese layout without changing
  the state file. It obeys the same mapping (so `zh-TW` still yields English).

### The settings dialog is split into Server and Appearance pages

- `settings.slint` now has a left navigation bar with two entries:
  - **Server** — port, bind-address multi-select, root directory, block size,
    timeout and maximum concurrent sessions. The footer keeps *Cancel* and
    *Save settings*; Save runs the same `ServerConfig::validate` rules as the
    daemon and restarts an idle running server as before.
  - **Appearance** — the theme choice (follow the system / light / dark) and the
    language choice (follow the system / English / Simplified Chinese). The
    footer shows a single *Close* button. These preferences are applied
    independently: the theme applies immediately, the language is stored for the
    next start, and neither touches the server configuration or restarts the
    server.
- The page is local dialog state and resets to Server each time the dialog
  opens, so the first-run flow still lands on the server fields. The validation
  error box stays outside the pages, above the footer, so a failed Save is
  visible regardless of the active page.

### Root history inside the settings form

- The recent-root list is already loaded for the status-bar menu. The settings
  dialog receives the same read-only `RecentDir` model and renders a small
  history drop-down under the root field, opened by a history icon next to
  *Browse…*.
- Selecting an entry only fills the form (and shows the same inline "click Save
  settings" reminder as a picked directory). It does not apply the root, record
  a new history entry or close the dialog: the settings form must remain
  cancellable, and the 清空 action still belongs to the status-bar menu.
- The drop-down has a short empty state ("No recent directories.") and is an
  overlay sibling of the card, like the bind address picker, so it is not
  clipped by the content layout.

### Theme: three choices, custom tokens, and a detected system value

- `settings.slint` has an **Appearance** row with three mutually exclusive
  buttons: *Follow system*, *Light*, *Dark*. The choice applies on click (it is
  a display preference) and is persisted in `ui-prefs.txt`
  (`theme-mode=system|light|dark`).
- `ui/theme.slint` holds **both palettes** and exposes `Theme.dark`. Every
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

- **Runtime language switch in the settings dialog.** Rejected: it requires
  structured log storage (message IDs and arguments) so existing lines can be
  re-rendered, and the pre-warmed folder picker and save dialog would have to
  rebuild their native titles. The request is satisfied by a persisted
  preference and a clear "applies after restart" hint, which keeps the panel
  consistent for its whole lifetime.
- **No language picker; system locale only.** Rejected by the later request: an
  operator on a shared or English-configured machine needs an explicit override.
  `TFTPD_R_GUI_LANGUAGE` remains a development aid, not the user-facing answer.
- **A single settings page with a section heading instead of a navigation bar.**
  Rejected: server values and display preferences have different save semantics
  (one validates and may restart the server, the other applies immediately or
  on the next start). A left navigation makes that distinction structural.
- **Let the settings history apply the root immediately.** Rejected: the form
  must remain cancellable; applying on selection would make Cancel unable to
  restore the active root and would unexpectedly restart the server.
- **A second recent-directory component.** Rejected: the status-bar `RootMenu`
  is anchored to the status bar and applies on click. The dialog reuses the
  `RecentDir` model with a small selector whose contract is "fill the form".
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

## Consequences

- The panel is bilingual without a WebView, without a new runtime dependency and
  without untranslated Rust-built strings. Adding a language means a new
  `Language` variant, a locale mapping, a `.po` catalogue and the matching
  `Text` methods.
- English is the **source** language: the `.slint` files must stay English, and
  a unit test enforces that (plus catalogue coverage, no unused entries and
  matching `{}` placeholders).
- Chinese text is no longer visible in the source files, so reviewing a
  translation change means reading the `.po` file.
- `ui-prefs.txt` now stores two independent keys:
  `theme-mode=system|light|dark` and `language=system|en|zh-CN`. A missing or
  corrupt key falls back to its default; a failed write logs a WARN and, for the
  language, means the next start keeps the previous language.
- The running panel never switches language. The activity log and native dialog
  titles stay in the startup language, and the Appearance page says so. A
  restart is required after changing the preference.
- The settings dialog has a fixed 780 × 500 px card with a scrollable content
  pane, so a long error message or a future extra field does not push the footer
  out of the window. The minimum window height (600 px) leaves room for it.
- The two pages share one `SettingsDialog` component and one set of properties;
  the validation path is unchanged, and Save is only offered on the Server page.
- `TFTPD_R_GUI_LANGUAGE` still wins over the persisted preference for one run;
  `--smoke-test` prints the resolved language, the persisted preference, the
  translated tagline (proof that the catalogue was applied), the theme choice
  and the resolved darkness.
