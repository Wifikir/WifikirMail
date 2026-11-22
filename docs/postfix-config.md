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

DNS'e DMARC kaydı ekleyin. **Başlangıçta `p=none` kullanın** (sadece raporlama) ve SPF/DKIM doğrulandıktan sonra `quarantine` veya `reject`'e geçin:

```
_dmarc.wifikir.com. IN TXT "v=DMARC1; p=none; rua=mailto:dmarc-reports@wifikir.com; pct=100; adkim=s; aspf=s"
```

SPF ve DKIM düzgün çalıştığını doğruladıktan sonra:
```
_dmarc.wifikir.com. IN TXT "v=DMARC1; p=quarantine; rua=mailto:dmarc-reports@wifikir.com; pct=100; adkim=s; aspf=s"
```

## Reverse DNS (PTR Record)

Sunucunuzun IP adresi için PTR kaydının mail.wifikir.com'a işaret ettiğinden emin olun. Bu işlemi hosting sağlayıcınız üzerinden yapabilirsiniz.

```
1.2.3.4 -> mail.wifikir.com
```

## Postfix Loglarını İzleme

WifikirMail, Postfix loglarını `/var/log/mail.log` üzerinden izler:

```bash
# Manuel log izleme
tail -f /var/log/mail.log
```

**Not:** WifikirMail `/var/log/mail.log` dosyasını takip eder ve okuma pozisyonunu (inode + offset) `/var/lib/wifikir-mail/mail.log.cursor` dosyasında saklar. Cursor yalnızca DB güncellemesi başarılı olduktan sonra ilerletilir. Logrotate sonrası inode değişimi veya truncation algılanır ve yeni dosyaya geçilir.

**systemd erişimi:**
- `StateDirectory=wifikir-mail` → `/var/lib/wifikir-mail` yazılabilir
- `SupplementaryGroups=adm` + `usermod -aG adm wifikirmail` → `/var/log/mail.log` okunabilir

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

## Bounce

İki yol vardır:

1. **Postfix logu.** `status=deferred` geçici hatadır. Postfix denemeyi bırakınca logda `status=bounced` yazar; tracker işi `Bounced` yapar. Bu, anlık 5xx ve soft-bounce sonrası final failure’ı kapsar.
2. **VERP.** `BOUNCE_DOMAIN` doluysa envelope sender `bounce+<job_id>@BOUNCE_DOMAIN` olur. Alıcı sunucu mesajı kabul edip sonra DSN dönerse bu adres mailbox’a düşer.

`BOUNCE_DOMAIN` boşsa envelope sender `MAIL_FROM` olur. İlk gönderim için bounce alanı şart değildir.

Örnek Postfix (yalnızca bounce alıcısı, açık relay değil):

```
recipient_delimiter = +
virtual_alias_domains = bounce.wifikir.com
virtual_alias_maps = regexp:/etc/postfix/bounce-regexp
```

`/etc/postfix/bounce-regexp`:

```
/^bounce\+.*@bounce\.wifikir\.com$/  wifikir-bounce
```

`wifikir-bounce` kullanıcısının home Maildir’i `BOUNCE_MAILDIR` ile aynı olsun (`new/` ve `cur/`). Servis bu dizini okur, DSN’den job id’yi alır ve dosyayı `cur/` altına taşır.
