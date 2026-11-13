use anyhow::{Context, Result};
use minijinja::Environment;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub struct EmailTemplate {
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct EmailVerificationData {
    pub verification_code: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PasswordResetData {
    pub reset_code: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NotificationData {
    pub subject: String,
    pub message: String,
}

/// Validate typed per-template fields, then render.
pub fn render_template(
    template_key: &str,
    template_data: Option<&JsonValue>,
) -> Result<EmailTemplate> {
    let data = template_data.ok_or_else(|| anyhow::anyhow!("Template data is required"))?;

    match template_key {
        "EmailVerification" => {
            let typed: EmailVerificationData = serde_json::from_value(data.clone())
                .context("Invalid data for EmailVerification: requires verification_code")?;
            if typed.verification_code.is_empty() {
                anyhow::bail!("verification_code cannot be empty");
            }
            render_email_verification(&typed)
        }
        "PasswordReset" => {
            let typed: PasswordResetData = serde_json::from_value(data.clone())
                .context("Invalid data for PasswordReset: requires reset_code")?;
            if typed.reset_code.is_empty() {
                anyhow::bail!("reset_code cannot be empty");
            }
            render_password_reset(&typed)
        }
        "Notification" => {
            let typed: NotificationData = serde_json::from_value(data.clone())
                .context("Invalid data for Notification: requires subject and message")?;
            if typed.subject.is_empty() || typed.message.is_empty() {
                anyhow::bail!("subject and message cannot be empty");
            }
            render_notification(&typed)
        }
        _ => anyhow::bail!("Unknown template key: {}", template_key),
    }
}

fn render_email_verification(data: &EmailVerificationData) -> Result<EmailTemplate> {
    let mut env = Environment::new();
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

    let subject = env.get_template("subject")?.render(())?;
    let body = env.get_template("body")?.render(data)?;
    Ok(EmailTemplate { subject, body })
}

fn render_password_reset(data: &PasswordResetData) -> Result<EmailTemplate> {
    let mut env = Environment::new();
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

    let subject = env.get_template("subject")?.render(())?;
    let body = env.get_template("body")?.render(data)?;
    Ok(EmailTemplate { subject, body })
}

fn render_notification(data: &NotificationData) -> Result<EmailTemplate> {
    let mut env = Environment::new();
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

    let subject = env.get_template("subject")?.render(data)?;
    let body = env.get_template("body")?.render(data)?;
    Ok(EmailTemplate { subject, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_render_email_verification() {
        let data = json!({ "verification_code": "ABC123" });
        let template = render_template("EmailVerification", Some(&data)).unwrap();
        assert!(template.subject.contains("Doğrulayın"));
        assert!(template.body.contains("ABC123"));
    }

    #[test]
    fn test_render_password_reset() {
        let data = json!({ "reset_code": "XYZ789" });
        let template = render_template("PasswordReset", Some(&data)).unwrap();
        assert!(template.subject.contains("Şifre"));
        assert!(template.body.contains("XYZ789"));
    }

    #[test]
    fn test_render_notification() {
        let data = json!({ "subject": "Önemli Bildirim", "message": "Test mesajı" });
        let template = render_template("Notification", Some(&data)).unwrap();
        assert!(template.subject.contains("Önemli"));
        assert!(template.body.contains("Test mesajı"));
    }

    #[test]
    fn test_unknown_template() {
        let data = json!({ "test": "data" });
        assert!(render_template("Unknown", Some(&data)).is_err());
    }

    #[test]
    fn test_missing_template_data() {
        assert!(render_template("EmailVerification", None).is_err());
    }

    #[test]
    fn test_missing_required_field() {
        let data = json!({ "wrong_field": "x" });
        assert!(render_template("EmailVerification", Some(&data)).is_err());
    }

    #[test]
    fn test_empty_required_field() {
        let data = json!({ "verification_code": "" });
        assert!(render_template("EmailVerification", Some(&data)).is_err());
    }
}
