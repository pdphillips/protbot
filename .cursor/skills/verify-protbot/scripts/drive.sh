#!/usr/bin/env bash
# Drive one protbot feature over the tmux PTY and write evidence.
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
load_state

feature=${1:-}
[[ -n "$feature" ]] || die "usage: drive.sh mail|pass|drive|calendar|settings"

mkdir -p "$EVIDENCE_DIR"
kill -0 "$PROTBOT_PID" 2>/dev/null || die "protbot pid $PROTBOT_PID is not running; run launch.sh"

save_lines() {
  local dest=$1
  shift
  : >"$dest"
  local line
  for line in "$@"; do
    printf '%s\n' "$line" >>"$dest"
  done
}

require_text() {
  local label=$1
  local line=$2
  local text
  if ! text=$(tool_text "$line"); then
    die "$label failed"
  fi
  printf '%s' "$text"
}

drive_mail() {
  local list_line read_line folder_line note_line send_line
  local list_text read_text folder_text note_text send_text
  list_line=$(call_tool mail_list '{"folder":"inbox","unread_only":false}')
  list_text=$(require_text mail_list "$list_line")
  folder_line=$(call_tool mail_folders '{}')
  folder_text=$(require_text mail_folders "$folder_line")
  note_line=$(call_tool mail_notifications '{"limit":20}')
  note_text=$(require_text mail_notifications "$note_line")

  if [[ "$MAIL_BACKEND" == "fixture" ]]; then
    printf '%s' "$list_text" | python3 -c '
import json, sys
rows = json.loads(sys.stdin.read())
want = [row for row in rows if row.get("id") == "inbox:7"]
if len(want) != 1:
    raise SystemExit(f"expected inbox:7, got {rows}")
row = want[0]
if row.get("from") != "owner@proton.me" or row.get("subject") != "Verify mailbox" or row.get("unread") is not True:
    raise SystemExit(f"unexpected header {row}")
'
    read_line=$(call_tool mail_read '{"id":"inbox:7"}')
    read_text=$(require_text mail_read "$read_line")
    printf '%s' "$read_text" | grep -F 'hello from the loopback mailbox' >/dev/null \
      || die "mail_read body did not contain the seeded sentence"
    grep -F 'BODY.PEEK[]' "$IMAP_LOG" >/dev/null || die "backend log has no BODY.PEEK[] from mail_read"
    grep -F 'protbot tool=mail_read id=inbox7' "$STDERR_LOG" >/dev/null \
      || die "stderr missing mail_read audit line"
    printf '%s' "$folder_text" | python3 -c '
import json, sys
names = json.loads(sys.stdin.read())["folders"]
if "INBOX" not in names or "Sent" not in names:
    raise SystemExit(names)
'
    printf '%s' "$note_text" | python3 -c '
import json, sys
if json.loads(sys.stdin.read()) != []:
    raise SystemExit("fresh launch replayed mailbox notices")
'
  else
    printf '%s' "$list_text" | python3 -c 'import json,sys; json.loads(sys.stdin.read())'
    read_line=
    read_text=
  fi

  if [[ "$ALLOW_WRITES" == "true" || "$ALLOW_WRITES" == "1" || "$ALLOW_WRITES" == "yes" ]]; then
    send_line=
    send_text="skipped: ALLOW_WRITES is on; drive.sh does not send mail"
  else
    send_line=$(call_tool mail_send '{"to":["owner@proton.me"],"subject":"verify","body":"nope"}')
    send_text=$(tool_error_text "$send_line")
    printf '%s' "$send_text" | grep -F 'writes are disabled; set ALLOW_WRITES=true' >/dev/null \
      || die "mail_send did not refuse while writes are disabled: $send_text"
    if [[ "$MAIL_BACKEND" == "fixture" ]] && grep -F 'SMTP DATA' "$IMAP_LOG" >/dev/null; then
      die "fixture accepted SMTP DATA even though mail_send was refused"
    fi
  fi

  grep -F 'protbot tool=mail_list id=-' "$STDERR_LOG" >/dev/null || die "stderr missing mail_list audit line"

  save_lines "$EVIDENCE_DIR/mail.json" "$list_line" ${read_line:+"$read_line"} "$folder_line" "$note_line" ${send_line:+"$send_line"}
  cp "$IMAP_LOG" "$EVIDENCE_DIR/mail-backend.log"
  grep 'protbot tool=mail_' "$STDERR_LOG" >"$EVIDENCE_DIR/mail-audit.txt" || true
  assert_no_secrets "$EVIDENCE_DIR/mail.json" "$EVIDENCE_DIR/mail-backend.log" "$EVIDENCE_DIR/mail-audit.txt"
  printf 'protbot verify: drove mail\n' | tee "$EVIDENCE_DIR/mail.txt"
  printf 'protbot verify: list subject check done, read=%s, send=%s\n' \
    "$([[ -n "$read_line" ]] && printf seeded || printf skipped)" \
    "$send_text"
}

drive_pass() {
  local vault_line item_line create_line vault_text item_text create_text
  vault_line=$(call_tool pass_vaults '{}')
  vault_text=$(require_text pass_vaults "$vault_line")
  if [[ "$PASS_BACKEND" == "stand-in" ]]; then
    printf '%s' "$vault_text" | python3 -c '
import json, sys
body = json.loads(sys.stdin.read())
if body["vaults"][0]["id"] != "v1" or body["vaults"][0]["name"] != "Bot":
    raise SystemExit(body)
'
    item_line=$(call_tool pass_items '{"vault":"Bot"}')
    item_text=$(require_text pass_items "$item_line")
    printf '%s' "$item_text" | python3 -c '
import json, sys
body = json.loads(sys.stdin.read())
if body["items"][0]["id"] != "item1":
    raise SystemExit(body)
'
    grep -F 'info --output json' "$CLI_LOG" >/dev/null || die "cli log missing pass info"
    grep -F 'reason=protbot pass_session' "$CLI_LOG" >/dev/null || die "cli log missing pass session reason"
    grep -F 'vault list --output json' "$CLI_LOG" >/dev/null || die "cli log missing vault list"
    grep -F 'reason=protbot pass_vaults' "$CLI_LOG" >/dev/null || die "cli log missing pass_vaults reason"
    grep -F 'item list --vault-name Bot --output json' "$CLI_LOG" >/dev/null || die "cli log missing item list"
  else
    printf '%s' "$vault_text" | python3 -c 'import json,sys; json.loads(sys.stdin.read())'
    item_line=
  fi

  if [[ "$ALLOW_WRITES" == "true" || "$ALLOW_WRITES" == "1" || "$ALLOW_WRITES" == "yes" ]]; then
    create_line=
    create_text="skipped: ALLOW_WRITES is on; drive.sh does not create a login"
  else
    create_line=$(call_tool pass_item_create_login '{"vault":"Bot","title":"Verify login","password":"fixture-secret"}')
    create_text=$(tool_error_text "$create_line")
    printf '%s' "$create_text" | grep -F 'writes are disabled; set ALLOW_WRITES=true' >/dev/null \
      || die "pass create did not refuse while writes are disabled: $create_text"
    if [[ -e "$VERIFY_DIR/data/pass/last-create.json" ]]; then
      die "pass stand-in stored a login even though the create was refused"
    fi
    if grep -F 'fixture-secret' "$CLI_LOG" >/dev/null; then
      die "pass cli log contains the login password"
    fi
  fi

  grep -F 'protbot tool=pass_vaults id=-' "$STDERR_LOG" >/dev/null || die "stderr missing pass_vaults audit line"
  save_lines "$EVIDENCE_DIR/pass.json" "$vault_line" ${item_line:+"$item_line"} ${create_line:+"$create_line"}
  cp "$CLI_LOG" "$EVIDENCE_DIR/pass-cli.log"
  grep 'protbot tool=pass_' "$STDERR_LOG" >"$EVIDENCE_DIR/pass-audit.txt" || true
  assert_no_secrets "$EVIDENCE_DIR/pass.json" "$EVIDENCE_DIR/pass-cli.log" "$EVIDENCE_DIR/pass-audit.txt"
  if grep -F 'fixture-secret' "$EVIDENCE_DIR/pass.json" >/dev/null; then
    die "pass evidence contains the login password"
  fi
  printf 'protbot verify: drove pass (%s)\n' "$PASS_BACKEND" | tee "$EVIDENCE_DIR/pass.txt"
}

drive_drive() {
  local list_line bad_line upload_line list_text bad_text upload_text before
  list_line=$(call_tool drive_list '{"path":"/my-files"}')
  list_text=$(require_text drive_list "$list_line")
  if [[ "$DRIVE_BACKEND" == "stand-in" ]]; then
    printf '%s' "$list_text" | python3 -c '
import json, sys
rows = json.loads(sys.stdin.read())
if not any(row.get("name") == "notes.txt" for row in rows):
    raise SystemExit(rows)
'
    grep -F 'filesystem list --json /my-files' "$CLI_LOG" >/dev/null \
      || die "cli log missing filesystem list --json /my-files"
  else
    printf '%s' "$list_text" | python3 -c 'import json,sys; json.loads(sys.stdin.read())'
  fi

  before=$(wc -l <"$CLI_LOG" | tr -d '[:space:]')
  bad_line=$(call_tool drive_list '{"path":"/my-files/../secret"}')
  bad_text=$(tool_error_text "$bad_line")
  printf '%s' "$bad_text" | grep -F 'path traversal rejected' >/dev/null \
    || die "drive_list did not reject .. : $bad_text"
  if tail -n +"$((before + 1))" "$CLI_LOG" | grep -F '..' >/dev/null; then
    die "drive cli was started for a path that contains .."
  fi

  if [[ "$ALLOW_WRITES" == "true" || "$ALLOW_WRITES" == "1" || "$ALLOW_WRITES" == "yes" ]]; then
    upload_line=
    upload_text="skipped: ALLOW_WRITES is on; drive.sh does not upload"
  else
    upload_line=$(call_tool drive_upload '{"local_path":"'"$VERIFY_DIR"'/data/upload/note.txt","remote_path":"/my-files/note.txt"}')
    upload_text=$(tool_error_text "$upload_line")
    printf '%s' "$upload_text" | grep -F 'writes are disabled; set ALLOW_WRITES=true' >/dev/null \
      || die "drive_upload did not refuse while writes are disabled: $upload_text"
  fi

  grep -F 'protbot tool=drive_list id=-' "$STDERR_LOG" >/dev/null || die "stderr missing drive_list audit line"
  save_lines "$EVIDENCE_DIR/drive.json" "$list_line" "$bad_line" ${upload_line:+"$upload_line"}
  cp "$CLI_LOG" "$EVIDENCE_DIR/drive-cli.log"
  grep 'protbot tool=drive_' "$STDERR_LOG" >"$EVIDENCE_DIR/drive-audit.txt" || true
  assert_no_secrets "$EVIDENCE_DIR/drive.json" "$EVIDENCE_DIR/drive-cli.log" "$EVIDENCE_DIR/drive-audit.txt"
  printf 'protbot verify: drove drive (%s)\n' "$DRIVE_BACKEND" | tee "$EVIDENCE_DIR/drive.txt"
  printf 'protbot verify: traversal=%s upload=%s\n' "$bad_text" "$upload_text"
}

drive_calendar() {
  local line text
  line=$(call_tool calendar_events '{}')
  text=$(require_text calendar_events "$line")
  printf '%s' "$text" | python3 -c '
import json, sys
body = json.loads(sys.stdin.read())
if "failed_feeds" not in body or "events" not in body:
    raise SystemExit(body)
'
  if [[ "$MAIL_BACKEND" == "fixture" ]] && grep -q 'calendar_feeds = \[\]' "$SETTINGS_PATH"; then
    has_ics=0
    if tr '\0' '\n' <"/proc/${PROTBOT_PID}/environ" | grep -q '^PROTBOT_CALENDAR_ICS=.\+'; then
      has_ics=1
    fi
    if [[ "$has_ics" == 0 ]]; then
      printf '%s' "$text" | python3 -c '
import json, sys
body = json.loads(sys.stdin.read())
if body["failed_feeds"] != 0 or body["events"] != []:
    raise SystemExit(body)
'
    fi
  fi
  grep -F 'protbot tool=calendar_events id=-' "$STDERR_LOG" >/dev/null \
    || die "stderr missing calendar_events audit line"
  save_lines "$EVIDENCE_DIR/calendar.json" "$line"
  grep 'protbot tool=calendar_events' "$STDERR_LOG" >"$EVIDENCE_DIR/calendar-audit.txt" || true
  assert_no_secrets "$EVIDENCE_DIR/calendar.json" "$EVIDENCE_DIR/calendar-audit.txt"
  printf 'protbot verify: drove calendar\n' | tee "$EVIDENCE_DIR/calendar.txt"
}

drive_settings() {
  local who_line get_line upd_line who_text upd_text
  who_line=$(call_tool whoami '{}')
  who_text=$(require_text whoami "$who_line")
  printf '%s' "$who_text" | python3 -c '
import json, sys
body = json.loads(sys.stdin.read())
if body.get("bridge") != "127.0.0.1":
    raise SystemExit(body)
if "@" not in body.get("address", ""):
    raise SystemExit(body)
' 
  if [[ "$MAIL_BACKEND" == "fixture" ]]; then
    printf '%s' "$who_text" | python3 -c '
import json, sys, os
body = json.loads(sys.stdin.read())
want = os.environ["BRIDGE_USER"]
if body.get("address") != want:
    raise SystemExit(body)
' 
  fi
  get_line=$(call_tool settings_get '{}')
  require_text settings_get "$get_line" >/dev/null
  upd_line=$(call_tool settings_update '{"poll_interval_secs":1,"importance_keywords":["otp"]}')
  upd_text=$(require_text settings_update "$upd_line")
  printf '%s' "$upd_text" | python3 -c '
import json, sys
body = json.loads(sys.stdin.read())
if body.get("poll_interval_secs") != 30 or body.get("importance_keywords") != ["otp"]:
    raise SystemExit(body)
'
  grep -F 'poll_interval_secs = 30' "$SETTINGS_PATH" >/dev/null || die "settings file did not record the clamped poll interval"
  grep -F 'otp' "$SETTINGS_PATH" >/dev/null || die "settings file did not record the keyword"
  cp "$SETTINGS_PATH" "$EVIDENCE_DIR/settings.toml"
  chmod 600 "$EVIDENCE_DIR/settings.toml"
  save_lines "$EVIDENCE_DIR/settings.json" "$who_line" "$get_line" "$upd_line"
  assert_no_secrets "$EVIDENCE_DIR/settings.json" "$EVIDENCE_DIR/settings.toml"
  printf 'protbot verify: drove settings\n' | tee "$EVIDENCE_DIR/settings.txt"
  printf 'protbot verify: whoami=%s\n' "$who_text"
}

export BRIDGE_USER
case "$feature" in
  mail) drive_mail ;;
  pass) drive_pass ;;
  drive) drive_drive ;;
  calendar) drive_calendar ;;
  settings) drive_settings ;;
  *) die "usage: drive.sh mail|pass|drive|calendar|settings" ;;
esac
