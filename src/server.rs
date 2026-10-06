use std::{convert::Infallible, time::Duration};

use crate::state::{OverlayState, SharedState};
use axum::{
    Json, Router,
    extract::{Path, State},
    response::{
        Html, IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use tokio::sync::{broadcast, watch};
use tokio_stream::StreamExt;

#[derive(Clone)]
struct HttpState {
    shared: SharedState,
    shutdown: watch::Receiver<bool>,
    _shutdown_keepalive: watch::Sender<bool>,
}

pub fn router(state: SharedState) -> Router {
    let (keepalive, shutdown) = watch::channel(false);
    router_with_shutdown(state, shutdown, keepalive)
}

fn router_with_shutdown(
    state: SharedState,
    shutdown: watch::Receiver<bool>,
    shutdown_keepalive: watch::Sender<bool>,
) -> Router {
    let state = HttpState {
        shared: state,
        shutdown,
        _shutdown_keepalive: shutdown_keepalive,
    };
    Router::new()
        .route("/", get(root))
        .route("/overlay", get(overlay))
        .route("/style.css", get(stylesheet))
        .route("/school-palette.json", get(school_palette))
        .route("/state", get(current_state))
        .route("/events", get(events))
        .route("/worlds/{asset}", get(world_asset))
        .route("/schools/{asset}", get(school_asset))
        .with_state(state)
}
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: SharedState,
    mut shutdown: watch::Receiver<bool>,
    shutdown_keepalive: watch::Sender<bool>,
) -> std::io::Result<()> {
    axum::serve(
        listener,
        router_with_shutdown(state, shutdown.clone(), shutdown_keepalive),
    )
    .with_graceful_shutdown(async move {
        let _ = shutdown.changed().await;
    })
    .await
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
async fn school_palette() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/json; charset=utf-8",
        )],
        include_str!("../static/school-palette.json"),
    )
}
async fn world_asset(Path(asset): Path<String>) -> Response {
    let Some(bytes) = (match asset.as_str() {
        "arcanum" => Some(&include_bytes!("../assets/worlds/arcanum.png")[..]),
        "aquila" => Some(&include_bytes!("../assets/worlds/aquila.png")[..]),
        "avalon" => Some(&include_bytes!("../assets/worlds/avalon.png")[..]),
        "azteca" => Some(&include_bytes!("../assets/worlds/azteca.png")[..]),
        "celestia" => Some(&include_bytes!("../assets/worlds/celestia.png")[..]),
        "darkmoor" => Some(&include_bytes!("../assets/worlds/darkmoor.png")[..]),
        "dragonspyre" => Some(&include_bytes!("../assets/worlds/dragonspyre.png")[..]),
        "empyrea" => Some(&include_bytes!("../assets/worlds/empyrea.png")[..]),
        "grizzleheim" => Some(&include_bytes!("../assets/worlds/grizzleheim.png")[..]),
        "karamelle" => Some(&include_bytes!("../assets/worlds/karamelle.png")[..]),
        "khrysalis" => Some(&include_bytes!("../assets/worlds/khrysalis.png")[..]),
        "krokotopia" => Some(&include_bytes!("../assets/worlds/krokotopia.png")[..]),
        "lumeria" => Some(&include_bytes!("../assets/worlds/lumeria.png")[..]),
        "marleybone" => Some(&include_bytes!("../assets/worlds/marleybone.png")[..]),
        "mirage" => Some(&include_bytes!("../assets/worlds/mirage.png")[..]),
        "mooshu" => Some(&include_bytes!("../assets/worlds/mooshu.png")[..]),
        "novus" => Some(&include_bytes!("../assets/worlds/novus.png")[..]),
        "polaris" => Some(&include_bytes!("../assets/worlds/polaris.png")[..]),
        "wallaru" => Some(&include_bytes!("../assets/worlds/wallaru.png")[..]),
        "wizardcity" => Some(&include_bytes!("../assets/worlds/wizardcity.png")[..]),
        "wizard101" => Some(&include_bytes!("../assets/icons/sizes/128.png")[..]),
        "wysteria" => Some(&include_bytes!("../assets/worlds/wysteria.png")[..]),
        "zafaria" => Some(&include_bytes!("../assets/worlds/zafaria.png")[..]),
        _ => None,
    }) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    ([(axum::http::header::CONTENT_TYPE, "image/png")], bytes).into_response()
}
async fn school_asset(Path(asset): Path<String>) -> Response {
    let Some(bytes) = (match asset.as_str() {
        "balance" => Some(&include_bytes!("../assets/schools/balance.png")[..]),
        "death" => Some(&include_bytes!("../assets/schools/death.png")[..]),
        "fire" => Some(&include_bytes!("../assets/schools/fire.png")[..]),
        "ice" => Some(&include_bytes!("../assets/schools/ice.png")[..]),
        "life" => Some(&include_bytes!("../assets/schools/life.png")[..]),
        "myth" => Some(&include_bytes!("../assets/schools/myth.png")[..]),
        "storm" => Some(&include_bytes!("../assets/schools/storm.png")[..]),
        _ => None,
    }) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    ([(axum::http::header::CONTENT_TYPE, "image/png")], bytes).into_response()
}
async fn current_state(State(state): State<HttpState>) -> Json<OverlayState> {
    Json(state.shared.snapshot())
}
async fn events(
    State(state): State<HttpState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.shared.subscribe();
    let initial = state.shared.snapshot();
    let first = tokio_stream::once(Ok::<_, Infallible>(
        Event::default()
            .event("state")
            .json_data(initial)
            .expect("state serialization"),
    ));
    let updates = futures_util::stream::unfold(
        (receiver, state.shutdown, state._shutdown_keepalive),
        |(mut receiver, mut shutdown, shutdown_keepalive)| async move {
            loop {
                tokio::select! {
                    _ = shutdown.changed() => return None,
                    item = receiver.recv() => match item {
                        Ok(value) => return Some((
                            Ok(Event::default().event("state").json_data(value).expect("state serialization")),
                            (receiver, shutdown, shutdown_keepalive),
                        )),
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            }
        },
    );
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
    async fn localhost_server_stops_cleanly_when_application_quits() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let (stop, receiver) = watch::channel(false);
        let server = tokio::spawn(serve(
            listener,
            SharedState::new(AppConfig::default()),
            receiver,
            stop.clone(),
        ));
        stop.send(true).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .expect("HTTP server should stop promptly")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn active_sse_stream_closes_when_application_quits() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, receiver) = watch::channel(false);
        let server = tokio::spawn(serve(
            listener,
            SharedState::new(AppConfig::default()),
            receiver,
            stop.clone(),
        ));
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        client
            .write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\nConnection: keep-alive\r\n\r\n")
            .await
            .unwrap();
        let mut initial = [0; 1024];
        let read =
            tokio::time::timeout(std::time::Duration::from_secs(2), client.read(&mut initial))
                .await
                .unwrap()
                .unwrap();
        assert!(String::from_utf8_lossy(&initial[..read]).contains("text/event-stream"));
        stop.send(true).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .expect("HTTP server should finish with open SSE clients")
            .unwrap()
            .unwrap();
    }
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
    async fn school_palette_endpoint_serves_the_central_palette() {
        let response = router(SharedState::new(AppConfig::default()))
            .oneshot(
                Request::builder()
                    .uri("/school-palette.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let served: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let source: serde_json::Value =
            serde_json::from_str(include_str!("../static/school-palette.json")).unwrap();
        assert_eq!(served, source);
    }

    #[tokio::test]
    async fn serves_actual_rpc_world_icons_and_wizard101_fallback() {
        let app = router(SharedState::new(AppConfig::default()));
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../static/world-assets.json")).unwrap();
        let mut assets = catalog["worlds"]
            .as_object()
            .unwrap()
            .values()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        assets.insert(catalog["fallback"].as_str().unwrap().to_owned());
        for asset in assets {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/worlds/{asset}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], "image/png");
            assert!(
                !response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .is_empty()
            );
        }
        let missing = app
            .oneshot(
                Request::builder()
                    .uri("/worlds/unknown")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn serves_all_school_icons_from_the_assets_directory() {
        let app = router(SharedState::new(AppConfig::default()));
        for school in ["balance", "death", "fire", "ice", "life", "myth", "storm"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/schools/{school}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], "image/png");
            assert!(
                !response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .is_empty()
            );
        }
        let missing = app
            .oneshot(
                Request::builder()
                    .uri("/schools/unknown")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
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

    #[tokio::test]
    async fn overlay_omits_session_timer_and_unknown_location_placeholders() {
        let response = router(SharedState::new(AppConfig::default()))
            .oneshot(
                Request::builder()
                    .uri("/overlay")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let html = String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(!html.contains("session_seconds"));
        assert!(!html.contains("Unknown World"));
        assert!(!html.contains("Location unknown"));
        assert!(html.contains("/worlds/${safeKey}"));
        assert!(html.contains("/schools/${schoolIconKey(school)}"));
        assert!(html.contains("const partyCards = new Map()"));
        assert!(html.contains("members.slice(0, 3)"));
        assert!(!html.contains("party.replaceChildren()"));
        assert!(!html.contains("joined your party"));
        assert!(!html.contains("left your party"));
        assert!(!html.contains("textContent = wizard.school"));
        assert!(html.contains("id=\"primary-school\""));
        assert!(!html.contains("party-notice"));
        let css = include_str!("../static/style.css");
        assert!(css.contains("flex-direction: column;"));
        assert!(css.contains(".party { display: flex; flex-direction: column;"));
        assert!(!css.contains(".party-notice"));
    }
}
