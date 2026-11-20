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
