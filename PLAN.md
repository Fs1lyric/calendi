# Lyric Cal — plan

A calendar (Google Calendar-style month/week/day grid) and booking links (Calendly/Cal.com-style)
in one light desktop app. Local-first, no Google, no accounts.

## Decisions (from Lyric, 2026-10-15)
- Desktop app: plain Rust, **no Tauri** (Lyric, 2026-10-15, session 3): egui/eframe 0.36 with the glow
  renderer, SQLite, pastel Lyric theme. No web frontend, no Node.
- Booking runs on this PC, published through a Cloudflare Tunnel. Bookable only while the PC is on.
- v1 features: month/week/day views with click-to-add/edit; booking links (event types, weekly
  hours, public booking page); recurring events + desktop reminders. No email confirmations in v1.
- Cal.diy source may be reused. It is MIT (`~/claude/cal.diy/LICENSE`): keep its copyright notice in
  `THIRD_PARTY_LICENSES` for anything ported. Do NOT copy from the main cal.com repo (AGPL/EE parts).

## Shape
Two binaries over one SQLite file (`~/.local/share/lyric-cal/cal.db`, WAL mode):

1. **`lyric-cal`**: the egui desktop app (`crates/app`). Calendar views, event editor, event types, availability
   settings, list of bookings.
2. **`lyric-cal-booker`**: a tiny always-on daemon (axum, systemd user service, ~10 MB RAM). It
   serves the public booking page and JSON API, writes bookings into the same DB, and fires
   reminder notifications (so they work with the window closed). `cloudflared` points at it.

Splitting them means closing the app window never takes the booking page offline.

## Data model
- `events`: id, title, notes, start, end, all_day, tz, rrule (nullable), color, reminder_minutes
- `event_exceptions`: event_id, original_start, cancelled | override fields (edit/delete one occurrence)
- `event_types`: slug, title, duration_min, buffer_before/after, min_notice_min, max_days_ahead, active
- `availability`: weekday, start_minute, end_minute (weekly hours) + `date_overrides`
- `bookings`: event_type_id, start, end, guest_name, guest_email, guest_note, status, created_at
  → each booking also creates a row in `events`, so it appears in the grid.

## What to port from Cal.diy
- `packages/features/schedules/lib/slots.ts` + `date-ranges.ts`: slot generation from weekly
  hours, overrides, busy times, buffers, minimum notice. Port to Rust and **port their test files
  first** (`slots.test.ts`, `date-ranges.test.ts`) as Rust tests, so the port is checked against
  Cal.diy's own cases.
- Booking page flow and layout (pick a day → pick a slot → name/email → confirmed), re-styled in the
  Lyric theme. No Next.js/React, just a single static page plus the JSON API.
- Recurrence: `rrule` crate (RFC 5545), so events can later export to .ics or CalDAV.

## Reuse from Odyssey Workspace
Dropped with Tauri: the views were written fresh in Rust (`crates/app`).

## Build order
1. Scaffold the Tauri app plus the shared `core` crate (DB, migrations, models). Tests: migrations, CRUD.
2. Calendar views + event editor + recurrence/exceptions. Tests: rrule expansion, exception handling.
3. Slot engine port, with Cal.diy's tests ported first.
4. Booker daemon + public booking page + double-booking guard (transaction checks overlap).
5. Reminders (daemon → `notify-send`/freedesktop notifications), systemd user unit, `.desktop` entry + icon.
6. Cloudflare tunnel. A quick tunnel (`cloudflared tunnel --url`) needs no account but gives a
   random URL on every restart; a stable link needs a free Cloudflare account + a domain.

## Verification
- `cargo test` (core + slot engine incl. ported Cal.diy cases), `tsc --noEmit`, lint.
- Run the app and screenshot each view (month/week/day, editor, settings).
- End to end: open the booking page via the tunnel URL from a phone, book a slot, confirm it appears
  in the grid, that the slot disappears for the next visitor, and that the reminder fires.

## Open question
- A stable booking URL needs a free Cloudflare account + a domain (~$10/yr). Without it the link
  changes whenever the PC restarts.

## Progress

### Session 1 (2026-10-15): core crate + slot engine (build steps 1 and 3, Rust side)
Done, all verified: `cargo test` 63 passed, `cargo clippy --all-targets` 0 warnings, `cargo fmt --check` clean.
- Cargo workspace at the repo root (`crates/core` only so far). Rust lives in `~/.cargo/bin`; the
  mise `cargo` shim has no version set, so run `export PATH=~/.cargo/bin:$PATH` first.
- `crates/core/src/db.rs`: schema v1 (all tables in "Data model" + `settings`; `events.booking_id`
  links a booking's grid event). Migrations via `PRAGMA user_version`, WAL, `busy_timeout` 5s.
  Timed values are stored as UTC RFC3339 `...Z` (text-sortable); all-day as `YYYY-MM-DD`, exclusive
  end. CRUD for events, event types (slug is unique and URL-safe; deleting a booked type deactivates
  it), weekly hours and overrides (replaced whole), settings. `events_between` returns overlaps plus
  every series started by `to`. The caller expands those (step 2). `Db::default_path()` honours
  `$LYRIC_CAL_DB`.
- `crates/core/src/slots.rs`: port of Cal.diy `date-ranges.ts` + `slots.ts`. `tests/caldiy_slots.rs`
  (35) and `tests/caldiy_date_ranges.rs` (17) are their test files ported case for case. Left out on
  purpose: OOO, travel schedules, team `intersect`, perf case. `bookable_slots()` is the entry point
  for the booker: hours → ranges, busy blocks padded by the buffers and subtracted, then slots, with
  min notice and max days ahead.
- `THIRD_PARTY_LICENSES` carries Cal.diy's MIT notice.

### Session 2 (2026-10-15): recurrence (build step 2, core side)
Done, all verified: `cargo test` 78 passed, clippy 0 warnings, fmt clean.
- `rrule` 0.14 (workspace dep, shares chrono-tz 0.10). `crates/core/src/recur.rs`:
  `check_rule` replaces the hand-written `check_rrule` (now a real parse against the event's start;
  HOURLY or more often and `COUNT=0` rejected). A date-only `UNTIL=YYYYMMDD` means through that day
  (UTC for all-day, the event's zone for timed). `expand(events, exceptions, from, to)` returns
  sorted `Occurrence`s. Timed series repeat at the event's wall-clock time in `tz` (DST-safe), all-day
  series step by date, and length is kept exactly. Each occurrence carries `original_start`, which
  names it for exceptions. A moved occurrence shows wherever it now falls, inside or outside the window.
- `db.rs`: `Exception` model, `set_exception` (upsert; needs a series, an `original_start` that the
  rule really gives, and start+end together), `delete_exception`, `exceptions_for(ids)`,
  `occurrences_between(from, to)` (the app and booker call this). `update_event` drops a series'
  exceptions when its start, all-day flag, tz or rule changes. A rename keeps them.
- Tests: `tests/recur.rs` (15).

### Session 3 (2026-10-15): desktop app (build step 1, rest + step 2 UI)
Done: `cargo test` 98 passed (20 new in the app crate), clippy 0 warnings, fmt clean. Ran the app on a
seeded DB (`LYRIC_CAL_DB=…`) and screenshotted week, month, day and the editor.
- `crates/app` (binary `lyric-cal`): `grid.rs` (view days, stepping, titles, local↔UTC, column
  packing, pixel↔minute; tested), `form.rs` (editor data, repeat presets + ends ↔ RRULE,
  this-one/whole-series saves; tested), `theme.rs` (palette), `views.rs` (month grid, week/day time
  grid with all-day lane, click-to-add, drag-to-create, drag-to-move, now line), `editor.rs` (modal
  with a home-made date popup, so no second date library), `main.rs` (top bar that wraps when narrow,
  keys t/m/w/d/n/←/→, reload every 30 s so booker writes show up).
- Flags: `--view month|week|day`, `--new` (opens the editor; good for a quick-add keybind).
- Dragging a repeating event moves only that occurrence (an exception). The editor asks "only this
  one / every occurrence" for saves and deletes on a series.
- Not hand-tested yet: clicking, drag-create, drag-move and saving through the editor. Rendering was
  checked; input was not, because simulated keys hit other windows. Click through these once by hand.
- Screenshots: launch it and `grim -g` its geometry from `hyprctl clients -j`. Do NOT send `wtype` or
  toggle maximize, since focus can land on another window.

### Session 4 (2026-10-15): booking screen + installed
Done: `cargo test` 106 passed, clippy 0 warnings, fmt clean. Screenshotted the booking screen on a seeded DB.
- core: `Booking` model, `bookings_from(from)` (with the type's title, cancelled included),
  `cancel_booking(id)` (marks it cancelled and deletes its grid event). Tests in `tests/store.rs`.
- app: `setup.rs` (hh:mm parsing, `slugify`, weekly hours and date overrides forms; tested) and
  `booking.rs` (the "Booking" screen: event types list + modal editor with an open/off toggle, weekly hours
  with several spans a day, days off / special hours, upcoming bookings with Cancel). Opened from the top bar,
  key `b` or `--booking`. A day off is stored as an override with start = end = 0.
- Installed with `./install.sh` to `~/.local/bin/lyric-cal` plus the launcher entry.
- Still not hand-tested: clicking through either screen (input is never simulated here).

### Session 5 (2026-10-15): booker (build step 4)
Done: `cargo test` 116 passed, clippy 0 warnings, fmt clean. Installed and running.
- core (`db.rs`): `Error::NotFound` / `Error::Taken`, `BookRequest`, `busy_between` (timed occurrences; all-day
  = free; a booking's event grows by its own type's gaps, as Cal.com does), `slots_for`, and `book`: a
  `BEGIN IMMEDIATE` transaction that re-runs the slot engine for the days around the slot, then inserts the
  booking and its grid event ("<type> with <guest>", reminder 10 min). `tests/book.rs` (6) includes 8 threads
  racing for one slot → exactly one booking.
- `crates/booker` (axum 0.8, current-thread tokio, ~8 MB RSS): `GET /{slug}` (page; 404 if not open),
  `GET /api/types/{slug}` (public fields only), `GET /api/slots?type&from&to&tz` (≤ 62 days, slots in the
  guest's zone), `POST /api/book` (201 / 400 / 404 / 409; no ids returned), `GET /health`. Organizer zone =
  `tz` setting, else the system's. Binds 127.0.0.1:8787 by default.
- `src/page.html`: one static page (month → times → name/email/note → confirmed), guest zone from the browser,
  text only via `textContent`. Driven end to end in headless Chromium over CDP (book, then a second guest on
  the same slot gets the 409 message); screenshots at 1000 px and 390 px.
- `install.sh` now also installs `lyric-cal-booker` and `packaging/lyric-cal-booker.service` (systemd user unit,
  enabled + restarted on install).

### Next session: start here
1. Click-test the desktop app's calendar and booking screens by hand.
2. **Step 5, reminders**: the booker fires `notify-send` for events with `reminder_minutes` (a tokio interval
   over `occurrences_between`, remembering what it already fired). The unit and `.desktop` entry exist.
3. **Step 6, tunnel**: `pacman -S cloudflared`, then `cloudflared tunnel --url http://127.0.0.1:8787` for a quick
   URL, or a named tunnel for a stable one (needs a Cloudflare account + domain; Lyric's call).
