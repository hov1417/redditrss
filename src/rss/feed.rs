use std::sync::Arc;
use std::time::Duration;

use atom_syndication::{Entry, Feed};
use eyre::{bail, eyre, Context};
use futures::future::try_join_all;
use itertools::Itertools;
use reqwest::{Client, Response};
use tracing::info;

use crate::reddit::client::RedditClient;

/// A provider for RSS feed.
/// Should be cheaply cloneable.
#[derive(Clone)]
pub struct RssFeedProvider {
    reddit_client: RedditClient,
    client: Client,
    score_cache: Arc<moka::future::Cache<String, u64>>,
}

impl RssFeedProvider {
    pub fn new(client: Client, reddit_client: RedditClient) -> Self {
        Self {
            reddit_client,
            client,
            score_cache: Arc::new(
                moka::future::CacheBuilder::new(1000)
                    .time_to_live(Duration::from_secs(60 * 60))
                    .build(),
            ),
        }
    }

    pub async fn feed_filter(&self, subreddit: &str, min_score: u64) -> eyre::Result<String> {
        let request = self.fetch_feed(subreddit).await?;
        let feed = request.text().await.context("cannot parse feed")?;
        let mut atom_feed =
            Feed::read_from(feed.as_bytes()).map_err(|e| eyre!("Cannot parse feed: {e:?}"))?;

        info!("fetching scores");
        let score_fetch = atom_feed
            .entries()
            .iter()
            .map(|e| self.get_score(e))
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

    async fn fetch_feed(&self, subreddit: &str) -> eyre::Result<Response> {
        info!("fetching feed");
        let request = self
            .client
            .get(format!("https://reddit.com/{subreddit}/.rss"))
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
        Ok(request)
    }

    async fn load_score(&self, mut url: String) -> eyre::Result<u64> {
        url = url.replace("https://www.reddit.com/", "");
        self.reddit_client
            .get_article_score(&url)
            .await
            .context("Cannot load score from reddit")
    }

    async fn get_score(&self, entry: &Entry) -> eyre::Result<Option<u64>> {
        if let Some(link) = entry.links.first() {
            let url = link.href.clone();
            let score = self
                .score_cache
                .try_get_with(url.clone(), self.load_score(url))
                .await
                .map_err(|e| eyre!("cannot load score, {e:?}"))?;
            Ok(Some(score))
        } else {
            info!("Cannot find link in the entry\n{entry:?}");
            Ok(None)
        }
    }
}
