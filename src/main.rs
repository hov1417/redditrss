use std::sync::Arc;

use crate::front::{subreddit_rss, ApplicationState};
use axum::{routing::get, Router};
use shuttle_runtime::SecretStore;

mod authorization;
mod front;
mod logging;
mod reddit;
mod rss;

#[shuttle_runtime::main]
async fn axum(#[shuttle_runtime::Secrets] secrets: SecretStore) -> shuttle_axum::ShuttleAxum {
    logging::init_logging();
    let application = ApplicationState::new(Arc::new(secrets));
    let router = Router::new()
        .route("/feed/:subreddit", get(subreddit_rss))
        .with_state(application);

    Ok(router.into())
}
