use std::sync::Arc;

use axum::{Router, routing::get};
use shuttle_runtime::SecretStore;
use crate::front::{ApplicationState, subreddit_rss};


mod authorization;
mod logging;
mod reddit;
mod rss;
mod front;

#[shuttle_runtime::main]
async fn axum(#[shuttle_runtime::Secrets] secrets: SecretStore) -> shuttle_axum::ShuttleAxum {
    logging::init_logging();
    let application = ApplicationState::new(Arc::new(secrets));
    let router = Router::new()
        .route("/feed/:subreddit", get(subreddit_rss))
        .with_state(application);

    Ok(router.into())
}
