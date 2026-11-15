# Protbot features

The user is an MCP host. It does not open a URL. It spawns protbot, sends `initialize`, then `tools/call`. Drive a feature only after `doctor.sh` prints `worth driving`. The recipes are `drive.sh <name>` and the JSON in each file.

| Feature | File | What the host is doing |
| --- | --- | --- |
| Mail | [mail.md](mail.md) | Read the bot mailbox through Bridge, and refuse a send while writes are off |
| Pass | [pass.md](pass.md) | List the bot's Pass vaults and items |
| Drive | [drive.md](drive.md) | List `/my-files` and reject a `..` path |
| Calendar | [calendar.md](calendar.md) | Read ICS events from settings and `PROTBOT_CALENDAR_ICS` |
| Settings | [settings.md](settings.md) | See the signed-in mailbox and change the poll interval |
