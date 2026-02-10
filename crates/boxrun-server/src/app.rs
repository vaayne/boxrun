use std::sync::Arc;

use axum::Router;

use crate::box_manager::BoxManager;
use crate::events::EventBus;
use crate::routes::v1_router;
use crate::store::Store;
use crate::ui::ui_router;

/// Shared application state.
pub struct AppState {
    pub manager: BoxManager,
}

/// Build the complete axum application.
pub async fn build_app(db_path: &str) -> Result<(Router, Arc<AppState>), String> {
    tracing::info!("Starting BoxRun server...");

    // Initialize store
    let store = Store::new(db_path).await?;

    // Initialize event bus
    let event_bus = EventBus::new();

    // Initialize box manager
    let manager = BoxManager::new(store, event_bus);
    manager.init().await?;

    let state = Arc::new(AppState { manager });

    let app = Router::new()
        .merge(v1_router())
        .merge(ui_router())
        .with_state(state.clone());

    tracing::info!("BoxRun server ready");

    Ok((app, state))
}

/// Gracefully shutdown the application.
pub async fn shutdown(state: Arc<AppState>) {
    tracing::info!("Shutting down BoxRun server...");
    state.manager.shutdown().await;
}
