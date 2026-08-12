# Design Doc: Entry Timestamps for intake

Status: Implemented

## Overview

Every log entry gets an optional timestamp recording when it was logged. It
is stored as a full RFC 3339 timestamp string in UTC
(`2026-08-12T21:45:03Z`) and written automatically when entries are added —
by the plain `log` command (both food and ad-hoc paths) and by `ai log`
additions. The feature is on by default and configurable: writing is
toggled by `write_timestamps` (default `true`), the day view gains a
configurable Time column (`show_timestamp`, default `true`), and the cell
format is controlled by `time_format` (default `"24h"` → `HH:MM`). The time
of day of an entry can also be set or corrected explicitly — the
"I can't log the meal 'til later" workflow: `--time HH:MM` on `log` stamps
the new entry at that time instead of now, and `intake retime <n> <HH:MM>`
adjusts an entry already logged (see "Adjusting timestamps"). Either way
the stored value is the same full UTC string.

Timestamps are metadata, not macros: they never participate in totals,
targets, colors, or AI context, and they are set by intake at write time —
never by the model, never by the ops schema. Every timestamp byte is
constructed by intake's own parse/conversion code, whether it comes from
the clock or from explicit user input.

## Storage & schema

```rust
// log.rs
#[derive(Debug, Clone, PartialEq)]
pub struct Timestamp(chrono::DateTime<Utc>);

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogEntry {
    pub title: String,
    pub servings: Servings,
    // ... the six macros ...
    pub timestamp: Option<Timestamp>,
}
```

- The type is a thin newtype over `chrono::DateTime<Utc>`, serialized as an
  RFC 3339 string via chrono's `serde` feature (enabled on the existing
  `chrono = "0.4"` dependency): `timestamp = "2026-08-12T21:45:03Z"`. Full
  date+time+zone, seconds precision — never a bare time.
- `Option<Timestamp>` makes the field backward-compatible on read: existing
  day files without `timestamp` deserialize as `None` and keep working.
  Strict parsing holds: a present-but-malformed timestamp string (garbage,
  local-offset-only, out-of-range components) is a load error naming the
  file — never a silent default, per the strict-parsing convention.
- `deny_unknown_fields` is unchanged, which makes this a **forward-breaking
  schema change** in the established sense (see AI-DESIGN.md "Change review
  & safety"): a day file containing `timestamp` fails loudly on an older
  binary instead of being silently rewritten without it. That policy is
  intentional; the `Option` is the migration — new binaries read old files,
  and the field only appears once the new binary writes it.
- Not an amount type: no decimal, no overflow concerns. Validation is
  format validation, exactly like the `title` string.

## Writing timestamps

Semantics: by default the timestamp is the time the entry was **logged** —
captured once per command invocation as `Utc::now()` — not the target date.
Backfilling with `intake log coffee --date 2026-08-01` still stamps now,
which is the honest record of when the entry was written. An explicit
`--time HH:MM` replaces that default with a time of day on the target date
(see "Adjusting timestamps").

Stamping is a **command-layer concern**: the pure write functions
(`append_entry`, `update_day`, `write_day_checked`) and `apply_ops` never
stamp, so their determinism — and every existing unit test — is untouched.

- **Plain `log`** (`src/commands/log.rs::cmd_log`): both paths construct
  their `LogEntry` with `timestamp: Some(Timestamp::now())` when
  `config.write_timestamps()` is true, `None` otherwise — unless `--time`
  was given, which always wins over both the default and the toggle.
  `append_entry` serializes it like any other field.
- **`ai log`**: `apply_ops` stays pure and the `DayLogOps` schema is
  unchanged — the model never sees, emits, or manipulates timestamps, and
  the proposal diff's entry lines (`title | servings | macros`) do not show
  them. After confirmation, immediately before `write_day_checked`, the ai
  command stamps the entries **added by add ops** with one captured `now`
  (see below); `remove` is unaffected and `replace` keeps the row's original
  timestamp — a replace edits an existing log entry's content, it does not
  re-log it. The written file carries the timestamps even though the
  proposal didn't preview them.
  - Identifying additions: `apply_ops` appends all additions at the end
    (by construction — `entries.extend(additions)`), so the added rows are
    exactly the trailing `k` entries where `k` is the count of
    `add-food` + `add-adhoc` ops in the applied op list. A small helper in
    the gated module (`stamp_added_entries(day, ops, now)`) implements this
    rather than threading indices through `apply_ops`.
  - The stale-write check is unaffected: stamping happens on `new` *after*
    the `current == expected` comparison.
- **`write_timestamps = false`**: no automatic stamping — plain `log`
  entries and `ai log` additions serialize with no `timestamp` field unless
  the user explicitly requests one via `--time` / `retime`. Nothing else
  changes; the Time column renders empty cells for such entries.
- **`rm`**: unaffected. `remove_entry`'s expected-entry equality compares
  whole entries including the timestamp, and both sides come from the same
  day file, so a listed entry still matches.
- Clock: `Timestamp::now()` is called once per command, so all entries added
  by one invocation share one timestamp. Unit tests that need determinism
  construct `Timestamp`s explicitly; e2e tests assert presence and format,
  not exact values.

## Adjusting timestamps

Two explicit surfaces record or correct the time of day of an entry — the
"I want to track the time of a meal but can't log it 'til later" workflow.
Both take `HH:MM` in **local** time on the target date (the date flags'
target — `--date` / `--days-ago`, defaulting to today) and convert to UTC at
the boundary, matching the display side's local-time rendering. Seconds are
zero in the stored value (`2026-08-11T12:30:00Z`). Explicit user intent wins
over the `write_timestamps` toggle: an explicitly timed entry always carries
its timestamp, even when the flag is off.

- **`--time HH:MM` on `log`** — sets the new entry's timestamp at log time,
  replacing the "now" default. Composes with `--date` / `--days-ago`, so the
  late-logging workflow is `intake log turkey-chili --days-ago 1 --time 12:30`:
  the entry lands on yesterday's log stamped yesterday 12:30 local. Applies
  to both the food and ad-hoc paths.
- **`intake retime <n> <HH:MM> [--date D] [--yes]`** — adjusts an existing
  entry's timestamp after the fact: the 1-based row number as shown in the
  day view, plus the new time. Mirrors `rm`'s shape and safety: loads the
  day, shows the row ("Set entry 3 (Coffee, 1 serving, 12 kcal) to 14:30?"),
  runs the standard `[y]es` / `[n]o` confirm (`--yes` skips, per the shared
  `Args`-style flags), then performs a locked read-modify-write via a new
  `log.rs::set_entry_timestamp` — the `remove_entry` pattern: expected-entry
  equality under `lock_log_dir`, then `write_day_locked`. A concurrent
  change between the confirm read and the write aborts instead of stamping a
  different row. Also fills in a legacy entry's missing timestamp
  (`None` → `Some`) — the one-off backfill path.

Parsing and edge cases:

- `HH:MM` is parsed strictly (chrono `%H:%M`); anything else is a usage
  error. Input is always 24-hour — `time_format` controls display only and
  never input.
- The local→UTC conversion is intake's, with the same discipline as
  everything else: an ambiguous or nonexistent local time (DST gap or
  fall-back) is a clear error, never a guessed instant.
- `retime` only rewrites the timestamp of a row on the target day; it cannot
  move an entry to another day. Cross-day moves are future work.
- Storage never records provenance: `--time` / `retime` produce the same RFC
  3339 UTC strings as the automatic stamp, with no marker distinguishing an
  explicit time from an automatic one. The file stores the instant, not
  where it came from.

AI stays out of adjustment: the model never emits or manipulates timestamps,
so the "intake computes, never the model" property holds. A `set-time` op
for `ai log` is deferred (see Open questions) — the plain `retime` covers
the workflow.

## Display

The day view (`render_day`) gains a **Time** column between `Servings` and
the macro columns, controlled by a new config key `show_timestamp` (default
`true`). "Configurable like the other columns" is implemented as a separate
boolean rather than a `time` value inside `show_columns`: the `Column` /
`ColumnValue` machinery is Decimal-typed (targets, `column_color`,
`log_cell`) and time cannot be a macro column without polluting the
targets/colors system. The Time column is metadata — uncolored,
right-aligned, no target band.

- Cell rendering: convert the stored UTC instant to **local time** at the
  display boundary (matching the day view's existing "today" semantics —
  the color scaling already uses `Local::now()`), then format per
  `time_format`:
  - `"24h"` (default) → `HH:MM`, zero-padded, e.g. `14:05`.
  - `"12h"` → `h:mm AM/PM`, e.g. `2:05 PM`.
  - Any other value is a config error (`Config::time_format()` bails),
    per the strict-parsing convention.
- Entries with no timestamp (`None` — legacy days, or written with
  `write_timestamps = false`) render an **empty cell**; the column stays in
  place when `show_timestamp` is on.
- Scope is the day view only: `summary` aggregates whole days and `food
  list` / `food show` have no entries, so they gain nothing. "Log views"
  means the day-log table.
- AI context is unaffected: the entry-line format embedded in prompts
  (`title | servings | macros`) does not gain a time — config controls
  human display, not model context, the same principle that keeps
  `show_columns` out of prompts (AI-DESIGN.md "Context windows").

## Configuration

```toml
# config.toml
write_timestamps = true    # write timestamps on new entries (default true)
show_timestamp = true      # Time column in the day view (default true)
time_format = "24h"        # "24h" → HH:MM (default) | "12h" → h:mm AM/PM
```

- All three are plain `Option<…>` fields on `Config` with resolving getters
  (`write_timestamps()`, `show_timestamp()`, `time_format()`) in the style
  of `columns()`: absent key → default, present-but-invalid → a friendly
  error. `time_format` is a string-backed enum (`"24h"` / `"12h"`) so
  unknown values fail at config load.
- Config-only, like the targets: no env vars, no CLI flags (except the
  explicit `--time` / `retime` arguments, which are not config — they
  override the `write_timestamps` default per invocation).
- No interaction with the `[ai]` table; the keys are top-level.

## Testing

- `log.rs` unit: `Timestamp` roundtrip (serializes exactly
  `2026-08-12T21:45:03Z`); unparseable timestamp strings rejected at load;
  absent field → `None`;
  legacy fixture days (no `timestamp`) still load and render; entry equality
  with timestamps.
- `config.rs` unit: defaults (write true / show true / 24h); `write_timestamps
  = false` and `show_timestamp = false` parse; `time_format = "12h"` parses;
  `time_format = "bogus"` errors with a friendly message.
- `commands/log.rs` unit: `cmd_log` stamps both paths when enabled and not
  when disabled (assert `Some`/`None` on the appended entry); `--time`
  stamps the given local time on the target date — and overrides
  `write_timestamps = false`; `render_day`
  renders the Time column (24h and 12h cells for a fixed UTC instant,
  converting to local; empty cell for `None`; column absent when
  `show_timestamp = false`).
- `log.rs` unit (`set_entry_timestamp`): sets on an existing entry,
  overwrites an existing timestamp, fills `None`, out-of-range and no-day
  errors, stale-entry abort (entry changed since the confirm read), and the
  lock is held across the transaction — the `remove_entry` test pattern;
  entry equality now includes the timestamp.
- `commands/log.rs` unit (`cmd_retime`): confirm flow mirrors `cmd_rm`
  (decline exits 0, `--yes` skips); bad `HH:MM` is a usage error.
- `ai` unit: `apply_ops` unchanged (existing tests already cover it);
  `stamp_added_entries` stamps exactly the trailing `k` entries (`k` = add-op
  count), stamps nothing for empty op lists, and leaves replaced rows'
  timestamps untouched.
- e2e (`tests/cli.rs`): day view shows the Time column by default;
  `show_timestamp = false` hides it; `time_format = "12h"` renders 12-hour
  cells; `write_timestamps = false` produces files without `timestamp`;
  `--time 14:30` and `retime` write `…T14:30:00Z` exactly (child runs with
  `TZ=UTC` so the local-on-target-date conversion is deterministic);
  default config writes `timestamp = "…Z"` and legacy fixture days still
  render. e2e (`tests/ai.rs`): an `ai log` write produces a file whose added
  entries carry RFC 3339 timestamps.
- Quality gates unchanged (AGENTS.md): all four `--workspace` commands plus
  the no-AI pair; the timestamp feature is non-gated (shared files only, no
  cfg attributes).

## Implementation steps

1. **`Cargo.toml` + `src/log.rs`** — enable `chrono`'s `serde` feature; add
   the `Timestamp` newtype and `LogEntry.timestamp: Option<Timestamp>` with
   roundtrip and rejection tests.
2. **`src/config.rs`** — the three keys as `Option<…>` fields plus getters
   with defaults and the `time_format` enum validation; config tests.
3. **`src/cli.rs` + `src/log.rs` + `src/commands/log.rs`** — `--time HH:MM`
   on `log` (both paths, overriding the toggle); the `retime` subcommand
   (`DateArgs` + `--yes`, match arm in `commands/mod.rs`, handler mirroring
   `cmd_rm`) and `log::set_entry_timestamp`; Time column in `render_day`
   (column after Servings when `show_timestamp`); the local-conversion and
   `time_format` formatting live on `log::Timestamp::format` (display
   boundary in the type, matching how `time_cell` renders in the day view);
   tests.
4. **`src/ai/`** — `stamp_added_entries` called before `write_day_checked`
   in the `ai log` write path; tests.
5. **Docs** — README Configuration section (the three keys), the `log`
   usage line (`--time`), and the new `retime` usage line; AGENTS.md
   model note (entries are title, servings, six macros, and an optional
   timestamp).

## Open questions

- **Display timezone**: local by default (proposal). UTC-only display would
  be more literal ("stored in UTC") but reads wrong for evening logs in
  negative offsets; a future `timezone` key is the escape hatch.
- **Format growth**: seconds, ISO 8601, or a strftime-style pattern if
  `"24h"`/`"12h"` prove limiting.
- **`ai log` diff preview**: added rows could show the timestamp in the
  proposal diff; deferred — the diff format is stable width-independent
  lines and the value is "now", which the one-line write confirmation
  covers.
- **Cross-timezone logging**: `--time` / `retime` are local-on-target-date
  with no escape hatch for a meal eaten in another timezone — a future
  `--utc` flag or explicit-offset form.
- **`ai log` `set-time` op**: if AI day editing ever wants time adjustment,
  an op where the model supplies only the `HH:MM` string and intake
  validates and converts preserves the "intake computes" invariant; deferred
  — plain `retime` covers the workflow, and the model should not be a
  required path for scalar metadata.
- **Backfill**: `retime` is the one-off backfill (fills a legacy `None`
  entry); a bulk backfill of old entries (e.g. from file mtime) stays
  future work.
- **Column position**: fixed after `Servings` for v1; a `show_columns`-style
  ordering for the metadata column is deferred.
