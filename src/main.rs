use atom_syndication::{Entry, Feed};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::{routing::get, Router};
use eyre::Context;
use futures::future::try_join_all;
use itertools::Itertools;
use regex::Regex;
use reqwest::Client;
use tracing::{error, info};

use serde::Deserialize;

const USER_AGENT: &str = "shuttle-reddit-rss:0.1.0";
lazy_static::lazy_static! {
    static ref DATA_SCORE: Regex = Regex::new(r#" data-score="(?P<score>\d+)""#).unwrap();
}

async fn get_score(client: Client, entry: &Entry) -> eyre::Result<Option<u64>> {
    match entry.links.get(0) {
        Some(link) => {
            let mut url = link.href.clone();
            url = url.replace("www.reddit.com", "old.reddit.com");
            let res = client
                .get(url)
                .header("User-Agent", USER_AGENT)
                .send()
                .await
                .context("cannot load post")?
                .text()
                .await?;
            let score = DATA_SCORE
                .captures(&res)
                .and_then(|c| c["score"].parse::<u64>().ok());
            Ok(score)
        }
        None => Ok(None),
    }
}

async fn feed_filter(client: Client, min_score: u64) -> eyre::Result<String> {
    info!("fetching feed");
    let feed = client
        .get("https://reddit.com/r/rust/.rss")
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .context("cannot load feed")?
        .text()
        .await
        .context("cannot parse feed")?;
    let mut atom_feed = Feed::read_from(feed.as_bytes()).context("Cannot parse feed")?;

    info!("fetching scores");
    let score_fetch = atom_feed
        .entries()
        .into_iter()
        .map(|e| get_score(client.clone(), e))
        .collect_vec();
    let scores = try_join_all(score_fetch).await?;

    info!("filtering feed");
    atom_feed.entries = atom_feed
        .entries
        .into_iter()
        .zip(scores.into_iter())
        .filter_map(|(e, s)| match s {
            Some(s) if s >= min_score => Some(e),
            _ => None,
        })
        .collect_vec();

    Ok(atom_feed.to_string())
}

#[derive(Deserialize)]
struct Filter {
    min_score: u64,
}

async fn rust_rss(
    State(client): State<Client>,
    Query(Filter { min_score }): Query<Filter>,
) -> (StatusCode, String) {
    let res = feed_filter(client, min_score).await;
    match res {
        Ok(s) => (StatusCode::OK, s),
        Err(e) => {
            error!("error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Something went wrong"),
            )
        }
    }
}

#[shuttle_runtime::main]
async fn axum() -> shuttle_axum::ShuttleAxum {
    let state = Client::new();
    let router = Router::new()
        .route("/rust", get(rust_rss))
        .with_state(state);

    Ok(router.into())
}
