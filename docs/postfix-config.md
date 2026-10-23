# Postfix Yapılandırması - WifikirMail

Bu belge, WifikirMail için sadece gönderen (send-only) Postfix MTA yapılandırmasını açıklar.

## Temel Prensipler

**ÖNEMLİ:** Postfix yalnızca localhost üzerinden bağlantı kabul etmeli ve asla açık relay olarak çalışmamalıdır.

## main.cf Ayarları

```
# Temel ayarlar
myhostname = mail.wifikir.com
mydomain = wifikir.com
myorigin = $mydomain

# Sadece localhost'tan bağlantı kabul et
inet_interfaces = loopback-only
inet_protocols = ipv4

# E-posta almayı devre dışı bırak
mydestination = 
local_transport = error:local delivery is disabled

# Relay ayarları - sadece localhost'tan
mynetworks = 127.0.0.0/8 [::1]/128
relay_domains = 
smtpd_relay_restrictions = permit_mynetworks, reject

# SMTP sunucu ayarları (localhost için)
smtpd_banner = $myhostname ESMTP
smtpd_helo_required = yes

# TLS için outbound bağlantılar
smtp_tls_security_level = may
smtp_tls_loglevel = 1
smtp_tls_session_cache_database = btree:${data_directory}/smtp_scache

# Queue ayarları
maximal_queue_lifetime = 2d
bounce_queue_lifetime = 2d
minimal_backoff_time = 300s
maximal_backoff_time = 4000s

# Logging
maillog_file = /var/log/mail.log
```

## master.cf Ayarları

```
# Sadece localhost'tan SMTP kabul et
127.0.0.1:25      inet  n       -       n       -       -       smtpd
  -o smtpd_client_restrictions=permit_mynetworks,reject
```

## SPF Kaydı

DNS'e aşağıdaki SPF kaydını ekleyin:

```
wifikir.com. IN TXT "v=spf1 ip4:YOUR_SERVER_IP -all"
```

## DKIM İmzalama

### OpenDKIM Kurulumu

```bash
sudo apt install opendkim opendkim-tools

# DKIM anahtarı oluştur
sudo mkdir -p /etc/opendkim/keys/wifikir.com
sudo opendkim-genkey -b 2048 -d wifikir.com -D /etc/opendkim/keys/wifikir.com -s default -v
sudo chown -R opendkim:opendkim /etc/opendkim
```

### OpenDKIM Yapılandırması (/etc/opendkim.conf)

```
Syslog                  yes
SyslogSuccess           yes
LogWhy                  yes
UMask                   002
Mode                    s
Canonicalization        relaxed/simple
Domain                  wifikir.com
Selector                default
KeyFile                 /etc/opendkim/keys/wifikir.com/default.private
Socket                  inet:8891@localhost
```

### Postfix'i OpenDKIM ile Entegre Et

main.cf'e ekle:

```
# DKIM
milter_default_action = accept
milter_protocol = 6
smtpd_milters = inet:localhost:8891
non_smtpd_milters = inet:localhost:8891
```

### DKIM DNS Kaydı

Oluşturulan public key'i (/etc/opendkim/keys/wifikir.com/default.txt) DNS'e ekleyin:

```
default._domainkey.wifikir.com. IN TXT "v=DKIM1; k=rsa; p=MIGfMA0GCS..."
```

## DMARC Politikası

DNS'e DMARC kaydı ekleyin:

```
_dmarc.wifikir.com. IN TXT "v=DMARC1; p=quarantine; rua=mailto:dmarc-reports@wifikir.com; pct=100; adkim=s; aspf=s"
```

## Reverse DNS (PTR Record)

Sunucunuzun IP adresi için PTR kaydının mail.wifikir.com'a işaret ettiğinden emin olun. Bu işlemi hosting sağlayıcınız üzerinden yapabilirsiniz.

```
1.2.3.4 -> mail.wifikir.com
```

## Postfix Loglarını İzleme

WifikirMail, Postfix loglarını journald üzerinden izler:

```bash
# Manuel log izleme
journalctl -u postfix@- -f

# veya klasik log dosyası
tail -f /var/log/mail.log
```

## Test

```bash
# Postfix'in sadece localhost'tan bağlantı kabul ettiğini doğrula
telnet 127.0.0.1 25
telnet YOUR_PUBLIC_IP 25  # Bu BAŞARISIZ olmalı

# Test e-postası gönder
echo "Test" | mail -s "Test" test@example.com
```

## Önemli Notlar

1. **Postfix mesajı kabul etti** = Mesaj Postfix kuyruğuna eklendi
2. **Alıcı sunucu mesajı kabul etti** = Uzak MTA 250 OK döndü (status=sent)
3. **Mesaj inbox'a ulaştı** = Son kullanıcı mesajı gördü (spam/promotions klasörü olabilir)

Bu üç durum farklıdır! WifikirMail sadece 1 ve 2'yi takip edebilir. Son kullanıcının mesajı görüp görmediğini bilmek için ayrı bir mekanizma (örn: tracking pixel) gerekir.

## Bounce Handling (Gelecek)

VERP (Variable Envelope Return Path) kullanarak bounce'ları otomatik işlemek için:

- Return-Path: bounce+{job_id}@wifikir.com
- Bounce mailbox'ı izle ve gelen bounce'ları job_id'ye göre eşle

Bu özellik şu an stub olarak bırakılmıştır.
