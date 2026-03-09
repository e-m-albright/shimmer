//! Generate a dev JWT for local development.
//!
//! Usage: `cargo run -p shimmer-server --bin gen-token`
//! Or via justfile: `just gen-token`
//!
//! Capture into env: `export SHIMMER_JWT=$(just gen-token)`, then `just dev`

use shimmer_server::auth::{create_token, Claims};

fn main() {
    let secret =
        std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-in-production".into());
    let user_id = std::env::var("SHIMMER_USER_ID").unwrap_or_else(|_| "u_dev_user".into());
    let org_id = std::env::var("SHIMMER_ORG_ID").unwrap_or_else(|_| "org_dev".into());

    let exp = usize::try_from((chrono::Utc::now() + chrono::Duration::days(30)).timestamp())
        .unwrap_or(usize::MAX);

    let claims = Claims {
        sub: user_id.clone(),
        name: "Dev User".into(),
        role: "admin".into(),
        org: org_id,
        exp,
    };

    match create_token(&claims, &secret) {
        Ok(token) => {
            // Print token only — no extra output so the shell can capture it cleanly
            println!("{token}");
        }
        Err(e) => {
            eprintln!("error generating token: {e}");
            std::process::exit(1);
        }
    }
}
