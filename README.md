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

Manual cron example:

```cron
*/5 * * * * /bin/sh -c '. "$HOME/.openclaw/.env" 2>/dev/null; exec /usr/local/bin/feedwake scan --config /etc/feedwake.toml'
```

If `--config` is omitted, FeedWake searches:

1. `/etc/feedwake.toml`
2. `$HOME/.config/feedwake/config.toml`
3. `$HOME/.feedwake.toml`

The default OpenClaw route is local loopback delivery to `/hooks/wake`.
Keep the hook token in the environment using the configured `token_env`.

## Cron

FeedWake is intentionally not a daemon in v1. Let cron own scheduling:

```cron
*/5 * * * * /bin/sh -c '. "$HOME/.openclaw/.env" 2>/dev/null; exec /usr/local/bin/feedwake scan --config /etc/feedwake.toml'
```

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
- **OpenClaw delivery**: posts compact local wake events to
  `http://127.0.0.1:18789/hooks/wake` using a bearer token from the
  environment.

The scan flow is:

```text
cron -> feedwake scan -> fetch feeds -> filter locally -> SQLite outbox -> /hooks/wake
```

## Filtering Profiles

- `exchange_watchlist`: NSE/BSE feeds match configured symbols, names, aliases,
  or ISINs, plus the optional category allowlist.
- `authority_passthrough`: SEBI/RBI feeds wake on every new item.
- `media_high_precision`: media feeds require a watchlist entity and a
  market-moving keyword, while excluding broad noisy topics.

See [config/feedwake.example.toml](config/feedwake.example.toml) for a starter
configuration.

## Source Notes

- NSE announcements use `https://nsearchives.nseindia.com/content/RSS/Online_announcements.xml`.
- SEBI uses `https://www.sebi.gov.in/sebirss.xml`.
- RBI press releases use `https://rbi.org.in/pressreleases_rss.xml`; RBI also publishes notification, speech, tender, and publication feeds.
- BSE is supported through the `bse` source type and `exchange_watchlist` profile. Copy the current Corporate Announcements RSS URL from BSE's RSS page into the config.

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
