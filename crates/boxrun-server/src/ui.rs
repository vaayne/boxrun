use std::sync::Arc;

use axum::http::header;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;

use crate::app::AppState;

const UI_HTML: &str = include_str!("ui.html");

pub fn ui_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(root_redirect))
        .route("/ui", get(dashboard))
}

async fn root_redirect() -> Redirect {
    Redirect::to("/ui")
}

async fn dashboard() -> Response {
    ([(header::CACHE_CONTROL, "no-cache")], Html(UI_HTML)).into_response()
}
