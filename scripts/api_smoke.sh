#!/bin/bash
# End-to-end API smoke test against a running headless LanShare server.
#
#   npm run headless -- --local-port 53299 > .dev-data/headless.log 2>&1 &
#   PHONE_OVERRIDE=https://127.0.0.1:39210 scripts/api_smoke.sh .
#
# PHONE_OVERRIDE avoids the macOS firewall prompt for the unsigned test binary;
# without it the script talks to the LAN address printed in the log.
# It pairs a fake phone, so run it against a throwaway .dev-data directory.
set -u
P="$1"
LOG="$P/.dev-data/headless.log"
T=$(mktemp -d)
pass=0; fail=0
ok()   { echo "  ok   $1"; pass=$((pass+1)); }
bad()  { echo "  FAIL $1"; fail=$((fail+1)); }
check() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (expected $3, got $2)"; fi; }
contains() { if echo "$2" | grep -q -- "$3"; then ok "$1"; else bad "$1 (missing '$3' in: $(echo "$2" | head -c 300))"; fi; }

HOSTURL=$(grep -o 'http://127.0.0.1:[0-9]*/__local_login?token=[A-Za-z0-9_-]*' "$LOG" | head -1)
PHONE_FROM_LOG=$(grep -o 'phone: *https://[0-9.]*:[0-9]*' "$LOG" | head -1 | sed 's/phone: *//')
PHONE=${PHONE_OVERRIDE:-$PHONE_FROM_LOG}
LOCAL=$(echo "$HOSTURL" | sed 's#/__local_login.*##')
echo "host: $LOCAL  phone: $PHONE"
H=(-s -c "$T/host.jar" -b "$T/host.jar")
D=(-s -k -c "$T/dev.jar" -b "$T/dev.jar")
X=(-H "X-LanShare: 1")

echo "== host login"
check "local login redirects" "$(curl "${H[@]}" -o /dev/null -w '%{http_code}' "$HOSTURL")" 303
check "host /api/me" "$(curl "${H[@]}" "$LOCAL/api/me" | python3 -c 'import sys,json;print(json.load(sys.stdin)["kind"])')" host
check "wrong Host header on loopback rejected" "$(curl "${H[@]}" -o /dev/null -w '%{http_code}' -H 'Host: evil.example:53299' "$LOCAL/api/me")" 403

echo "== pairing"
S=$(curl "${H[@]}" "${X[@]}" -X POST "$LOCAL/api/host/pairing")
TOKEN=$(echo "$S" | python3 -c 'import sys,json,urllib.parse as u;d=json.load(sys.stdin);print(u.parse_qs(u.urlparse(d["urls"][0]["url"]).query)["t"][0])')
contains "pairing session has QR svg" "$S" '<svg'
check "phone unpaired /api/me is 401" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' "$PHONE/api/me")" 401
check "pair request without CSRF header rejected" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d "{\"token\":\"$TOKEN\",\"name\":\"curl\"}" "$PHONE/api/pair/request")" 403
R=$(curl "${D[@]}" "${X[@]}" -H 'Content-Type: application/json' -d "{\"token\":\"$TOKEN\",\"name\":\"curl-phone\"}" "$PHONE/api/pair/request")
RID=$(echo "$R" | python3 -c 'import sys,json;print(json.load(sys.stdin)["request_id"])')
contains "pair request returns confirm code" "$R" confirm_code
check "token is single-use" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d "{\"token\":\"$TOKEN\",\"name\":\"x\"}" "$PHONE/api/pair/request")" 403
check "status pending" "$(curl "${D[@]}" "$PHONE/api/pair/status/$RID" | python3 -c 'import sys,json;print(json.load(sys.stdin)["status"])')" pending
contains "host sees pending request" "$(curl "${H[@]}" "$LOCAL/api/host/pairing/requests")" curl-phone
check "approve" "$(curl "${H[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -X POST "$LOCAL/api/host/pairing/requests/$RID/approve")" 200
check "status approved" "$(curl "${D[@]}" "$PHONE/api/pair/status/$RID" | python3 -c 'import sys,json;print(json.load(sys.stdin)["status"])')" approved
grep -q ls_dev "$T/dev.jar" && ok "device cookie stored" || bad "device cookie stored"
check "approval cannot be claimed twice" "$(curl -s -k "$PHONE/api/pair/status/$RID" | python3 -c 'import sys,json;print(json.load(sys.stdin)["status"])')" expired
ME=$(curl "${D[@]}" "$PHONE/api/me")
check "device /api/me" "$(echo "$ME" | python3 -c 'import sys,json;print(json.load(sys.stdin)["kind"])')" device
DEV=$(echo "$ME" | python3 -c 'import sys,json;print(json.load(sys.stdin)["device_id"])')

echo "== authorization boundaries"
check "device cannot call host API" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' "$PHONE/api/host/devices")" 403
check "local login not available on LAN" "$(curl "${D[@]}" -o /dev/null -w '%{http_code}' "$PHONE/__local_login?token=x")" 404
check "tampered device cookie rejected" "$(curl -s -k -o /dev/null -w '%{http_code}' -b "ls_dev=$DEV.wrongsecret" "$PHONE/api/me")" 401
check "foreign Origin rejected" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Origin: https://evil.example' -H 'Content-Type: application/json' -d '{"text":"x"}' "$PHONE/api/messages")" 403

echo "== messages"
M=$(curl "${D[@]}" "${X[@]}" -H 'Content-Type: application/json' -d '{"client_id":"c-1","text":"你好，电脑 https://example.com"}' "$PHONE/api/messages")
contains "device sends text" "$M" '"from_host":false'
M2=$(curl "${D[@]}" "${X[@]}" -H 'Content-Type: application/json' -d '{"client_id":"c-1","text":"你好，电脑 https://example.com"}' "$PHONE/api/messages")
check "retry with same client_id is deduped" "$(echo "$M2" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')" "$(echo "$M" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')"
HM=$(curl "${H[@]}" "${X[@]}" -H 'Content-Type: application/json' -d "{\"device\":\"$DEV\",\"client_id\":\"h-1\",\"text\":\"收到\"}" "$LOCAL/api/messages")
contains "host replies" "$HM" '"from_host":true'
LIST=$(curl "${D[@]}" "$PHONE/api/messages?limit=10")
check "device sees 2 messages" "$(echo "$LIST" | python3 -c 'import sys,json;print(len(json.load(sys.stdin)["messages"]))')" 2
check "empty message rejected" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"text":"   "}' "$PHONE/api/messages")" 400
head -c 300000 /dev/urandom > "$T/pic.jpg"
FM=$(curl "${D[@]}" "${X[@]}" --data-binary @"$T/pic.jpg" "$PHONE/api/messages/upload?client_id=f-1&name=IMG_0001.jpg")
contains "device sends a file" "$FM" '"path":"收件箱/IMG_0001.jpg"'
FID=$(echo "$FM" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
curl "${H[@]}" -o "$T/back.jpg" "$LOCAL/api/messages/$FID/file"
cmp -s "$T/pic.jpg" "$T/back.jpg" && ok "host downloads identical file" || bad "host downloads identical file"
FM2=$(curl "${D[@]}" "${X[@]}" --data-binary @"$T/pic.jpg" "$PHONE/api/messages/upload?client_id=f-1&name=IMG_0001.jpg")
check "file upload retry is idempotent" "$(echo "$FM2" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')" "$FID"

echo "== SSE"
( curl "${D[@]}" -N --max-time 3 "$PHONE/api/events" > "$T/sse.txt" ) &
sleep 1
curl "${H[@]}" "${X[@]}" -o /dev/null -H 'Content-Type: application/json' -d "{\"device\":\"$DEV\",\"text\":\"live\"}" "$LOCAL/api/messages"
wait
contains "device receives live msg event" "$(cat "$T/sse.txt")" 'event: msg'
contains "live event payload" "$(cat "$T/sse.txt")" '"text":"live"'

echo "== files"
echo "hello" > "$T/a.txt"
check "upload to root" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' --data-binary @"$T/a.txt" "$PHONE/api/files/upload?dir=&name=a.txt")" 200
contains "upload duplicate gets renamed" "$(curl "${D[@]}" "${X[@]}" --data-binary @"$T/a.txt" "$PHONE/api/files/upload?dir=&name=a.txt")" '"name":"a (1).txt"'
check "mkdir" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"dir":"","name":"照片"}' "$PHONE/api/files/mkdir")" 200
check "mkdir duplicate conflicts" "$(curl "${D[@]}" "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"dir":"","name":"照片"}' "$PHONE/api/files/mkdir")" 409
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
check "upload name with path is flattened" "$(curl "${D[@]}" "${X[@]}" --data-binary @"$T/a.txt" "$PHONE/api/files/upload?dir=&name=..%2F..%2Fevil.txt" | python3 -c 'import sys,json;print(json.load(sys.stdin)["path"])')" evil.txt
printf '<script>alert(1)</script>' > "$T/x.html"
curl "${D[@]}" "${X[@]}" -o /dev/null --data-binary @"$T/x.html" "$PHONE/api/files/upload?dir=&name=x.html"
HDR=$(curl "${D[@]}" -D - -o /dev/null "$PHONE/api/files/download?path=x.html&inline=1")
contains "html served as text/plain" "$HDR" 'content-type: text/plain; charset=utf-8'
contains "inline files sandboxed" "$HDR" 'content-security-policy: sandbox'
SVG=$(curl "${D[@]}" -D - -o /dev/null "$PHONE/api/files/download?path=x.html&inline=1")
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
CODE=$(echo "$S2" | python3 -c 'import sys,json;print(json.load(sys.stdin)["code"])')
check "wrong code rejected" "$(curl -s -k "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d '{"code":"000000","name":"x"}' "$PHONE/api/pair/request")" 403
check "right code accepted" "$(curl -s -k "${X[@]}" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d "{\"code\":\"$CODE\",\"name\":\"code-phone\"}" "$PHONE/api/pair/request")" 200

echo "== TLS"
CERT=$(echo | openssl s_client -connect "${PHONE#https://}" -alpn h2 2>/dev/null)
contains "ALPN h2 negotiated" "$CERT" 'ALPN protocol: h2'
contains "chain includes local CA" "$CERT" 'LanShare Local CA'
CANAME=$(curl -s -k "$PHONE/api/ca" | python3 -c 'import sys,json;print(json.load(sys.stdin)["name"])')
contains "CA name has its own tag" "$CANAME" '^LanShare Local CA (.*) [0-9A-F]\{6\}$'
contains "served CA is the one /api/ca names" "$CERT" "i:.*CN *= *$CANAME"
SAN=$(echo "$CERT" | openssl x509 -noout -text 2>/dev/null | grep -A1 'Subject Alternative Name')
if LHN=$(scutil --get LocalHostName 2>/dev/null); then
  contains "leaf carries the Bonjour .local name" "$SAN" "DNS:$(echo "$LHN" | tr '[:upper:]' '[:lower:]').local"
fi

echo
echo "passed: $pass  failed: $fail"
rm -rf "$T"
