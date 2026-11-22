# ADR 0016: Level-based log filtering without `env-filter`

- **Status:** accepted
- **Date:** 2026-11-22
- **Follows:** [ADR 0010](0010-size-first-release-profile.md) (size-first release
  profile)

## Context

After ADRs 0010 and 0011 the GNU x64 daemon measured **1,357,824 B (1.29 MiB)**
and the GUI **9,036,288 B (8.62 MiB)**. To look for further low-cost reductions,
an unstripped release build was attributed to crates with `nm --demangle` over
the COFF symbol table (symbol sizes are the distance to the next code symbol;
the workspace keeps no such script).

The daemon's `.text` was 965,632 B and the largest contributors that the
workspace actually controls were:

| Crates | `.text` | Reason |
| --- | ---: | --- |
| `regex-automata` + `regex-syntax` + `matchers` | ~146 KB | `tracing-subscriber`'s `env-filter` |
| `clap_builder` | 105 KB | CLI parsing (kept) |
| `toml` + `toml_parser` | 67 KB | configuration parsing (kept) |
| `gimli` + `addr2line` + `rustc-demangle` | 68 KB | `std` panic/backtrace support, not ours to remove |

The daemon did not use `env-filter`'s span- or field-scoped directives, and its
own dependency graph logs only through `tracing`, so the `regex` engine was
linked for a filter syntax that was neither documented nor exercised. The GUI is
not affected: it has no `tracing` subscriber.

Prototype measurements with the same source tree and profile:

| Variant | Daemon |
| --- | ---: |
| baseline (`env-filter`) | 1,357,824 B |
| fixed `LevelFilter` only | 1,059,840 B |
| level-based `Targets` filter (chosen) | 1,063,936 B |

Further candidates were measured and rejected:

| Candidate | Saving | Why not |
| --- | ---: | --- |
| `clap` without `color` | 18,432 B | removes coloured `--help`/errors; UX cost not worth 1.8 % |
| `clap` without `suggestions` | 6,656 B | removes "did you mean" hints |
| `slint` `compat-1-2` → `compat-1-18` | 0 B | `compat-1-18` is the actual compile-time requirement |
| `chrono` `default-features = false` | 0 B | `clock` + `std` are the only used features |
| Slint image/SVG decoders, menus, clipboard | ~600 KB+ | hard-wired by `i-slint-backend-winit`; needs an upstream feature or a fork |
| Slint system-font/parley stack | ~1 MB | required for text rendering, including Chinese |
| nightly `build-std` / `panic_immediate_abort` | large | already excluded by ADR 0010 / AGENTS.md |

## Decision

Do not enable `tracing-subscriber`'s `env-filter` (or `tracing-log`) feature:

```toml
tracing-subscriber = { version = "0.3", default-features = false, features = ["fmt", "ansi"] }
```

`crates/tftpd-r-daemon/src/logging.rs` parses the level-based subset itself and
installs a `registry` + `fmt::layer` subscriber with a `Targets` filter. The
accepted directives are:

- `level` — `off`, `error`, `warn`, `info`, `debug`, `trace`, or `0`–`5`;
- `target=level` — maximum level for targets whose name starts with `target`;
- `target` — shorthand for `target=trace`.

Directives are comma-separated and surrounding spaces are ignored. Invalid
directives are reported on stderr and skipped so a typo cannot stop the daemon.

The rendered log lines are unchanged: UTC timestamp, level, message, and the
same fields, with ANSI colour only when stdout is a terminal.

## Consequences

- The GNU x64 daemon drops from 1,357,824 B to **1,063,936 B (−293,888 B,
  −21.6 %)**. The GUI is unchanged at **9,036,288 B**.
- Behaviour differs from `EnvFilter` in three documented ways:
  - a filter with only target directives keeps the `info` default for other
    targets instead of turning them off, so
    `TFTPD_R_LOG=tftpd_r_core=debug` still logs the daemon's lifecycle lines;
  - span- and field-scoped directives (`[span{field=value}]=level`) are not
    supported;
  - `TFTPD_R_LOG=""` now means `info` rather than "log nothing".
- `tracing-log` is not enabled. No crate in the daemon's dependency graph logs
  through the `log` crate today; if one starts to, its records are dropped
  silently and this decision must be revisited (6,144 B of the saving comes from
  this feature alone).
- Six unit tests cover the parser (`logging.rs`): default level, bare level,
  target directives, bare target, numeric/uppercase levels, and invalid
  directives. The daemon was also run manually with `--log debug`,
  `--log tftpd_r_core=debug`, `TFTPD_R_LOG=warn`, and an invalid directive to
  confirm the rendered output and warnings.
- Do not re-enable `env-filter` without re-measuring: it restores the `regex`
  engine and about 290 KiB. If regex or span-scoped filtering is ever needed,
  that is a new decision, not a quiet feature flip.
- The `clap` colour/suggestion reductions remain available (24,064 B together:
  18,432 B for `color`, 6,656 B for `suggestions`) if a future release values the
  last 2.3 % of the daemon over the CLI hints.
