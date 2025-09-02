use std::sync::Arc;
use std::time::Duration;

use eyre::{bail, Context, ContextCompat};
use reqwest::{Response, StatusCode};
use shuttle_runtime::SecretStore;
use tokio::sync::{RwLock, RwLockReadGuard};
use tracing::info;

use crate::reddit::auth::RedditAuth;

/// A client to interact with Reddit API.
///
/// Cheaply cloneable.
#[derive(Clone)]
pub struct RedditClient {
    client: reqwest::Client,
    auth: Arc<RedditAuth>,
    /// Throttle mechanism to prevent rate limiting.
    /// It abuses write-preferring implementation of
    /// tokio [RwLock](RwLock) to make other requests wait if needed.
    ///
    /// TODO: this is a very simple throttle mechanism with many flaws
    ///     maybe we should implement a more sophisticated one.
    permit: Arc<RwLock<bool>>,
}

impl RedditClient {
    pub fn new(secret_store: Arc<SecretStore>, client: reqwest::Client) -> Self {
        Self {
            client,
            auth: Arc::new(RedditAuth::new(secret_store)),
            permit: Arc::new(RwLock::new(false)),
        }
    }

    async fn get_token(&self) -> eyre::Result<String> {
        self.auth.get_token(&self.client).await
    }

    /// `ordinary_url` is the URL of the post without the `https://www.reddit.com` part.
    /// e.g. `/r/rust/comments/1234/this_is_a_post/`
    pub async fn get_article_score(&self, ordinary_url: &str) -> eyre::Result<u64> {
        for _ in 0..3 {
            if let Some(score) = self.load_article_score(ordinary_url).await? {
                return Ok(score);
            }
        }
        bail!("Cannot get article score after 3 retries")
    }

    async fn load_article_score(&self, ordinary_url: &str) -> eyre::Result<Option<u64>> {
        let token = self.get_token().await?;

        let guard = self.check_throttle().await?;
        let url = format!("https://oauth.reddit.com/{ordinary_url}");

        info!("Requesting {url}");

        let res = self
            .client
            .get(format!("https://oauth.reddit.com/{ordinary_url}"))
            .query(&[("limit", "1"), ("depth", "1")])
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .context("Cannot send request")?;

        drop(guard);

        if self.rate_limiting(&res).await? {
            return Ok(None);
        }

        let res = res
            .error_for_status()
            .context("Received error status code")?
            .json::<Vec<RedditComment>>()
            .await
            .context("Cannot deserialize article request")?;
        Ok(Some(
            res.first()
                .context("Comments returned empty array")?
                .data
                .children
                .first()
                .context("First comment's children is empty")?
                .data()
                .context("First comment's first child is provided as a comment")?
                .score,
        ))
    }

    /// Rate limiting logic, uses status code and following headers
    /// to determine if we should wait:
    ///
    /// retry-after: Number of seconds to wait before retrying
    /// X-Ratelimit-Used: Approximate number of requests used in this period
    /// X-Ratelimit-Remaining: Approximate number of requests left to use
    /// X-Ratelimit-Reset: Approximate number of seconds to end of period
    ///
    /// returns true if we should retry the request
    async fn rate_limiting(&self, response: &Response) -> eyre::Result<bool> {
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = parse_number_header(response, "retry-after")?
                .context("Received 429, but retry-after header is absent")?;
            self.throttle(retry_after).await;
            return Ok(true);
        }
        let used = parse_number_header(response, "X-Ratelimit-Used")?;
        let remaining = parse_number_header(response, "X-Ratelimit-Remaining")?;
        let reset = parse_number_header(response, "X-Ratelimit-Reset")?;
        info!(
            "rate limiting headers X-Ratelimit-Used: {used:?}, \
                                   X-Ratelimit-Remaining: {remaining:?}, \
                                   X-Ratelimit-Reset: {reset:?}"
        );
        match remaining {
            Some(f) if f <= 1f64 => {
                // By default, we throttle for 1 second
                self.throttle(reset.unwrap_or(1f64)).await;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
    async fn check_throttle(&self) -> eyre::Result<RwLockReadGuard<'_, bool>> {
        Ok(self.permit.read().await)
    }

    async fn throttle(&self, throttle_time: f64) {
        // getting mutable reference to the make other requests wait
        let mut_permit = self.permit.write().await;
        tokio::time::sleep(Duration::from_secs_f64(throttle_time)).await;
        drop(mut_permit);
    }
}

fn parse_number_header(response: &Response, header: &str) -> eyre::Result<Option<f64>> {
    response
        .headers()
        .get(header)
        .map(|h| {
            h.to_str()
                .with_context(|| format!("Cannot parse {header} header"))?
                .parse()
                .with_context(|| format!("Cannot parse {header} header"))
        })
        .transpose()
}

#[derive(serde::Deserialize, Debug)]
struct RedditComment {
    data: RedditCommentData,
}
#[derive(serde::Deserialize, Debug)]
struct RedditCommentData {
    children: Vec<RedditCommentChild>,
}

#[derive(serde::Deserialize, Debug)]
#[serde(untagged)]
#[allow(dead_code)]
enum RedditCommentChild {
    RedditCommentItem(RedditCommentItem),
    String(String),
    Comment(RedditComment),
    Other(serde_json::Value),
}

impl RedditCommentChild {
    fn data(&self) -> eyre::Result<&RedditCommentItemInfo> {
        match self {
            Self::RedditCommentItem(item) => Ok(&item.data),
            Self::Other(v) => {
                bail!("Comment child is an unknown type: {v}")
            }
            _ => {
                bail!("Comment child is not a known type")
            }
        }
    }
}

#[derive(serde::Deserialize, Debug)]
struct RedditCommentItem {
    data: RedditCommentItemInfo,
}

#[derive(serde::Deserialize, Debug)]
struct RedditCommentItemInfo {
    score: u64,
}

#[cfg(test)]
mod tests {

    #[test]
    fn deserialize_test() {
        let data = include_str!("./tests/result.json");
        let res: Vec<super::RedditComment> = serde_json::from_str(data).unwrap();
        insta::assert_debug_snapshot!(res);
    }
}
