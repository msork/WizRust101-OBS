use std::{convert::Infallible, time::Duration};

use crate::state::{OverlayState, SharedState};
use axum::{
    Json, Router,
    extract::State,
    response::{
        Html, IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/overlay", get(overlay))
        .route("/style.css", get(stylesheet))
        .route("/state", get(current_state))
        .route("/events", get(events))
        .with_state(state)
}
pub async fn serve(listener: tokio::net::TcpListener, state: SharedState) -> std::io::Result<()> {
    axum::serve(listener, router(state)).await
}
async fn root() -> Response {
    axum::response::Redirect::temporary("/overlay").into_response()
}
async fn overlay() -> Html<&'static str> {
    Html(include_str!("../static/overlay.html"))
}
async fn stylesheet() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/style.css"),
    )
}
async fn current_state(State(state): State<SharedState>) -> Json<OverlayState> {
    Json(state.snapshot())
}
async fn events(
    State(state): State<SharedState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.subscribe();
    let initial = state.snapshot();
    let first = tokio_stream::once(Ok::<_, Infallible>(
        Event::default()
            .event("state")
            .json_data(initial)
            .expect("state serialization"),
    ));
    let updates = BroadcastStream::new(receiver).filter_map(|item| match item {
        Ok(value) => Some(Ok(Event::default()
            .event("state")
            .json_data(value)
            .expect("state serialization"))),
        Err(_) => None,
    });
    let stream = first.chain(updates);
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    #[tokio::test]
    async fn overlay_and_state_endpoints_are_available() {
        let app = router(SharedState::new(AppConfig::default()));
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/overlay")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let r = router(SharedState::new(AppConfig::default()))
            .oneshot(
                Request::builder()
                    .uri("/style.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let r = app
            .oneshot(
                Request::builder()
                    .uri("/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let b = r.into_body().collect().await.unwrap().to_bytes();
        let s: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert!(s.get("session_seconds").is_some());
    }

    #[tokio::test]
    async fn events_stream_sends_initial_and_updated_state() {
        use crate::{mapping::ZoneCatalog, parser::GameEvent};
        let shared = SharedState::new(AppConfig::default());
        let app = router(shared.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let mut body = response.into_body();
        let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
        assert!(String::from_utf8_lossy(&first).contains("session_seconds"));
        shared.apply(
            GameEvent::ZoneChanged {
                raw_zone_id: "Unknown/Zone".into(),
            },
            &ZoneCatalog::default(),
        );
        let second = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap();
        assert!(String::from_utf8_lossy(&second).contains("Unknown/Zone"));
    }
    #[tokio::test]
    async fn settings_and_configuration_routes_are_not_exposed() {
        let app = router(SharedState::new(crate::config::AppConfig::default()));
        for uri in ["/settings", "/api/config"] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
        }
    }
}
