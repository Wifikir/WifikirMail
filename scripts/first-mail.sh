#!/usr/bin/env bash
# Send the first transactional email via a running WifikirMail API.
# Usage:
#   API_KEY=... RECIPIENT=you@example.com ./scripts/first-mail.sh
set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:8080}"
API_KEY="${API_KEY:?Set API_KEY}"
RECIPIENT="${RECIPIENT:?Set RECIPIENT}"
CODE="${CODE:-ABC123}"
IDEM="${IDEM:-first-$(date +%s)}"

curl -fsS "$BASE_URL/health" >/dev/null
curl -fsS "$BASE_URL/ready" >/dev/null

RESP=$(curl -fsS -X POST "$BASE_URL/v1/emails" \
  -H "Authorization: Bearer ${API_KEY}" \
  -H "Content-Type: application/json" \
  -d "{\"idempotencyKey\":\"${IDEM}\",\"recipient\":\"${RECIPIENT}\",\"templateKey\":\"EmailVerification\",\"referenceId\":\"first\",\"templateData\":{\"verification_code\":\"${CODE}\"}}")

echo "$RESP"
JOB_ID=$(echo "$RESP" | sed -n 's/.*"jobId"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')

if [[ -n "${JOB_ID:-}" ]]; then
  echo
  echo "Polling job $JOB_ID ..."
  for _ in $(seq 1 30); do
    STATUS=$(curl -fsS -H "Authorization: Bearer ${API_KEY}" "$BASE_URL/v1/emails/$JOB_ID")
    echo "$STATUS"
    echo "$STATUS" | grep -Eq '"status"[[:space:]]*:[[:space:]]*"(AcceptedByRecipientServer|Bounced|Failed|Ambiguous)"' && break
    sleep 2
  done
fi
