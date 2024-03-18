use std::time::Duration;

use atom_syndication::{Entry, Feed};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{routing::get, Router};
use color_eyre::config::{EyreHook, HookBuilder, PanicHook, Theme};
use eyre::eyre;
use eyre::{bail, Context};
use futures::future::try_join_all;
use itertools::Itertools;
use moka::future::Cache;
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use tracing::{error, info};

use tracing_error::ErrorLayer;
use tracing_subscriber::fmt;

const USER_AGENT: &str = concat!("shuttle-reddit-rss:", env!("CARGO_PKG_VERSION"));

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
    static DATA_SCORE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#" data-score="(?P<score>\d+)""#).unwrap());

    let score = DATA_SCORE
        .captures(&res)
        .and_then(|c| c["score"].parse::<u64>().ok());

    if score.is_none() {
        info!("Cannot find score in post");
    }

    Ok(score)
}

async fn get_score(
    client: Client,
    entry: &Entry,
    score_cache: &ScoreCache,
) -> eyre::Result<Option<u64>> {
    match entry.links.first() {
        Some(link) => {
            let url = link.href.clone();
            let score = score_cache
                .try_get_with(url.clone(), load_score(client, url))
                .await
                .map_err(|e| eyre!("cannot load score, {e}"))?;
            Ok(score)
        }
        None => {
            info!("Cannot find link in entry");
            Ok(None)
        }
    }
}

async fn feed_filter(
    client: Client,
    subreddit: &str,
    score_cache: ScoreCache,
    min_score: u64,
) -> eyre::Result<String> {
    info!("fetching feed");
    let request = client
        .get(format!("https://reddit.com/{subreddit}/.rss"))
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .context("cannot send feed request")?;
    let status = request.status();
    if status.is_client_error() || status.is_server_error() {
        bail!(
            "cannot load feed: \t\nstatus: {:?}\t\nbody: {:?}",
            status,
            request.text().await
        );
    }
    let feed = request.text().await.context("cannot parse feed")?;
    let mut atom_feed = Feed::read_from(feed.as_bytes()).context("Cannot parse feed")?;

    info!("fetching scores");
    let score_fetch = atom_feed
        .entries()
        .iter()
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

async fn subreddit_rss(
    State((client, score_cache)): State<(Client, ScoreCache)>,
    Path(subreddit): Path<String>,
    Query(Filter { min_score }): Query<Filter>,
) -> (StatusCode, String) {
    let res = feed_filter(client, &format!("r/{subreddit}"), score_cache, min_score).await;
    match res {
        Ok(s) => (StatusCode::OK, s),
        Err(e) => {
            error!("error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                String::from("Something went wrong"),
            )
        }
    }
}

fn build_error_hooks() -> (PanicHook, EyreHook) {
    HookBuilder::new()
        .theme(Theme::default())
        .add_default_filters()
        .into_hooks()
}

fn init_panic_hook() -> eyre::Result<()> {
    let (panic_hook, eyre_hook) = build_error_hooks();

    eyre_hook.install()?;
    std::panic::set_hook(Box::new(move |pi| {
        error!("Panic caught: {}", panic_hook.panic_report(pi));
    }));
    Ok(())
}

fn tracing() {
    use tracing_subscriber::prelude::*;

    tracing_subscriber::registry()
        .with(ErrorLayer::default())
        .with(fmt::layer().with_target(false).with_ansi(true).json())
        .with(
            // let user override RUST_LOG in local run if they want to
            tracing_subscriber::filter::EnvFilter::try_from_default_env()
                .or_else(|_| tracing_subscriber::filter::EnvFilter::try_new("info,shuttle=trace"))
                .unwrap(),
        )
        .init();
}

#[shuttle_runtime::main]
async fn axum() -> shuttle_axum::ShuttleAxum {
    tracing();
    init_panic_hook().unwrap();
    let client = Client::new();
    let score_cache: ScoreCache = moka::future::CacheBuilder::new(1000)
        .time_to_live(Duration::from_secs(60 * 60))
        .build();
    let router = Router::new()
        .route("/feed/:subreddit", get(subreddit_rss))
        .with_state((client, score_cache));

    Ok(router.into())
}
