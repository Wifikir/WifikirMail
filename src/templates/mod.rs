use anyhow::{Context, Result};
use minijinja::{context, Environment};

pub struct EmailTemplate {
    pub subject: String,
    pub body: String,
}

pub fn render_template(template_key: &str, reference_id: &str) -> Result<EmailTemplate> {
    let mut env = Environment::new();

    match template_key {
        "EmailVerification" => {
            env.add_template("subject", "E-posta Adresinizi Doğrulayın")?;
            env.add_template(
                "body",
                r#"
Merhaba,

Wifikir hesabınızı oluşturduğunuz için teşekkür ederiz.

E-posta adresinizi doğrulamak için aşağıdaki kodu kullanın:

Doğrulama Kodu: {{ verification_code }}

Bu kod 24 saat geçerlidir.

Eğer bu işlemi siz yapmadıysanız, lütfen bu e-postayı görmezden gelin.

Saygılarımızla,
Wifikir Ekibi
"#,
            )?;
        }
        "PasswordReset" => {
            env.add_template("subject", "Şifre Sıfırlama İsteği")?;
            env.add_template(
                "body",
                r#"
Merhaba,

Wifikir hesabınız için şifre sıfırlama talebinde bulundunuz.

Şifrenizi sıfırlamak için aşağıdaki kodu kullanın:

Sıfırlama Kodu: {{ reset_code }}

Bu kod 1 saat geçerlidir.

Eğer bu işlemi siz yapmadıysanız, lütfen hesabınızın güvenliğini kontrol edin.

Saygılarımızla,
Wifikir Ekibi
"#,
            )?;
        }
        "Notification" => {
            env.add_template("subject", "Wifikir Bildirimi")?;
            env.add_template(
                "body",
                r#"
Merhaba,

Wifikir platformunda sizin için yeni bir bildirim var.

Bildirim: {{ message }}

Detaylar için lütfen hesabınıza giriş yapın.

Saygılarımızla,
Wifikir Ekibi
"#,
            )?;
        }
        _ => anyhow::bail!("Unknown template key: {}", template_key),
    }

    let subject_tmpl = env.get_template("subject")?;
    let body_tmpl = env.get_template("body")?;

    let ctx = context! {
        verification_code => reference_id,
        reset_code => reference_id,
        message => reference_id,
    };

    let subject = subject_tmpl
        .render(&ctx)
        .context("Failed to render subject")?;
    let body = body_tmpl.render(&ctx).context("Failed to render body")?;

    Ok(EmailTemplate { subject, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_email_verification() {
        let result = render_template("EmailVerification", "ABC123");
        assert!(result.is_ok());
        let template = result.unwrap();
        assert!(template.subject.contains("Doğrulayın"));
        assert!(template.body.contains("ABC123"));
    }

    #[test]
    fn test_render_password_reset() {
        let result = render_template("PasswordReset", "XYZ789");
        assert!(result.is_ok());
        let template = result.unwrap();
        assert!(template.subject.contains("Şifre"));
        assert!(template.body.contains("XYZ789"));
    }

    #[test]
    fn test_render_notification() {
        let result = render_template("Notification", "Test mesajı");
        assert!(result.is_ok());
        let template = result.unwrap();
        assert!(template.subject.contains("Bildirim"));
        assert!(template.body.contains("Test mesajı"));
    }

    #[test]
    fn test_unknown_template() {
        let result = render_template("Unknown", "test");
        assert!(result.is_err());
    }
}
