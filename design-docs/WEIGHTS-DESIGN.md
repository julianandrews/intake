# Design Doc: Weight Logging for intake

Status: Proposed

## Overview

A `weight` command records body weight measurements, timestamped like log
entries. Weights live in the existing day log files — each day's file gains a
`weights` list — so the day view can show the day's weigh-ins without reading
anywhere new. A weight is stored canonically in kilograms (an exact decimal,
0.001 kg = 1 g precision), with the input and display unit configurable
(`weight_unit`, default `"kg"`, `"lbs"` also accepted; 1 lb = 0.45359237 kg
exactly, so conversion is exact decimal arithmetic — no floats, per
convention).

Timestamps follow the entry-timestamp semantics: the default stamp is the
time the weight was recorded (`Timestamp::now()`, captured once per
invocation), and an explicit `--time HH:MM` replaces it with a time of day on
the target date (local, converted to UTC by intake). Unlike log entries,
weight timestamps are not optional — the timestamp is the point of the
feature — so `write_timestamps = false` does not apply to weights.

## Command

```
intake weight <value> [--time HH:MM] [--date D | --days-ago N]
intake weight rm <n> [--yes] [--date D | --days-ago N]
```

- `weight` is a noun, matching the closest existing analog `exercise`
  (`intake exercise 300`); `intake weight 75.5` reads the same way. It is one
  command with an optional positional and a nested `rm` subcommand
  (`subcommand_precedence_over_arg`), so the record form stays flat while
  `weight rm` has room to be its own shape; `weight` with neither a value nor
  a subcommand is a usage error.
- `<value>` is a non-negative decimal parsed by a new `Kilograms` amount type
  (see Storage). Zero and negative values are rejected.
- The shared `DateArgs` (`--date` / `--days-ago`) are flattened like `log`
  and `exercise`: subcommand args win over root args, both supplied together
  is an error, default is today. `weight` is a date-targeting command.
- `--time HH:MM` is parsed strictly (chrono `%H:%M`, 24-hour — `time_format`
  controls display only) and interpreted as local time on the target date,
  exactly like `log --time`: an ambiguous or nonexistent local time (DST gap
  or fall-back) is an error, never a guessed instant. It cannot land the
  weight on a different day than `--date` — by construction it is on the
  target date.
- Output: a confirmation line like `Recorded 75.5 kg for 2026-09-19` (unit
  per config), then the day view.
- `weight rm <n>` deletes the n-th weigh-in of the target day (1-based, as
  numbered in the day view's Weights section), mirroring `rm` for entries:
  loads the day, confirms (`Remove weight 1 (75.5 kg) from 2026-09-19?`,
  `--yes` skips), then shows the updated day view. There is no `weight
  retime` — re-stamping a weigh-in is `rm` then `weight --time` again.

## Storage & schema

```rust
// log.rs
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WeightEntry {
    pub kg: Kilograms,
    pub timestamp: Timestamp,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DayLog {
    pub entries: Vec<LogEntry>,
    pub exercise_calories: Calories,
    #[serde(default)]
    pub weights: Vec<WeightEntry>,
}
```

```toml
exercise_calories = 0

[[entries]]
# ... unchanged ...

[[weights]]
kg = 75.5
timestamp = "2026-09-19T08:00:00Z"
```

- `weights` is a **list**, not a scalar like `exercise_calories`: multiple
  weigh-ins per day (morning/evening) are expected, each with its own
  timestamp. `weight` appends.
- `#[serde(default)]` on the field makes the change **backward compatible on
  read**: existing day files without `weights` deserialize as an empty list
  and keep working. `deny_unknown_fields` is unchanged, which makes this a
  **forward-breaking schema change** in the established sense (see
  AI-DESIGN.md "Change review & safety"): a day file containing `weights`
  fails loudly on an older binary instead of being silently rewritten without
  it. That policy is intentional, matching the `timestamp` field precedent.
- `WeightEntry.timestamp` is non-`Option`: unlike log entries, weights are
  always stamped. There are no legacy weight files to be backward compatible
  with, and an unstamped weight is useless for tracking. An explicit
  `--time` overrides the default `now`; nothing else can suppress it
  (`write_timestamps` does not apply).
- Strict parsing holds: a malformed `kg` value or timestamp string is a load
  error naming the file — never a silent default.
- The write path is the existing `update_day` locked read-modify-write, via a
  new `log::append_weight(log_dir, date, entry)` mirroring `append_entry`:
  lock the directory, read, push, `write_day_locked` (atomic rename + sync).
- Removal mirrors append: a new `log::remove_weight(log_dir, date, index,
  expected)` following the `remove_entry` pattern — locked read-modify-write
  with expected-entry equality under the lock, so a concurrent change aborts
  instead of removing a different weigh-in. The day-file-deletion rule is
  shared with `remove_entry`: the day file is deleted when its last entry and
  its last weight are both gone (and exercise calories are zero); a day
  holding only a weight must persist.
- The entry-level `rm` / `retime` operate on entries and never touch weights;
  `weight rm` is the only weight correction surface. `summary` and the AI
  context (`ai log`) ignore weights; the AI model never sees, emits, or
  manipulates them.

## Units

- **Canonical storage is kilograms.** A new `Kilograms` amount type in
  `amount.rs`, via the existing `decimal_type!` macro (non-negative, 0.001
  precision). 1 kg = 1000 g; 0.001 kg resolution is 1 g — plenty for body
  weight.
- Input is interpreted in the configured unit (`weight_unit`), converted to
  kg at the **input boundary**, rounded to storage precision (half away from
  zero, the existing convention). Display converts back at the **display
  boundary** (0.1 precision) — rounding happens only at display, matching the
  rest of the codebase.
- The conversion factor is exact: 1 lb = 0.45359237 kg (international
  avoirdupois pound, by definition). Conversion helpers (`from_lbs` /
  `to_lbs` on `Kilograms`, or a small `weight` module) use checked `Decimal`
  arithmetic — overflow errors loudly, never wraps. Round-trip behavior:
  150 lbs → 68.039 kg stored → 150.0 lbs displayed. The 0.001 kg storage
  rounding is invisible at 0.1 display precision.
- Round-trip guarantee: storage rounding error is at most 0.0005 kg ≈
  0.0011 lbs, ~100× below the 0.1 display quantum, so any input with 0.1-unit
  precision displays back exactly — e.g. 233.8 lbs → 106.049896106 kg exact →
  106.050 kg stored → 233.800229 lbs → 233.8 lbs displayed. An input finer
  than 0.1 units could in principle sit within 0.0011 lbs of a display
  rounding boundary and flip it; that is accepted.
- Storing the unit per entry is not needed: the file holds kg, so a later
  `weight_unit` change only affects input interpretation and display, never
  the stored values.

## Configuration

```toml
weight_unit = "kg"   # "kg" (default) | "lbs"
```

- A `WeightUnit` enum (`"kg"` / `"lbs"`, serde lowercase) plus a
  `Config::weight_unit()` getter defaulting to kg — the `time_format` pattern
  (config.rs): absent key → default, present-but-invalid → friendly error at
  config load.
- Config-only, like the targets and time display keys: no env var, no CLI
  flag.

## Display

The day view (`render_day`) gains a footer section when the day has weights,
placed after the summary lines:

```
Weights:
  1. 75.5 kg (08:00)
  2. 75.4 kg (20:00)
```

- Each weigh-in renders its value in the configured unit (0.1 precision) and
  its timestamp in local time per `time_format` — the same display boundary
  conversion as the Time column. The `Weights:` heading is bold magenta
  (matching the summary labels), values bold cyan, numbers and times dim.
- The section appears only when `weights` is non-empty. It is metadata, not a
  macro: no color targets, no table row, never a `show_columns`
  member — the Time column precedent.
- Each weigh-in is numbered 1-based; the numbers are what `weight rm <n>`
  targets.
- No other views change: `summary` aggregates days and `food` views have
  nothing to do with weights; a history/trend view is deferred.

## Testing

- `amount.rs` unit: `Kilograms` parse and roundtrip (rejects negative and
  zero); lbs↔kg conversion exactness (150 lbs → 68.039 kg at storage
  precision; display round-trip at 0.1).
- `log.rs` unit: `WeightEntry` TOML roundtrip; legacy day files (no
  `weights` key) load with an empty list; malformed `kg` / `timestamp`
  rejected; `append_weight` round-trips and waits on the directory lock (the
  `append_entry` lock test pattern); `remove_weight` removes, out-of-range /
  no-day errors, stale-entry abort, and keeps the day file when weights
  remain — the `remove_entry` test pattern; `remove_entry` and `remove_weight`
  both delete the day file only when entries, weights, and exercise are all
  gone.
- `config.rs` unit: `weight_unit` default is kg; `"lbs"` parses; `"bogus"`
  errors.
- `commands/log.rs` unit: `cmd_weight` stamps `now` by default and the
  `--time` local-on-target-date value when given; `cmd_weight_rm` confirms,
  `--yes` skips, and re-renders the day; renders the footer line with unit,
  time, and 1-based numbers per config; renders nothing when no weights.
- e2e (`tests/cli.rs`): `weight 75.5` appends a `[[weights]]` entry with an
  RFC 3339 timestamp; `--time`/`--date`/`--days-ago` compose (child runs with
  `TZ=UTC` so local-on-target-date conversion is deterministic); `weight_unit
  = "lbs"` interprets input as pounds, stores kg, and displays pounds;
  `weight rm` removes and re-renders.
- Quality gates unchanged (AGENTS.md): all four `--workspace` commands plus
  the no-AI pair. The weight feature is non-gated (shared files only, no cfg
  attributes).

## Implementation steps

1. **`src/amount.rs`** — the `Kilograms` type via `decimal_type!` plus the
   lbs conversion helpers; tests.
2. **`src/log.rs`** — `WeightEntry`, `DayLog.weights` (`#[serde(default)]`),
   `append_weight`, `remove_weight`, and the shared day-file-deletion rule
   change; tests.
3. **`src/config.rs`** — `WeightUnit` enum + `weight_unit` key and getter;
   tests.
4. **`src/cli.rs` + `src/commands/mod.rs` + `src/commands/log.rs`** — the
   `Weight` command (optional `Kilograms` positional, `--time`, flattened
   `DateArgs`, nested `rm` via `subcommand_precedence_over_arg`), match arms
   in `run`, `cmd_weight`, `cmd_weight_rm`, and the numbered footer line in
   `render_day`; tests.
5. **Docs** — README Usage and Configuration sections (both `weight` forms,
   `weight_unit`); AGENTS.md model note (day files also carry weights).

## Deferred

Kept on purpose, out of scope for this change:

- **History/trend views**: a bare `weight` history table (date, weight,
  delta) and a weight column or trend in `summary` would make tracking
  self-contained; for now the feature is "record, correct, and see today".
  A history view would also need to decide how the `weight` command parses
  when no value is given — currently a usage error.