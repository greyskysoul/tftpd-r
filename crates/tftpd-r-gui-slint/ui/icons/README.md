# Material Design Icons

`../icons.slint` embeds the 24x24 `materialicons` (filled) path data of the
icons used by the TftpdR panel. They come from the official
[google/material-design-icons](https://github.com/google/material-design-icons)
repository (Apache License 2.0, see `LICENSE`).

| Slint constant | Icon | Source SVG |
| --- | --- | --- |
| `Icons.check` | `check` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/navigation/check/materialicons/24px.svg` |
| `Icons.check-circle` | `check_circle` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/action/check_circle/materialicons/24px.svg` |
| `Icons.close` | `close` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/navigation/close/materialicons/24px.svg` |
| `Icons.crop-square` | `crop_square` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/image/crop_square/materialicons/24px.svg` |
| `Icons.delete-sweep` | `delete_sweep` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/content/delete_sweep/materialicons/24px.svg` |
| `Icons.description` | `description` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/action/description/materialicons/24px.svg` |
| `Icons.dns` | `dns` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/action/dns/materialicons/24px.svg` |
| `Icons.error` | `error` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/alert/error/materialicons/24px.svg` |
| `Icons.file-download` | `file_download` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/file/file_download/materialicons/24px.svg` |
| `Icons.filter-none` | `filter_none` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/image/filter_none/materialicons/24px.svg` |
| `Icons.folder` | `folder` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/file/folder/materialicons/24px.svg` |
| `Icons.folder-open` | `folder_open` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/file/folder_open/materialicons/24px.svg` |
| `Icons.history` | `history` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/action/history/materialicons/24px.svg` |
| `Icons.info` | `info` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/action/info/materialicons/24px.svg` |
| `Icons.keyboard-arrow-down` | `keyboard_arrow_down` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/hardware/keyboard_arrow_down/materialicons/24px.svg` |
| `Icons.play-arrow` | `play_arrow` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/av/play_arrow/materialicons/24px.svg` |
| `Icons.remove` | `remove` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/content/remove/materialicons/24px.svg` |
| `Icons.replay` | `replay` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/av/replay/materialicons/24px.svg` |
| `Icons.restart-alt` | `restart_alt` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/device/restart_alt/materialicons/24px.svg` |
| `Icons.settings` | `settings` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/action/settings/materialicons/24px.svg` |
| `Icons.speed` | `speed` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/av/speed/materialicons/24px.svg` |
| `Icons.stop` | `stop` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/av/stop/materialicons/24px.svg` |
| `Icons.swap-vert` | `swap_vert` | `https://raw.githubusercontent.com/google/material-design-icons/master/src/action/swap_vert/materialicons/24px.svg` |

Only the path data is embedded; the icons are recolored at runtime through
the `MaterialIcon.tint` property. To change or add an icon, download its
`materialicons/24px.svg`, append the `d` attribute(s) of its visible paths to
the matching constant in `../icons.slint` (keep the SVG winding order so holes
render correctly), and add the source URL to the table above.

## Product icon

The TftpdR application icon is built from the same Material set: the `dns`
(server racks) glyph in `#fafafa`, on a rounded `#263238` square (corner radius
56 of 256, glyph box 192 of 256) with a soft top-light sheen (white 12 % at the
top edge fading out by 42 %, black up to 10 % at the bottom edge).

| File | Format | Used by |
| --- | --- | --- |
| `app-icon.svg` | 256x256 SVG, master artwork | Source of every other file |
| `app-icon.png` | 256x256 PNG, straight alpha | General artwork / packaging |
| `app-icon.ico` | ICO with 16, 24, 32, 48, 64, 128, 256 px (BMP frames below 256, PNG for 256) | Windows shortcuts, future `.exe` resource |
| `app-icon-128.rgba` | 128x128 straight-alpha RGBA, no header (64 KiB) | Window / task-bar icon: `src/main.rs` embeds it with `include_bytes!` and uploads it through `Window.icon`, because the panel build has no runtime image decoder (it does not enable `image-default-formats`) |

The in-app brand mark (`Widgets.BrandMark` in `../widgets.slint`) draws the
same tile with Slint primitives so it stays crisp at any scale: `#263238` tile,
`#fafafa` glyph, ratios from the SVG. It is deliberately flat — the panel
surfaces use no gradients — and it sits on the flat `#37474f` header of
`../theme.slint`, one step lighter than the tile so the rounded square keeps its
shape.

The three bitmaps are generated from `app-icon.svg`; do not hand-edit them.
Regenerate them on Windows with the throwaway scripts kept outside version
control (`.bootstrap/icon/gen_icon.py` after `render_wpf.ps1`), or with any
SVG rasteriser plus an ICO packer, then confirm `cargo test -p
tftpd-r-gui-slint` still passes: `product_icon_pixels_are_a_valid_rounded_tile`
checks the embedded RGBA buffer (size, transparent corners, `#263238` tile,
`#fafafa` glyph).

Known gap: the Windows `.exe` files do not embed the icon as a PE resource, so
Explorer shows the generic executable icon until the packaging step (Phase 8)
adds a resource compiler. The running application does use it for the window and
task-bar icon.
