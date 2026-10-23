# WifikirMail

Wifikir platformu için küçük, kaynak-verimli işlemsel e-posta servisi. Linux üzerinde Rust ile yazılmıştır.

## Mimari

WifikirMail, transactional e-posta gönderimi için üç ana bileşenden oluşur:

### 1. HTTP API Servisi

- Wifikir'dan gelen istekleri kabul eder (e-posta doğrulama, şifre sıfırlama, bildirimler)
- Şablonları render eder
- Token üretir ve doğrular
- İşleri ve durumlarını PostgreSQL'de saklar
- API key kimlik doğrulama ve rate limiting sağlar
- Idempotency (reference_id + template_key) ile çift gönderimi önler

**Endpoints:**
- `GET /health` - Sağlık kontrolü
- `POST /v1/emails` - Yeni e-posta işi oluştur (202 Accepted döner)
- `GET /v1/emails/:id` - İş durumunu sorgula

### 2. Background Worker

- PostgreSQL'den bekleyen işleri alır (SELECT ... FOR UPDATE SKIP LOCKED)
- Şablonları render eder ve SMTP üzerinden localhost'taki Postfix'e gönderir
- Postfix'in `250 ... queued as <ID>` yanıtından queue ID'yi kaydeder
- Gönderim başarısızsa işi Failed olarak işaretler

### 3. Status Tracker

- Postfix loglarını (journald veya /var/log/mail.log) takip eder
- `status=sent|deferred|bounced` satırlarını arar
- Postfix queue ID ile eşleşen işlerin durumunu günceller

### Önemli Ayrımlar

**DİKKAT:** Aşağıdaki üç durum birbirinden farklıdır:

1. **Postfix mesajı kabul etti** → `SubmittedToMta` durumu
2. **Alıcı sunucu mesajı kabul etti** → `AcceptedByRecipientServer` durumu (status=sent)
3. **Mesaj kullanıcının inbox'una ulaştı** → Bu garanti EDİLEMEZ (spam/promotions klasörü)

WifikirMail sadece 1. ve 2. durumu takip edebilir. E-postanın son kullanıcı tarafından görüldüğünü bilmek için ayrı tracking mekanizmaları gerekir.

### Postfix'in Rolü

- **Postfix** MTA'dır ve delivery queue, retry ve bounce'ları yönetir
- **Sadece loopback** (127.0.0.1:25) üzerinden dinler, asla açık relay değildir
- **WifikirMail** sadece Postfix'e mesaj gönderir, kendi retry/queue mantığı YOKTUR
- Postfix mesajı kabul etmezse (5xx), WifikirMail işi Failed olarak işaretler

## Veri Modeli

### email_jobs Tablosu

```sql
- id (UUID, primary key)
- recipient (VARCHAR)
- template_key (VARCHAR): EmailVerification, PasswordReset, Notification
- reference_id (VARCHAR): Doğrulama kodu, sıfırlama token'ı vb.
- status (VARCHAR): Pending, SubmittedToMta, Deferred, AcceptedByRecipientServer, Bounced, Failed
- postfix_queue_id (VARCHAR, nullable)
- attempt_count (INTEGER)
- created_at (TIMESTAMPTZ)
- submitted_at (TIMESTAMPTZ, nullable)
- completed_at (TIMESTAMPTZ, nullable)
- last_error (TEXT, nullable)
```

**Unique constraint:** (reference_id, template_key) - Idempotency için

### email_job_status_history Tablosu

Her durum değişikliğini kaydeder:
```sql
- id (BIGSERIAL)
- job_id (UUID, FK)
- status (VARCHAR)
- changed_at (TIMESTAMPTZ)
- error_message (TEXT, nullable)
- metadata (JSONB, nullable)
```

### api_keys Tablosu

API key'leri SHA-256 hash olarak saklar:
```sql
- id (UUID)
- key_hash (VARCHAR(64), unique)
- name (VARCHAR)
- created_at (TIMESTAMPTZ)
- last_used_at (TIMESTAMPTZ, nullable)
- is_active (BOOLEAN)
```

## Kurulum

### Gereksinimler

- Rust 1.70+ (`cargo`)
- PostgreSQL 14+
- Postfix (send-only, loopback yapılandırması)
- Linux (systemd + journald)

### 1. PostgreSQL Kurulumu

```bash
sudo apt install postgresql postgresql-contrib
sudo -u postgres createuser wifikirmail
sudo -u postgres createdb wifikir_mail -O wifikirmail
sudo -u postgres psql -c "ALTER USER wifikirmail WITH PASSWORD 'güvenli_şifre';"
```

### 2. Postfix Kurulumu

```bash
sudo apt install postfix

# "Internet Site" seçin ve hostname'i girin (örn: mail.wifikir.com)
```

Postfix yapılandırması için [`docs/postfix-config.md`](docs/postfix-config.md) dosyasına bakın.

**Kritik:** Postfix'in sadece localhost'tan bağlantı kabul ettiğinden emin olun!

### 3. WifikirMail Kurulumu

```bash
# Projeyi klonla
git clone https://github.com/Wifikir/WifikirMail.git
cd WifikirMail

# Build
cargo build --release

# Binary'yi kopyala
sudo mkdir -p /opt/wifikir-mail
sudo cp target/release/wifikir_mail /opt/wifikir-mail/

# Config dosyasını oluştur
sudo mkdir -p /etc/wifikir-mail
sudo cp .env.example /etc/wifikir-mail/wifikir-mail.env
sudo nano /etc/wifikir-mail/wifikir-mail.env  # Ayarları düzenle
```

### 4. Systemd Servisi

```bash
sudo cp systemd/wifikir-mail.service /etc/systemd/system/
sudo useradd -r -s /bin/false wifikirmail
sudo systemctl daemon-reload
sudo systemctl enable wifikir-mail
sudo systemctl start wifikir-mail
sudo systemctl status wifikir-mail
```

### 5. API Key Oluşturma

```bash
# PostgreSQL'e bağlan
psql -U wifikirmail -d wifikir_mail

# API key oluştur (SHA-256 hash)
INSERT INTO api_keys (key_hash, name) 
VALUES (
  encode(digest('sizin-gizli-api-key-burada', 'sha256'), 'hex'),
  'Wifikir Ana Uygulama'
);
```

## API Kullanımı

### E-posta Gönderme

```bash
curl -X POST http://localhost:8080/v1/emails \
  -H "Authorization: Bearer sizin-gizli-api-key-burada" \
  -H "Content-Type: application/json" \
  -d '{
    "recipient": "user@example.com",
    "templateKey": "EmailVerification",
    "referenceId": "ABC123DEF456"
  }'
```

**Yanıt (202 Accepted):**
```json
{
  "job_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "Pending"
}
```

### Durum Sorgulama

```bash
curl http://localhost:8080/v1/emails/550e8400-e29b-41d4-a716-446655440000 \
  -H "Authorization: Bearer sizin-gizli-api-key-burada"
```

**Yanıt:**
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "recipient": "user@example.com",
  "template_key": "EmailVerification",
  "reference_id": "ABC123DEF456",
  "status": "AcceptedByRecipientServer",
  "postfix_queue_id": "3F4A5B6C7D",
  "created_at": "2024-10-23T10:30:00Z",
  "submitted_at": "2024-10-23T10:30:05Z",
  "completed_at": "2024-10-23T10:30:15Z"
}
```

## E-posta Şablonları

Mevcut şablonlar (Türkçe içerik):

1. **EmailVerification** - E-posta doğrulama kodu
2. **PasswordReset** - Şifre sıfırlama kodu
3. **Notification** - Genel bildirimler

Şablonlar `src/templates/mod.rs` içinde minijinja kullanılarak render edilir.

## Güvenlik

- API key'ler SHA-256 hash olarak saklanır
- Rate limiting: Dakika başına 60 istek (varsayılan)
- Idempotency: Aynı `(reference_id, template_key)` için tek e-posta
- Postfix sadece localhost'tan bağlantı kabul eder

## Geliştirme

```bash
# Test
cargo test

# Clippy
cargo clippy

# Format
cargo fmt

# Yerel çalıştırma (.env gerekli)
cp .env.example .env
cargo run
```

## Bounce Handling (Gelecek İçin)

Bounce'ları otomatik işlemek için iki seçenek:

1. **VERP (Variable Envelope Return Path):**
   - Return-Path: `bounce+{job_id}@wifikir.com`
   - Bounce mailbox'ı izle ve job_id'ye göre işle

2. **Dedicated Bounce Mailbox:**
   - IMAP/POP3 ile bounce@wifikir.com'u izle
   - Message-ID veya custom header ile eşleştir

Şu an stub olarak bırakılmıştır.

## Lisans

MIT

## Katkıda Bulunma

Pull request'ler memnuniyetle karşılanır. Büyük değişiklikler için önce bir issue açın.

## İletişim

Wifikir Ekibi - [https://wifikir.com](https://wifikir.com)
