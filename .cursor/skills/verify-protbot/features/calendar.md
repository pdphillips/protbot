# Calendar

## Sub-features

- `calendar_events` reads every configured feed and returns up to 100 events. No arguments.
- Feeds come from two places. `PROTBOT_CALENDAR_ICS` is a comma-separated list of `https://` URLs in the environment. `calendar_feeds` in `settings.toml` is a list of Pass item refs shaped `vault/item`, not URLs. The URL is read at call time from that item's JSON by `pass-cli item view`.
- Each URL is fetched with `curl --config -`. The config on stdin is `silent`, `show-error`, `fail`, `max-time = 20`, and `url = "<url>"`, so the URL is not a curl argument.
- Returned fields per event are `uid`, `summary`, `start`, `end`, and `location`, parsed from `VEVENT` in the ICS body. The response object also has `failed_feeds`, a count of feeds that could not be read. One bad feed does not drop the others.

## How to get to it (user POV)

The host asks what is on the bot's calendar. That is `tools/call` named `calendar_events` with `{}`. With the default seed, nothing is configured, so the host gets no events and `failed_feeds` of 0. That empty result is the real answer for a fresh data directory. It is not a skipped call.

To show an event, the host either exports `PROTBOT_CALENDAR_ICS` before launch or sets `calendar_feeds` to a Pass ref such as `Bot/Calendar` through `settings_update`. The Pass item's note or URL field must contain an `https://` link. Protbot prefers a link whose text contains `ics` or `calendar`.

## Driving it with tmux

```bash
.cursor/skills/verify-protbot/scripts/drive.sh calendar
```

The line it types:

```json
{"jsonrpc":"2.0","id":41,"method":"tools/call","params":{"name":"calendar_events","arguments":{}}}
```

On the default launch (no `PROTBOT_CALENDAR_ICS`, `calendar_feeds = []`):

- The tool text is `{"failed_feeds":0,"events":[]}`.
- `isError` is false.
- Stderr contains `protbot tool=calendar_events id=-`.
- `curl` is not started, so `.verify/data/cli.log` gains no `--config` line from this call.

Evidence: `.verify/evidence/calendar.json`, `calendar-audit.txt`, `calendar.txt`.

A live feed is a different launch. Export `PROTBOT_CALENDAR_ICS` to one `https://` ICS URL before `launch.sh`. `drive.sh calendar` then checks only that the JSON has `failed_feeds` and `events`. Read the event summaries in the tool text yourself. Do not copy the feed URL into evidence or logs. Errors replace URLs with `[url]`.

There is no local HTTPS fixture in this skill. An empty calendar is what a fresh data directory can prove without a network feed.

## Gotchas

- A `calendar_feeds` entry that starts with `http://` or `https://` is rejected. The settings file stores `vault/item` only. `validate_feed_ref` requires exactly one slash, no `..`, and no empty side.
- `PROTBOT_CALENDAR_ICS` URLs must be `https://`. The variable is unset by `launch.sh` so a value left in the shell does not leak into the verify server.
- Feed bodies are cached for 30 seconds. A successful write anywhere clears that cache.
- `failed_feeds` increments when Pass has no `https://` string, curl exits non-zero, or the URL fails validation. The tool still returns 200-style MCP success (`isError` false) with the count. Check the count. Do not treat a zero-length `events` array alone as a healthy feed when `failed_feeds` is non-zero.
- The curl child is scrubbed of the Pass token, the bridge password, and `PROTBOT_CALENDAR_ICS` before `exec`. The URL travels on stdin.
- Event text is unfolded (lines that start with space or tab continue the previous line) and ICS escapes are decoded. Properties other than `UID`, `SUMMARY`, `DTSTART`, `DTEND`, and `LOCATION` are ignored.
