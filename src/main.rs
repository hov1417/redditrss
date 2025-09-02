#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]
#![allow(clippy::multiple_crate_versions)]
use std::sync::Arc;

use crate::front::{subreddit_rss, ApplicationState};
use axum::{routing::get, Router};
use shuttle_runtime::SecretStore;

mod authorization;
mod front;
mod logging;
mod reddit;
mod rss;

#[expect(clippy::unused_async)]
#[shuttle_runtime::main]
async fn axum(#[shuttle_runtime::Secrets] secrets: SecretStore) -> shuttle_axum::ShuttleAxum {
    logging::init_logging();
    let application = ApplicationState::new(Arc::new(secrets));
    let router = Router::new()
        .route("/feed/{subreddit}", get(subreddit_rss))
        .with_state(application);

    Ok(router.into())
}
