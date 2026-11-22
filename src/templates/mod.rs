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

fn template_source(key: &str, part: &str, fallback: &str) -> String {
    let dir = std::env::var("TEMPLATE_DIR").unwrap_or_else(|_| "templates".to_string());
    let path = format!("{}/{}/{}.jinja", dir, key, part);
    std::fs::read_to_string(path).unwrap_or_else(|_| fallback.to_string())
}

fn render_email_verification(data: &EmailVerificationData) -> Result<EmailTemplate> {
    let mut env = Environment::new();
    let subject_src = template_source(
        "EmailVerification",
        "subject",
        "E-posta Adresinizi Doğrulayın",
    );
    env.add_template("subject", &subject_src)?;
    let body_src = template_source(
        "EmailVerification",
        "body",
        "Doğrulama Kodu: {{ verification_code }}",
    );
    env.add_template("body", &body_src)?;

    let subject = env.get_template("subject")?.render(())?;
    let body = env.get_template("body")?.render(data)?;
    Ok(EmailTemplate { subject, body })
}

fn render_password_reset(data: &PasswordResetData) -> Result<EmailTemplate> {
    let mut env = Environment::new();
    let subject_src = template_source("PasswordReset", "subject", "Şifre Sıfırlama İsteği");
    let body_src = template_source("PasswordReset", "body", "Sıfırlama Kodu: {{ reset_code }}");
    env.add_template("subject", &subject_src)?;
    env.add_template("body", &body_src)?;

    let subject = env.get_template("subject")?.render(())?;
    let body = env.get_template("body")?.render(data)?;
    Ok(EmailTemplate { subject, body })
}

fn render_notification(data: &NotificationData) -> Result<EmailTemplate> {
    let mut env = Environment::new();
    let subject_src = template_source("Notification", "subject", "{{ subject }}");
    let body_src = template_source("Notification", "body", "{{ message }}");
    env.add_template("subject", &subject_src)?;
    env.add_template("body", &body_src)?;

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
