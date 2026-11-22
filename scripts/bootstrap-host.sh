#!/usr/bin/env bash
# Run on the mail host (Ubuntu) with sudo.
# Usage:
#   sudo MAIL_FROM=noreply@example.com DB_PASSWORD='...' ./scripts/bootstrap-host.sh
# Optional:
#   INSTALL_ROOT=/opt/wifikir-mail DB_NAME=wifikir_mail DB_USER=wifikirmail
#   BOUNCE_DOMAIN= bounce.example.com  BOUNCE_MAILDIR=/var/lib/wifikir-mail/bounce
set -euo pipefail

INSTALL_ROOT="${INSTALL_ROOT:-/opt/wifikir-mail}"
ENV_DIR="${ENV_DIR:-/etc/wifikir-mail}"
ENV_FILE="${ENV_FILE:-$ENV_DIR/wifikir-mail.env}"
APP_USER="${APP_USER:-wifikirmail}"
DB_NAME="${DB_NAME:-wifikir_mail}"
DB_USER="${DB_USER:-wifikirmail}"
DB_PASSWORD="${DB_PASSWORD:?Set DB_PASSWORD}"
MAIL_FROM="${MAIL_FROM:?Set MAIL_FROM (e.g. noreply@yourdomain.com)}"
SMTP_HOST="${SMTP_HOST:-127.0.0.1}"
SMTP_PORT="${SMTP_PORT:-25}"
SERVER_HOST="${SERVER_HOST:-127.0.0.1}"
SERVER_PORT="${SERVER_PORT:-8080}"
BOUNCE_DOMAIN="${BOUNCE_DOMAIN:-}"
BOUNCE_MAILDIR="${BOUNCE_MAILDIR:-}"
TEMPLATE_DIR="${TEMPLATE_DIR:-$INSTALL_ROOT/templates}"
REPO_SRC="${REPO_SRC:-$INSTALL_ROOT/src}"

if [[ "$(id -u)" -ne 0 ]]; then
  echo "Run as root (sudo)." >&2
  exit 1
fi

export DEBIAN_FRONTEND=noninteractive
apt-get update -y
apt-get install -y postgresql postgresql-contrib postfix libssl-dev pkg-config curl build-essential

id -u "$APP_USER" >/dev/null 2>&1 || useradd -r -s /usr/sbin/nologin -d "$INSTALL_ROOT" "$APP_USER"
usermod -aG adm "$APP_USER"

mkdir -p "$INSTALL_ROOT/bin" "$INSTALL_ROOT/templates" "$ENV_DIR" /var/lib/wifikir-mail
if [[ -n "$BOUNCE_MAILDIR" ]]; then
  mkdir -p "$BOUNCE_MAILDIR/new" "$BOUNCE_MAILDIR/cur" "$BOUNCE_MAILDIR/tmp"
fi
chown -R "$APP_USER:$APP_USER" "$INSTALL_ROOT" /var/lib/wifikir-mail
chmod 750 "$ENV_DIR"

# Postgres role + database
sudo -u postgres psql -v ON_ERROR_STOP=1 <<SQL
DO \$\$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '${DB_USER}') THEN
    CREATE ROLE ${DB_USER} LOGIN PASSWORD '${DB_PASSWORD}';
  ELSE
    ALTER ROLE ${DB_USER} WITH LOGIN PASSWORD '${DB_PASSWORD}';
  END IF;
END
\$\$;
SELECT 'CREATE DATABASE ${DB_NAME} OWNER ${DB_USER}'
WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = '${DB_NAME}')\gexec
GRANT ALL PRIVILEGES ON DATABASE ${DB_NAME} TO ${DB_USER};
SQL

umask 077
cat >"$ENV_FILE" <<EOF
DATABASE_URL=postgres://${DB_USER}:${DB_PASSWORD}@127.0.0.1/${DB_NAME}
SMTP_HOST=${SMTP_HOST}
SMTP_PORT=${SMTP_PORT}
MAIL_FROM=${MAIL_FROM}
BOUNCE_DOMAIN=${BOUNCE_DOMAIN}
BOUNCE_MAILDIR=${BOUNCE_MAILDIR}
BOUNCE_POLL_SECS=30
SERVER_HOST=${SERVER_HOST}
SERVER_PORT=${SERVER_PORT}
WORKER_INTERVAL_SECS=10
MAX_ATTEMPTS=3
JOB_LEASE_DURATION_SECS=300
TRACKER_POLL_INTERVAL_SECS=30
RATE_LIMIT_PER_MINUTE=60
RATE_LIMIT_GLOBAL=1000
QUEUE_CAPACITY_LIMIT=10000
TEMPLATE_DIR=${TEMPLATE_DIR}
METRICS_HOST=127.0.0.1
METRICS_PORT=9090
MAIL_LOG_PATH=/var/log/mail.log
CURSOR_PATH=/var/lib/wifikir-mail/mail.log.cursor
RUST_LOG=wifikir_mail=info,tower_http=info
EOF
chown root:"$APP_USER" "$ENV_FILE"
chmod 640 "$ENV_FILE"

if [[ -f "$REPO_SRC/systemd/wifikir-mail.service" ]]; then
  install -m 644 "$REPO_SRC/systemd/wifikir-mail.service" /etc/systemd/system/wifikir-mail.service
  systemctl daemon-reload
  systemctl enable wifikir-mail
fi

if [[ -d "$REPO_SRC/templates" ]]; then
  rsync -a --delete "$REPO_SRC/templates/" "$INSTALL_ROOT/templates/"
  chown -R "$APP_USER:$APP_USER" "$INSTALL_ROOT/templates"
fi

echo
echo "Host bootstrap done."
echo "  env: $ENV_FILE"
echo "  Next: build binaries into $INSTALL_ROOT/bin, then:"
echo "    systemctl restart wifikir-mail"
echo "    sudo -u $APP_USER env \$(grep -v '^#' $ENV_FILE | xargs) $INSTALL_ROOT/bin/wifikir_mail_cli keys create --name first"
echo "Postfix loopback + SPF/DKIM: see docs/postfix-config.md"
echo "Edit /etc/postfix/main.cf for inet_interfaces=loopback-only before opening the public IP."
