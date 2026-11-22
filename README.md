# WifikirMail

Wifikir platformu için küçük, kaynak-verimli işlemsel e-posta servisi. Linux üzerinde Rust ile yazılmıştır.

## Mimari

WifikirMail üç ana bileşenden oluşur. **Token üretimi/doğrulaması Wifikir auth servisinin işidir**; WifikirMail yalnızca aldığı değerleri şablona yerleştirir.

### 1. HTTP API Servisi

- Wifikir'dan gelen istekleri kabul eder (e-posta doğrulama, şifre sıfırlama, bildirimler)
- Typed `templateData` ile şablonları render eder / doğrular
- İşleri ve durumlarını PostgreSQL'de saklar
- API key kimlik doğrulama, iki katmanlı rate limiting ve kuyruk kapasite limiti
- Idempotency: zorunlu `idempotencyKey`, kapsam `(api_key_id, idempotency_key)`

**Endpoints:**
- `GET /health` — liveness
- `GET /ready` — DB connectivity
- `POST /v1/emails` — yeni iş (202 Accepted; e-posta gönderildi iddiası değil)
- `GET /v1/emails/:id` — iş durumu (api_key scoped)

### 2. Background Worker

- PostgreSQL job kuyruğu (`SELECT ... FOR UPDATE SKIP LOCKED` + `Processing` lease)
- SMTP handoff öncesi `Submitting` marker; lease mid-handoff biterse `Ambiguous` (kör resend yok)
- Yerel Postfix'e SMTP; `250 ... queued as <ID>` kaydı
- Retry politikası: bağlantı/4xx → exponential backoff (MAX_ATTEMPTS); 5xx/validation → Failed

### 3. Status Tracker

- `/var/log/mail.log` izler (inode + offset cursor: `/var/lib/wifikir-mail/mail.log.cursor`)
- Cursor yalnızca DB güncellemesi başarılı olduktan sonra ilerler
- Logrotate (inode/truncation) algılanır
- `status=sent|deferred|bounced` satırlarını parse eder

### Durum Ayrımları

1. **Postfix mesajı kabul etti** → `SubmittedToMta`
2. **Alıcı sunucu kabul etti** → `AcceptedByRecipientServer` (`status=sent`)
3. **Inbox'a ulaştı** → **garanti edilemez**

WifikirMail yalnızca 1 ve 2'yi (loglardan) gözlemleyebilir. Asynchronous bounce için isteğe bağlı VERP (`BOUNCE_DOMAIN` + `BOUNCE_MAILDIR`); soft-bounce final `status=bounced` log satırı tracker ile işlenir.

### Postfix'in Rolü

- Postfix MTA'dır; delivery queue, remote retry ve bounce ownership'i onda
- Sadece loopback (`127.0.0.1:25`), açık relay değil
- WifikirMail yalnızca Postfix'e handoff eder; remote delivery retry Postfix'tedir

## Veri Modeli

### email_jobs

```
id, api_key_id, recipient, template_key, reference_id,
idempotency_key (required), template_data (JSONB),
status, postfix_queue_id, attempt_count, next_attempt_at,
worker_id, locked_until,
created_at, submitted_at, completed_at, last_error
```

**Statuses:** Pending, Processing, Submitting, SubmittedToMta, Deferred,
AcceptedByRecipientServer, Bounced, Failed, Ambiguous

**Unique:** `(api_key_id, idempotency_key)`

### email_job_status_history / api_keys

Durum geçmişi ve SHA-256 hash'li API anahtarları.

## Şablonlar (typed)

| templateKey | Zorunlu templateData alanları |
|---|---|
| EmailVerification | `verification_code` |
| PasswordReset | `reset_code` |
| Notification | `subject`, `message` |

## Kurulum

### Gereksinimler

- Rust 1.70+
- PostgreSQL 14+
- Postfix (send-only, loopback)
- Linux + systemd

### Systemd

```bash
sudo cp systemd/wifikir-mail.service /etc/systemd/system/
sudo useradd -r -s /bin/false wifikirmail
sudo usermod -aG adm wifikirmail   # /var/log/mail.log okuma
sudo systemctl daemon-reload
sudo systemctl enable --now wifikir-mail
```

Unit: `StateDirectory=wifikir-mail`, `SupplementaryGroups=adm`. Ayrıntılar: [`docs/postfix-config.md`](docs/postfix-config.md).

### API Key (manuel — CLI sonraki PR)

```sql
INSERT INTO api_keys (key_hash, name)
VALUES (encode(digest('sizin-gizli-api-key', 'sha256'), 'hex'), 'Wifikir');
```

## API Örneği

```bash
curl -X POST http://localhost:8080/v1/emails \
  -H "Authorization: Bearer sizin-gizli-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "recipient": "user@example.com",
    "templateKey": "EmailVerification",
    "referenceId": "order-or-user-ref",
    "idempotencyKey": "verify-user-42-2024-10",
    "templateData": { "verification_code": "ABC123" }
  }'
```

JSON alanları **camelCase**. Aynı `idempotencyKey` + aynı içerik → mevcut job; farklı içerik → 409.

## Rate Limiting

- Per API key: `RATE_LIMIT_PER_MINUTE` (varsayılan 60)
- Global: `RATE_LIMIT_GLOBAL` (varsayılan 1000)
- Queue capacity: `QUEUE_CAPACITY_LIMIT` pending/processing → POST için 503
- In-memory; tek instance için uygundur

## Geliştirme

```bash
cargo build
cargo clippy -- -D warnings
cargo test
```

## Operatörün sağladığı adresler

| Değişken | Anlam |
|---|---|
| `DATABASE_URL` | PostgreSQL |
| `SMTP_HOST` / `SMTP_PORT` | Yerel Postfix (veya test SMTP) |
| `MAIL_FROM` | Gönderen adres |
| `SERVER_HOST` / `SERVER_PORT` | HTTP API |
| `METRICS_HOST` / `METRICS_PORT` | `/metrics` (varsayılan 127.0.0.1:9090) |
| `MAIL_LOG_PATH` / `CURSOR_PATH` | Postfix log ve cursor |
| `TEMPLATE_DIR` | `templates/` |

`docker compose up -d` yalnızca Postgres’i açar. Postfix sizin sunucunuzdadır.

## API anahtarı

```bash
cargo run --bin wifikir_mail_cli -- keys create --name Wifikir
cargo run --bin wifikir_mail_cli -- keys list
cargo run --bin wifikir_mail_cli -- keys revoke --id <uuid>
cargo run --bin wifikir_mail_cli -- keys rotate --id <uuid>
```

Düz metin anahtar yalnızca create/rotate çıktısında bir kez yazılır.

## Şablonlar ve metrikler

Dosyalar: `templates/<TemplateKey>/{subject,body}.jinja`. `GET /metrics` etiketlerinde adres, UUID veya idempotency key yoktur.

## Test

```bash
cargo test
TEST_DATABASE_URL=postgres://wifikirmail:password@localhost/wifikir_mail_test cargo test --test external
```

`TEST_DATABASE_URL` yoksa `tests/external.rs` atlanır. Varsa API idempotency, sahte SMTP (kabul / 5xx / bağlantı yok) ve log satırından durum güncellemesi çalışır.

## Bounce

`BOUNCE_DOMAIN` doluysa envelope sender `bounce+<job_id>@alan` olur. `BOUNCE_MAILDIR` bir Maildir ise gelen DSN işi `Bounced` yapar. İkisi de boş bırakılabilir; ilk gönderim için gerekmez. Postfix `status=bounced` satırları ayrıca log tracker ile işlenir.

Uzak kurulum: `Jenkinsfile`.

## Bounce Handling (Faz 2)

Şu an yalnızca Postfix loglarındaki immediate `status=bounced` gözlemlenir. Asynchronous bounce ve soft-bounce final failure kapsanmaz.

## Lisans

MIT
