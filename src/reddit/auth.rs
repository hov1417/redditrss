use std::sync::Arc;

use eyre::{eyre, Context, ContextCompat};
use reqwest::Client;
use serde::Deserialize;
use shuttle_runtime::SecretStore;
use tracing::debug;

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // used for debugging
struct AuthResponse {
    pub access_token: String,
    pub expires_in: i64,
    pub scope: String,
    pub token_type: String,
}

pub struct RedditAuth {
    // TODO: maybe there is a better way to cache the token
    token_cache: moka::future::Cache<(), String>,
    secrets: Arc<SecretStore>,
}

impl RedditAuth {
    pub fn new(secrets: Arc<SecretStore>) -> RedditAuth {
        RedditAuth {
            token_cache: moka::future::CacheBuilder::new(1)
                .time_to_live(std::time::Duration::from_secs(4 * 60 * 60)) // 4 hours
                .build(),
            secrets,
        }
    }

    pub async fn get_token(&self, client: &Client) -> eyre::Result<String> {
        self.token_cache
            .try_get_with((), get_token(client, &self.secrets))
            .await
            .map_err(|e| eyre!("cannot get token, {e}"))
    }
}

async fn get_token(client: &Client, secrets: &SecretStore) -> eyre::Result<String> {
    let client_id = secrets
        .get("REDDIT_CLIENT_ID")
        .context("cannot get client id")?;
    let client_secret = secrets
        .get("REDDIT_CLIENT_SECRET")
        .context("cannot get client secret")?;
    let username = secrets
        .get("REDDIT_USERNAME")
        .context("cannot get username")?;
    let password = secrets
        .get("REDDIT_PASSWORD")
        .context("cannot get password")?;

    client
        .post("https://oauth.reddit.com/api/v1/access_token")
        .basic_auth(client_id, Some(client_secret))
        .form(&[
            ("grant_type", "password"),
            ("username", &username),
            ("password", &password),
        ])
        .send()
        .await?
        .json::<AuthResponse>()
        .await
        .map(|r| {
            debug!("Got token: {r:?}");
            r.access_token
        })
        .context("cannot get token")
}
