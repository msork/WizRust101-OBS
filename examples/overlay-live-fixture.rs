use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
};
use serde_json::{Value, json};
use wizrust101_obs::{
    config::{AppConfig, CharacterProfile},
    server,
    state::{SharedState, WizardPresence},
};

const TEST_PORT: u16 = 17849;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = AppConfig::default();
    config.profiles.push(CharacterProfile {
        id: "live-test-owner".into(),
        name: "Live Test Owner".into(),
        school: "Life".into(),
        metadata: Default::default(),
    });
    config.active_profile = Some("live-test-owner".into());
    let shared = SharedState::new(config);
    shared.set_demo_state("Wizard City", "The Commons", "fixture/commons");

    let fixture = Router::new()
        .route(
            "/__test/live-overlay-runner",
            get(|| async { Html(include_str!("../tests/browser/live-overlay-runner.html")) }),
        )
        .route("/__test/party/{count}", post(set_party))
        .with_state(shared.clone());
    let app = server::router(shared).merge(fixture);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", TEST_PORT)).await?;
    println!("overlay live fixture listening on http://127.0.0.1:{TEST_PORT}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn set_party(
    Path(count): Path<usize>,
    State(shared): State<SharedState>,
) -> Result<Json<Value>, StatusCode> {
    if count > 3 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let members = (0..count)
        .map(|index| WizardPresence {
            peer_id: format!("live-test-{index}"),
            name: format!("Party Wizard {}", index + 1),
            school: ["Fire", "Ice", "Storm"][index].into(),
            active: true,
            world: Some("Wizard City".into()),
            world_icon: "wizardcity".into(),
            zone: Some("The Commons".into()),
            session_seconds: 17,
        })
        .collect();
    if !shared.replace_party(members, "live-test-owner") {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let state = shared.snapshot();
    Ok(Json(
        json!({ "revision": state.revision, "party": state.party.len() }),
    ))
}
