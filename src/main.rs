use atom_syndication::{Entry, Feed};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::{routing::get, Router};
use eyre::{eyre, Context};
use futures::future::try_join_all;
use itertools::Itertools;
use moka::future::Cache;
use regex::Regex;
use reqwest::Client;
use std::time::Duration;
use tracing::{error, info};

use serde::Deserialize;

const USER_AGENT: &str = concat!("shuttle-reddit-rss:", env!("CARGO_PKG_VERSION"));

lazy_static::lazy_static! {
    static ref DATA_SCORE: Regex = Regex::new(r#" data-score="(?P<score>\d+)""#).unwrap();
}

type ScoreCache = Cache<String, Option<u64>>;

async fn load_score(client: Client, mut url: String) -> eyre::Result<Option<u64>> {
    url = url.replace("www.reddit.com", "old.reddit.com");
    let res = client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .context("cannot load post")?
        .error_for_status()
        .context("status code error when loading post")?
        .text()
        .await
        .context("cannot parse post text")?;
    let score = DATA_SCORE
        .captures(&res)
        .and_then(|c| c["score"].parse::<u64>().ok());
    Ok(score)
}

async fn get_score(
    client: Client,
    entry: &Entry,
    score_cache: &ScoreCache,
) -> eyre::Result<Option<u64>> {
    match entry.links.get(0) {
        Some(link) => {
            let url = link.href.clone();
            let score = score_cache
                .try_get_with(url.clone(), load_score(client, url))
                .await
                .map_err(|e| eyre!("cannot load score, {e}"))?;
            Ok(score)
        }
        None => Ok(None),
    }
}

async fn feed_filter(
    client: Client,
    subreddit: &str,
    score_cache: ScoreCache,
    min_score: u64,
) -> eyre::Result<String> {
    info!("fetching feed");
    let feed = client
        .get(format!("https://reddit.com/{subreddit}/.rss"))
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .context("cannot send feed reqwest")?
        .error_for_status()
        .context("cannot load feed")?
        .text()
        .await
        .context("cannot parse feed")?;
    let mut atom_feed = Feed::read_from(feed.as_bytes()).context("Cannot parse feed")?;

    info!("fetching scores");
    let score_fetch = atom_feed
        .entries()
        .into_iter()
        .map(|e| get_score(client.clone(), e, &score_cache))
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
    State((client, score_cache)): State<(Client, ScoreCache)>,
    Query(Filter { min_score }): Query<Filter>,
) -> (StatusCode, String) {
    let res = feed_filter(client, "r/rust", score_cache, min_score).await;
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

async fn programming_rss(
    State((client, score_cache)): State<(Client, ScoreCache)>,
    Query(Filter { min_score }): Query<Filter>,
) -> (StatusCode, String) {
    let res = feed_filter(client, "r/programming", score_cache, min_score).await;
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
    let client = Client::new();
    let score_cache: ScoreCache = moka::future::CacheBuilder::new(1000)
        .time_to_live(Duration::from_secs(60 * 60))
        .build();
    let router = Router::new()
        .route("/rust", get(rust_rss))
        .route("/programming", get(programming_rss))
        .with_state((client, score_cache));

    Ok(router.into())
}
