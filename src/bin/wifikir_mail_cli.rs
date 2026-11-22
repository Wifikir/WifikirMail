//! API key management. Plaintext is printed once and never stored.
use anyhow::Result;
use clap::{Parser, Subcommand};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;
use wifikir_mail::auth::hash_api_key;
use wifikir_mail::db::{insert_api_key, list_api_keys, revoke_api_key, rotate_api_key};

#[derive(Parser)]
#[command(name = "wifikir_mail_cli", about = "WifikirMail operator CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create, list, revoke or rotate API keys
    Keys {
        #[command(subcommand)]
        action: KeyAction,
    },
}

#[derive(Subcommand)]
enum KeyAction {
    /// Generate a key, store only its SHA-256 hash, print the plaintext once
    Create {
        #[arg(long)]
        name: String,
    },
    /// List keys (id, name, active, timestamps). Never prints secrets.
    List,
    /// Disable a key
    Revoke {
        #[arg(long)]
        id: Uuid,
    },
    /// Replace the hash with a new random key and print the new plaintext once
    Rotate {
        #[arg(long)]
        id: Uuid,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/wifikir_mail".to_string());
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    match cli.command {
        Command::Keys { action } => match action {
            KeyAction::Create { name } => {
                let plain = generate_key();
                let hash = hash_api_key(&plain);
                let key = insert_api_key(&pool, &name, &hash).await?;
                println!("id={}", key.id);
                println!("name={}", key.name);
                println!("api_key={plain}");
                println!("Store this key now. It will not be shown again.");
            }
            KeyAction::List => {
                for key in list_api_keys(&pool).await? {
                    println!(
                        "id={} name={} active={} created_at={} last_used_at={}",
                        key.id,
                        key.name,
                        key.is_active,
                        key.created_at.to_rfc3339(),
                        key.last_used_at
                            .map(|t| t.to_rfc3339())
                            .unwrap_or_else(|| "-".into())
                    );
                }
            }
            KeyAction::Revoke { id } => {
                if revoke_api_key(&pool, id).await? {
                    println!("revoked id={id}");
                } else {
                    anyhow::bail!("key not found: {id}");
                }
            }
            KeyAction::Rotate { id } => {
                let plain = generate_key();
                let hash = hash_api_key(&plain);
                if rotate_api_key(&pool, id, &hash).await? {
                    println!("id={id}");
                    println!("api_key={plain}");
                    println!("Previous key is invalid. Store this key now.");
                } else {
                    anyhow::bail!("key not found: {id}");
                }
            }
        },
    }
    Ok(())
}

fn generate_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}
