use crate::authorization::{Authorization, QueryToken};
use crate::reddit::client::RedditClient;
use crate::rss::feed::RssFeedProvider;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use reqwest::{header, Client};
use serde::Deserialize;
use shuttle_runtime::SecretStore;
use std::sync::Arc;
use tracing::error;

/// Application state
/// Should be cheaply cloneable
#[derive(Clone)]
pub struct ApplicationState {
    feed_provider: RssFeedProvider,
    authorization: Authorization,
}

const USER_AGENT: &str = concat!("shuttle:reddit-rss:", env!("CARGO_PKG_VERSION"));

impl ApplicationState {
    pub fn new(secrets: Arc<SecretStore>) -> ApplicationState {
        let client = Client::builder()
            .default_headers({
                let mut headers = header::HeaderMap::new();
                headers.insert(header::USER_AGENT, USER_AGENT.parse().unwrap());
                headers
            })
            .build()
            .unwrap();
        ApplicationState {
            feed_provider: RssFeedProvider::new(
                client.clone(),
                RedditClient::new(secrets.clone(), client.clone()),
            ),
            authorization: Authorization::new(secrets.clone()),
        }
    }
}

#[derive(Deserialize)]
pub struct Filter {
    min_score: u64,
}

pub async fn subreddit_rss(
    State(ApplicationState {
        authorization,
        feed_provider,
        ..
    }): State<ApplicationState>,
    Path(subreddit): Path<String>,
    Query(Filter { min_score }): Query<Filter>,
    Query(auth): Query<QueryToken>,
) -> (StatusCode, String) {
    if !authorization.authorize(auth) {
        return (StatusCode::UNAUTHORIZED, String::from("Unauthorized"));
    }
    let res = feed_provider
        .feed_filter(&format!("r/{subreddit}"), min_score)
        .await;
    match res {
        Ok(s) => (StatusCode::OK, s),
        Err(e) => {
            error!("error: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                String::from("Something went wrong"),
            )
        }
    }
}
