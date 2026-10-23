#!/bin/bash
# End-to-end API smoke test against a running headless LanShare server.
#
#   npm run headless -- --local-port 53299 > .dev-data/headless.log 2>&1 &
#   PHONE_OVERRIDE=https://127.0.0.1:39210 scripts/api_smoke.sh .
#
# PHONE_OVERRIDE avoids the macOS firewall prompt for the unsigned test binary;
# without it the script talks to the LAN address printed in the log. Phone
# requests are checked against the CA the server hands out on its loopback port,
# so certificate checks are never skipped.
# It pairs a fake phone, so run it against a throwaway .dev-data directory.
set -u
P="$1"
PY="${PYTHON:-python3}"   # CI on Windows sets PYTHON=python
export PYTHONIOENCODING=utf-8
# Windows Python writes \r\n; strip the \r so comparisons work in Git Bash.
py() { "$PY" "$@" | tr -d '\r'; }
LOG="$P/.dev-data/headless.log"
T=$(mktemp -d)
pass=0; fail=0
ok()   { echo "  ok   $1"; pass=$((pass+1)); }
bad()  { echo "  FAIL $1"; fail=$((fail+1)); }
check() { if [ -n "$2" ] && [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (expected $3, got $2)"; fi; }
contains() { if echo "$2" | grep -q -- "$3"; then ok "$1"; else bad "$1 (missing '$3' in: $(echo "$2" | head -c 300))"; fi; }

HOSTURL=$(grep -o 'http://127.0.0.1:[0-9]*/__local_login?token=[A-Za-z0-9_-]*' "$LOG" | head -1)
PHONE_FROM_LOG=$(grep -o 'phone: *https://[0-9.]*:[0-9]*' "$LOG" | head -1 | sed 's/phone: *//')
PHONE=${PHONE_OVERRIDE:-$PHONE_FROM_LOG}
LOCAL=$(echo "$HOSTURL" | sed 's#/__local_login.*##')
echo "host: $LOCAL  phone: $PHONE"
H=(-s -c "$T/host.jar" -b "$T/host.jar")
TLS=(-sS --cacert "$T/ca.pem")
D=("${TLS[@]}" -c "$T/dev.jar" -b "$T/dev.jar")
X=(-H "X-LanShare: 1")
curl "${H[@]}" -o "$T/ca.pem" "$LOCAL/api/ca.crt"
grep -q 'BEGIN CERTIFICATE' "$T/ca.pem" || { echo "no CA from $LOCAL (is the server in the log still running?)" >&2; rm -rf "$T"; exit 1; }
curl "${TLS[@]}" -o /dev/null "$PHONE/api/ca" || { echo "TLS to $PHONE does not verify against the CA from $LOCAL" >&2; rm -rf "$T"; exit 1; }

echo "== host login"
check "local login redirects" "$(curl "${H[@]}" -o /dev/null -w '%{http_code}' "$HOSTURL")" 303
check "host /api/me" "$(curl "${H[@]}" "$LOCAL/api/me" | py -c 'import sys,json;print(json.load(sys.stdin)["kind"])')" host
check "wrong Host header on loopback rejected" "$(curl "${H[@]}" -o /dev/null -w '%{http_code}' -H 'Host: evil.example:53299' "$LOCAL/api/me")" 403

echo "== pairing"
S=$(curl "${H[@]}" "${X[@]}" -X POST "$LOCAL/api/host/pairing")
TOKEN=$(echo "$S" | py -c 'import sys,json,urllib.parse as u;d=json.load(sys.stdin);print(u.parse_qs(u.urlparse(d["urls"][0]["url"]).query)["t"][0])')
contains "pairing session has QR svg" "$S" '<svg'
check "phone unpaired /api/me is 401" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' "$PHONE/api/me")" 401
check "pair request without CSRF header rejected" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d "{\"token\":\"$TOKEN\",\"name\":\"curl\"}" "$PHONE/api/pair/request")" 403
R=$(curl "${D[@]}" "${X[@]}" -H 'Content-Type: application/json' -d "{\"token\":\"$TOKEN\",\"name\":\"curl-phone\"}" "$PHONE/api/pair/request")
RID=$(echo "$R" | py -c 'import sys,json;print(json.load(sys.stdin)["request_id"])')
contains "pair request returns confirm code" "$R" confirm_code
check "token is single-use" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d "{\"token\":\"$TOKEN\",\"name\":\"x\"}" "$PHONE/api/pair/request")" 403
check "status pending" "$(curl "${D[@]}" "$PHONE/api/pair/status/$RID" | py -c 'import sys,json;print(json.load(sys.stdin)["status"])')" pending
contains "host sees pending request" "$(curl "${H[@]}" "$LOCAL/api/host/pairing/requests")" curl-phone
check "approve" "$(curl "${H[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -X POST "$LOCAL/api/host/pairing/requests/$RID/approve")" 200
check "status approved" "$(curl "${D[@]}" "$PHONE/api/pair/status/$RID" | py -c 'import sys,json;print(json.load(sys.stdin)["status"])')" approved
grep -q ls_dev "$T/dev.jar" && ok "device cookie stored" || bad "device cookie stored"
check "approval cannot be claimed twice" "$(curl "${TLS[@]}" "$PHONE/api/pair/status/$RID" | py -c 'import sys,json;print(json.load(sys.stdin)["status"])')" expired
ME=$(curl "${D[@]}" "$PHONE/api/me")
check "device /api/me" "$(echo "$ME" | py -c 'import sys,json;print(json.load(sys.stdin)["kind"])')" device
DEV=$(echo "$ME" | py -c 'import sys,json;print(json.load(sys.stdin)["device_id"])')

echo "== authorization boundaries"
check "device cannot call host API" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' "$PHONE/api/host/devices")" 403
check "local login not available on LAN" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' "$PHONE/__local_login?token=x")" 404
check "tampered device cookie rejected" "$(curl "${TLS[@]}" -o /dev/null -w '%{http_code}' -b "ls_dev=$DEV.wrongsecret" "$PHONE/api/me")" 401
check "foreign Origin rejected" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Origin: https://evil.example' -H 'Content-Type: application/json' -d '{"text":"x"}' "$PHONE/api/messages")" 403

echo "== messages"
M=$(curl "${D[@]}" "${X[@]}" -H 'Content-Type: application/json' -d '{"client_id":"c-1","text":"\u4f60\u597d\uff0c\u7535\u8111 https://example.com"}' "$PHONE/api/messages")
contains "device sends text" "$M" '"from_host":false'
M2=$(curl "${D[@]}" "${X[@]}" -H 'Content-Type: application/json' -d '{"client_id":"c-1","text":"\u4f60\u597d\uff0c\u7535\u8111 https://example.com"}' "$PHONE/api/messages")
check "retry with same client_id is deduped" "$(echo "$M2" | py -c 'import sys,json;print(json.load(sys.stdin)["id"])')" "$(echo "$M" | py -c 'import sys,json;print(json.load(sys.stdin)["id"])')"
HM=$(curl "${H[@]}" "${X[@]}" -H 'Content-Type: application/json' -d "{\"device\":\"$DEV\",\"client_id\":\"h-1\",\"text\":\"\\u6536\\u5230\"}" "$LOCAL/api/messages")
contains "host replies" "$HM" '"from_host":true'
LIST=$(curl "${D[@]}" "$PHONE/api/messages?limit=10")
check "device sees 2 messages" "$(echo "$LIST" | py -c 'import sys,json;print(len(json.load(sys.stdin)["messages"]))')" 2
check "empty message rejected" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"text":"   "}' "$PHONE/api/messages")" 400
head -c 300000 /dev/urandom > "$T/pic.jpg"
FM=$(curl "${D[@]}" "${X[@]}" --data-binary @"$T/pic.jpg" "$PHONE/api/messages/upload?client_id=f-1&name=IMG_0001.jpg")
contains "device sends a file" "$FM" '"path":"收件箱/IMG_0001.jpg"'
FID=$(echo "$FM" | py -c 'import sys,json;print(json.load(sys.stdin)["id"])')
curl "${H[@]}" -o "$T/back.jpg" "$LOCAL/api/messages/$FID/file"
cmp -s "$T/pic.jpg" "$T/back.jpg" && ok "host downloads identical file" || bad "host downloads identical file"
# Errors carry no-store too, so check that each answer really is the file.
PH=$(curl "${D[@]}" -D - -o /dev/null "$PHONE/api/messages/$FID/file")
contains "phone downloads its own attachment" "$PH" 'content-disposition: attachment'
contains "phone keeps no cached copy of an attachment" "$PH" 'cache-control: no-store'
HH=$(curl "${H[@]}" -D - -o /dev/null "$LOCAL/api/messages/$FID/file")
contains "host attachment answer is the file" "$HH" 'content-disposition: attachment'
contains "host keeps no cached copy of an attachment" "$HH" 'cache-control: no-store'
FM2=$(curl "${D[@]}" "${X[@]}" --data-binary @"$T/pic.jpg" "$PHONE/api/messages/upload?client_id=f-1&name=IMG_0001.jpg")
check "file upload retry is idempotent" "$(echo "$FM2" | py -c 'import sys,json;print(json.load(sys.stdin)["id"])')" "$FID"
# Server data is only ever passed to Python as data, never spliced into its code.
DM=$(curl "${D[@]}" "${X[@]}" -H 'Content-Type: application/json' -d '{"client_id":"del-1","text":"delete me"}' "$PHONE/api/messages" | py -c 'import sys,json;i=json.load(sys.stdin)["id"];print(i if type(i) is int else "")')
case "$DM" in ''|*[!0-9]*) bad "new message has a numeric id (got '$DM')";; esac
check "device deletes its own message" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -X DELETE "$PHONE/api/messages/$DM")" 204
check "deleted message is gone" "$(curl "${D[@]}" "$PHONE/api/messages?limit=50" | py -c 'import sys,json;print(any(m["id"]==int(sys.argv[1]) for m in json.load(sys.stdin)["messages"]))' "$DM")" False
check "a late retry of the deleted message is refused" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"client_id":"del-1","text":"delete me"}' "$PHONE/api/messages")" 410

echo "== SSE"
( curl "${D[@]}" -N --max-time 3 "$PHONE/api/events" > "$T/sse.txt" 2>/dev/null ) &
sleep 1
curl "${H[@]}" "${X[@]}" -o /dev/null -H 'Content-Type: application/json' -d "{\"device\":\"$DEV\",\"text\":\"live\"}" "$LOCAL/api/messages"
wait
contains "device receives live msg event" "$(cat "$T/sse.txt")" 'event: msg'
contains "live event payload" "$(cat "$T/sse.txt")" '"text":"live"'

echo "== M8: unread, notifications, clipboard"
contains "device list carries unread counts" "$(curl "${H[@]}" "$LOCAL/api/host/devices")" '"unread":'
check "window reports the conversation in front" "$(curl "${H[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d "{\"device\":\"$DEV\"}" "$LOCAL/api/host/viewing")" 204
check "viewing an unknown device is refused" "$(curl "${H[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"device":"nope"}' "$LOCAL/api/host/viewing")" 404
check "notification text can be hidden" "$(curl "${H[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"hide_text":false}' "$LOCAL/api/host/notifications")" 204
check "headless has no global shortcut" "$(curl "${H[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"shortcut":"Control+Shift+KeyV"}' "$LOCAL/api/host/clipboard/shortcut")" 409
check "a one-modifier shortcut is refused" "$(curl "${H[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"shortcut":"Super+KeyV"}' "$LOCAL/api/host/clipboard/shortcut")" 400
check "Settings can say it is recording a shortcut" "$(curl "${H[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"on":false}' "$LOCAL/api/host/clipboard/shortcut/recording")" 204
check "an unknown message has no file to show" "$(curl "${H[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{}' "$LOCAL/api/host/messages/999999/reveal")" 404
check "a phone cannot set the clipboard target" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"device":null}' "$PHONE/api/host/clipboard/target")" 403
contains "overview names the platform" "$(curl "${H[@]}" "$LOCAL/api/host/overview")" '"platform":'

echo "== files"
echo "hello" > "$T/a.txt"
check "upload to root" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' --data-binary @"$T/a.txt" "$PHONE/api/files/upload?dir=&name=a.txt")" 200
contains "upload duplicate gets renamed" "$(curl "${D[@]}" "${X[@]}" --data-binary @"$T/a.txt" "$PHONE/api/files/upload?dir=&name=a.txt")" '"name":"a (1).txt"'
U1=$(curl "${D[@]}" "${X[@]}" --data-binary @"$T/a.txt" "$PHONE/api/files/upload?dir=&name=a.txt&upload_id=u-1" | py -c 'import sys,json;print(json.load(sys.stdin)["name"])')
U2=$(curl "${D[@]}" "${X[@]}" --data-binary @"$T/a.txt" "$PHONE/api/files/upload?dir=&name=a.txt&upload_id=u-1" | py -c 'import sys,json;print(json.load(sys.stdin)["name"])')
check "upload retried with its id keeps one copy" "$U1 $U2" "a (2).txt a (2).txt"
check "upload check says a small file fits" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' "$PHONE/api/upload-check?dir=&size=5")" 204
check "upload whose data does not match its size is refused" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' --data-binary @"$T/a.txt" "$PHONE/api/files/upload?dir=&name=short.txt&size=999")" 400
check "upload with the right size goes through" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' --data-binary @"$T/a.txt" "$PHONE/api/files/upload?dir=&name=sized.txt&size=6")" 200
check "mkdir" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"dir":"","name":"\u7167\u7247"}' "$PHONE/api/files/mkdir")" 200
check "mkdir duplicate conflicts" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"dir":"","name":"\u7167\u7247"}' "$PHONE/api/files/mkdir")" 409
L=$(curl "${D[@]}" "$PHONE/api/files?path=")
contains "listing shows folder" "$L" '"name":"照片","path":"照片","is_dir":true'
check "range request" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' -H 'Range: bytes=0-2' "$PHONE/api/files/download?path=a.txt")" 206
check "range body" "$(curl "${D[@]}" -H 'Range: bytes=1-3' "$PHONE/api/files/download?path=a.txt")" ell
check "rename" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"path":"a (1).txt","name":"b.txt"}' "$PHONE/api/files/rename")" 200
check "delete to trash" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"path":"b.txt"}' "$PHONE/api/files/delete")" 200
check "deleted file gone" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' "$PHONE/api/files/download?path=b.txt")" 404

echo "== file security"
for bad in '../secret' '..%2F..%2Fetc%2Fpasswd' '%2Fetc%2Fpasswd' '.dev' 'a%5Cb' 'C:%2FWindows'; do
  code=$(curl "${D[@]}" -o /dev/null -w '%{http_code}' "$PHONE/api/files/download?path=$bad")
  case "$code" in 400|403|404) ok "traversal '$bad' -> $code";; *) bad "traversal '$bad' -> $code";; esac
done
check "cannot delete shared root" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"path":""}' "$PHONE/api/files/delete")" 400
check "upload name with path is flattened" "$(curl "${D[@]}" "${X[@]}" --data-binary @"$T/a.txt" "$PHONE/api/files/upload?dir=&name=..%2F..%2Fevil.txt" | py -c 'import sys,json;print(json.load(sys.stdin)["path"])')" evil.txt
printf '<script>alert(1)</script>' > "$T/x.html"
curl "${D[@]}" "${X[@]}" -o /dev/null --data-binary @"$T/x.html" "$PHONE/api/files/upload?dir=&name=x.html"
HDR=$(curl "${D[@]}" -D - -o /dev/null "$PHONE/api/files/download?path=x.html&inline=1")
contains "html served as text/plain" "$HDR" 'content-type: text/plain; charset=utf-8'
contains "inline files sandboxed" "$HDR" 'content-security-policy: sandbox'
contains "files are not cached" "$HDR" 'cache-control: no-store'
printf '<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"/>' > "$T/x.svg"
curl "${D[@]}" "${X[@]}" -o /dev/null --data-binary @"$T/x.svg" "$PHONE/api/files/upload?dir=&name=x.svg"
SVG=$(curl "${D[@]}" -D - -o /dev/null "$PHONE/api/files/download?path=x.svg&inline=1")
contains "svg is only offered as a download" "$SVG" 'content-disposition: attachment'
PAGE=$(curl "${D[@]}" -D - -o /dev/null "$PHONE/")
contains "page has CSP" "$PAGE" "content-security-policy: default-src 'self'"
contains "page denies framing" "$PAGE" 'x-frame-options: DENY'

echo "== read-only device"
curl "${H[@]}" "${X[@]}" -o /dev/null -X PATCH -H 'Content-Type: application/json' -d '{"can_write":false}' "$LOCAL/api/host/devices/$DEV"
check "read-only device cannot upload" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' --data-binary @"$T/a.txt" "$PHONE/api/files/upload?dir=&name=c.txt")" 403
curl "${H[@]}" "${X[@]}" -o /dev/null -X PATCH -H 'Content-Type: application/json' -d '{"can_write":true}' "$LOCAL/api/host/devices/$DEV"

echo "== pause and revoke"
curl "${H[@]}" "${X[@]}" -o /dev/null -H 'Content-Type: application/json' -d '{"paused":true}' "$LOCAL/api/host/paused"
check "paused blocks phone" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' "$PHONE/api/me")" 503
check "host still works while paused" "$(curl "${H[@]}" -o /dev/null -w '%{http_code}' "$LOCAL/api/me")" 200
curl "${H[@]}" "${X[@]}" -o /dev/null -H 'Content-Type: application/json' -d '{"paused":false}' "$LOCAL/api/host/paused"
check "resume" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' "$PHONE/api/me")" 200
check "remove device" "$(curl "${H[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -X DELETE "$LOCAL/api/host/devices/$DEV")" 204
check "removed device locked out" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' "$PHONE/api/me")" 401

echo "== 6-digit code path"
S2=$(curl "${H[@]}" "${X[@]}" -X POST "$LOCAL/api/host/pairing")
CODE=$(echo "$S2" | py -c 'import sys,json;print(json.load(sys.stdin)["code"])')
check "wrong code rejected" "$(curl "${TLS[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"code":"000000","name":"x"}' "$PHONE/api/pair/request")" 403
check "right code accepted" "$(curl "${TLS[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d "{\"code\":\"$CODE\",\"name\":\"code-phone\"}" "$PHONE/api/pair/request")" 200

echo "== TLS"
CERT=$(echo | openssl s_client -connect "${PHONE#https://}" -alpn h2 -CAfile "$T/ca.pem" 2>/dev/null)
contains "ALPN h2 negotiated" "$CERT" 'ALPN protocol: h2'
contains "chain includes local CA" "$CERT" 'LanShare Local CA'
contains "leaf verifies against the local CA" "$CERT" 'Verify return code: 0'
CANAME=$(curl "${TLS[@]}" "$PHONE/api/ca" | py -c 'import sys,json;print(json.load(sys.stdin)["name"])')
contains "CA name has its own tag" "$CANAME" '^LanShare Local CA (.*) [0-9A-F]\{6\}$'
contains "served CA is the one /api/ca names" "$CERT" "i:.*CN *= *$CANAME"
HN=$(curl "${H[@]}" "$LOCAL/api/host/overview" | py -c 'import sys,json;print(json.load(sys.stdin)["host_name"])')
# The label is cut to 40 characters (tls::ca_common_name); CI runner names are longer.
HN40=$(printf '%s' "$HN" | py -c 'import sys;print(sys.stdin.read()[:40])')
check "CA label is the host name" "$(echo "$CANAME" | sed 's/ [0-9A-F]\{6\}$//')" "LanShare Local CA ($HN40)"
if CN=$(scutil --get ComputerName 2>/dev/null); then
  check "host name is the Computer Name" "$HN" "$CN"
fi
SAN=$(echo "$CERT" | openssl x509 -noout -text 2>/dev/null | grep -A1 'Subject Alternative Name')
if LHN=$(scutil --get LocalHostName 2>/dev/null); then
  contains "leaf carries the Bonjour .local name" "$SAN" "DNS:$(echo "$LHN" | tr '[:upper:]' '[:lower:]').local"
fi

echo
echo "passed: $pass  failed: $fail"
rm -rf "$T"
[ "$fail" -eq 0 ]
