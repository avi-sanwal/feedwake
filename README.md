# FeedWake

FeedWake is a small Rust one-shot CLI for India-market feed alerts. It is meant
to run from cron, filter locally, and wake OpenClaw only when a relevant item is
found.

## Quick Start

Build the binary:

```bash
cargo build --release
```

Create a config:

```bash
mkdir -p ~/.config/feedwake
cp config/feedwake.example.toml ~/.config/feedwake/config.toml
```

Set the OpenClaw hook token in the environment:

```bash
export OPENCLAW_HOOK_TOKEN="..."
```

Install the OpenClaw webhook configuration and a user crontab entry:

```bash
target/release/feedwake openclaw install
```

Run a dry scan without writing state:

```bash
target/release/feedwake scan --dry-run
```

Run with an explicit config:

```bash
feedwake scan --config /etc/feedwake.toml
```

The installer discovers `~/.openclaw/openclaw.json`, or the parent directory of
`OPENCLAW_CONFIG_PATH` when that variable is set. It enables OpenClaw hooks,
adds or updates a `/hooks/feed-wake` mapped hook for FeedWake RSS batches,
updates or creates the FeedWake config at `$HOME/.config/feedwake/config.toml`,
and reconciles FeedWake's OpenClaw URL and token environment variable from
OpenClaw's `gateway.port`, `hooks.path`, and `hooks.token`. If no hook token is
configured, it generates a secure random token, writes it to `~/.openclaw/.env`,
and points both OpenClaw and FeedWake at the same environment variable. It also
installs a managed current-user crontab block that runs every 5 minutes by
default. Pass `--openclaw-config-dir`, `--frequency-minutes`, `--config`, or
`--feedwake-bin` to override those defaults.

The managed crontab entry sources `~/.openclaw/.env` before running FeedWake, so
that file is a good place to keep `OPENCLAW_HOOK_TOKEN` for unattended cron runs.
It runs `feedwake --verbose scan` and appends timestamped output to
`$XDG_STATE_HOME/feedwake/feedwake.log`, or
`$HOME/.local/state/feedwake/feedwake.log` when `XDG_STATE_HOME` is not set.
Before each scan, the cron script rotates that file when it reaches 1 MiB and
keeps 5 rotated logs by default. Override those defaults with `--log-file`,
`--log-max-bytes`, and `--log-rotate-count`. Rerun
`feedwake openclaw install` after upgrading FeedWake to refresh an older managed
crontab entry.

Manual cron example:

```cron
*/5 * * * * /bin/sh -c 'mkdir -p "$HOME/.local/state/feedwake"; set -a; . "$HOME/.openclaw/.env" 2>/dev/null; set +a; exec /usr/local/bin/feedwake --verbose scan --config /etc/feedwake.toml >> "$HOME/.local/state/feedwake/feedwake.log" 2>&1'
```

If `--config` is omitted, FeedWake searches:

1. `/etc/feedwake.toml`
2. `$HOME/.config/feedwake/config.toml`
3. `$HOME/.feedwake.toml`

The default OpenClaw route is local loopback delivery to `/hooks/feed-wake`.
Keep the hook token in the environment using the configured `token_env`.
FeedWake sends one webhook call per scan with up to `openclaw.max_articles_per_wake`
matched articles, defaulting to 3. Extra pending articles remain in SQLite and
are delivered by a later cron run.

## Cron

FeedWake is intentionally not a daemon in v1. Let cron own scheduling:

```cron
*/5 * * * * /bin/sh -c 'mkdir -p "$HOME/.local/state/feedwake"; set -a; . "$HOME/.openclaw/.env" 2>/dev/null; set +a; exec /usr/local/bin/feedwake --verbose scan --config /etc/feedwake.toml >> "$HOME/.local/state/feedwake/feedwake.log" 2>&1'
```

The installer writes the full managed cron entry, including log rotation. Check
the active entry with:

```bash
crontab -l
```

The most common runtime checks are:

```bash
tail -f ~/.local/state/feedwake/feedwake.log
ls -lh ~/.local/state/feedwake/feedwake.log*
```

If cron was installed by an older FeedWake release and still has no log
redirect, cron output may only be available through the host cron mail or system
logs. Reinstalling the OpenClaw integration replaces the managed block with the
logged entry.

State is stored in SQLite. If `scan.state_db` is not configured, FeedWake tries
`/var/lib/feedwake/feedwake.db` when writable and otherwise uses
`$HOME/.local/share/feedwake/feedwake.db`.

## Architecture

FeedWake has four main pieces:

- **Feed scanner**: fetches RSS/Atom feeds with bounded timeouts, response-size
  limits, and conditional GET support through `ETag` / `Last-Modified`.
- **Source-aware filters**: applies a filter profile per source instead of one
  global rule set.
- **SQLite state**: tracks seen URLs, feed cache headers, and pending delivery
  events.
- **OpenClaw delivery**: posts compact local RSS batches to
  `http://127.0.0.1:18789/hooks/feed-wake` using a bearer token from the
  environment. Each payload includes a readable `text` summary and structured
  article details including title, URL, source feed URL, description, matched
  rule, matched entity, and publication time when present.

The scan flow is:

```text
cron -> feedwake scan -> fetch feeds -> filter locally -> SQLite outbox -> /hooks/feed-wake
```

## Filtering Profiles

- `exchange_watchlist`: NSE/BSE feeds match configured symbols, names, aliases,
  or ISINs across the title, description, feed subject/category, URL, and
  document filename, plus the optional category allowlist.
- `authority_passthrough`: SEBI/RBI feeds wake on every new item.
- `media_high_precision`: media feeds require a watchlist entity and a
  market-moving keyword, while excluding broad noisy topics.

See [config/feedwake.example.toml](config/feedwake.example.toml) for a starter
configuration.

## Source Notes

- NSE announcements use `https://nsearchives.nseindia.com/content/RSS/Online_announcements.xml`.
- SEBI uses `https://www.sebi.gov.in/sebirss.xml`.
- RBI press releases use `https://rbi.org.in/pressreleases_rss.xml`; RBI also publishes notification, speech, tender, and publication feeds.
- BSE is supported through the `bse` source type and `exchange_watchlist` profile. Copy the current Corporate Announcements RSS URL from BSE's RSS page into the config; FeedWake does not scrape that page at runtime. BSE feeds use a browser-compatible default User-Agent unless `user_agent` is set for the feed.

After adding or changing the BSE feed URL, validate it before enabling cron:

```bash
feedwake scan --config ~/.config/feedwake/config.toml --dry-run --verbose
```

## Tests

Run the full unit/integration test suite:

```bash
cargo test
```

Run lint checks:

```bash
cargo clippy --all-targets -- -D warnings
```

Current tests cover:

- watchlist matching, false-positive boundaries, and source-specific filters
- RSS parsing and item limits
- SQLite dedupe and outbox delivery state
- local HTTP delivery, bearer auth, and non-2xx failure handling

## Release

Releases are created manually from GitHub Actions. The release workflow reads the
latest `vX.Y.Z` tag, increments the selected version part, updates `Cargo.toml`
and `Cargo.lock`, builds the release binary, commits the version bump, tags it,
and creates a GitHub release with the binary archive.
