# İlk posta kontrol listesi

Bu liste sunucuda `scripts/bootstrap-host.sh` ve `scripts/first-mail.sh` ile birlikte kullanılır.

## Sizden gerekenler

| Alan | Örnek | Not |
|---|---|---|
| SSH | `ubuntu@146.235.206.210` + özel anahtar veya parola | Jenkins’teki host ile aynı olabilir |
| `MAIL_FROM` | `noreply@wifikir.com` | Görünen From; yalnızca adres |
| Gönderen alan adı | `wifikir.com` | SPF / DKIM / DMARC / PTR |
| `DB_PASSWORD` | (güçlü parola) | Yalnızca yerel Postgres; repoya yazılmaz |
| Test alıcısı | kendi adresiniz | İlk `POST /v1/emails` için |
| (İsteğe bağlı) `BOUNCE_DOMAIN` | `bounce.wifikir.com` | İlk gönderimde boş bırakılabilir |

## Sıra

1. Kaynağı sunucuya alın (`/opt/wifikir-mail/src` veya Jenkinsfile).
2. `sudo MAIL_FROM=... DB_PASSWORD=... ./scripts/bootstrap-host.sh`
3. Postfix loopback: `docs/postfix-config.md`
4. DNS: SPF (`ip4:SUNUCU_IP`), OpenDKIM, DMARC `p=none`, PTR
5. `cargo build --release` → `bin/wifikir_mail` ve `bin/wifikir_mail_cli`
6. `systemctl restart wifikir-mail` → `/health` ve `/ready`
7. `wifikir_mail_cli keys create --name first` (düz metin bir kez)
8. `API_KEY=... RECIPIENT=... ./scripts/first-mail.sh`

## Durum anlamı

- `SubmittedToMta` — Postfix kuyruğa aldı
- `AcceptedByRecipientServer` — alıcı sunucu kabul etti (`status=sent`)
- Gelen kutusu ayrıca garanti değildir
