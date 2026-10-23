use anyhow::{Context, Result};
use minijinja::Environment;
use serde_json::Value as JsonValue;

pub struct EmailTemplate {
    pub subject: String,
    pub body: String,
}

pub fn render_template(template_key: &str, template_data: Option<&JsonValue>) -> Result<EmailTemplate> {
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
            env.add_template("subject", "{{ subject }}")?;
            env.add_template(
                "body",
                r#"
Merhaba,

Wifikir platformunda sizin için yeni bir bildirim var.

{{ message }}

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

    let data = template_data
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow::anyhow!("Template data is required"))?;

    let subject = subject_tmpl
        .render(data)
        .context("Failed to render subject")?;
    let body = body_tmpl.render(data).context("Failed to render body")?;

    Ok(EmailTemplate { subject, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_render_email_verification() {
        let data = json!({
            "verification_code": "ABC123"
        });
        let result = render_template("EmailVerification", Some(&data));
        assert!(result.is_ok());
        let template = result.unwrap();
        assert!(template.subject.contains("Doğrulayın"));
        assert!(template.body.contains("ABC123"));
    }

    #[test]
    fn test_render_password_reset() {
        let data = json!({
            "reset_code": "XYZ789"
        });
        let result = render_template("PasswordReset", Some(&data));
        assert!(result.is_ok());
        let template = result.unwrap();
        assert!(template.subject.contains("Şifre"));
        assert!(template.body.contains("XYZ789"));
    }

    #[test]
    fn test_render_notification() {
        let data = json!({
            "subject": "Önemli Bildirim",
            "message": "Test mesajı"
        });
        let result = render_template("Notification", Some(&data));
        assert!(result.is_ok());
        let template = result.unwrap();
        assert!(template.subject.contains("Önemli"));
        assert!(template.body.contains("Test mesajı"));
    }

    #[test]
    fn test_unknown_template() {
        let data = json!({
            "test": "data"
        });
        let result = render_template("Unknown", Some(&data));
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_template_data() {
        let result = render_template("EmailVerification", None);
        assert!(result.is_err());
    }
}
