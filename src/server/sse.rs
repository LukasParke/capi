//! Server-Sent Events stream of CEC bus activity.

use super::AppState;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

pub async fn events_sse(State(state): State<AppState>) -> Response {
    let rx = state.0.hub.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(ev) => {
                let body = serde_json::to_string(&ev).ok()?;
                Some(Ok::<_, std::convert::Infallible>(
                    axum::response::sse::Event::default()
                        .event(ev.kind.clone())
                        .data(body),
                ))
            }
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                tracing::warn!("sse subscriber lagged, dropped {n}");
                None
            }
        }
    });

    axum::response::Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text(": keepalive"),
        )
        .into_response()
}
