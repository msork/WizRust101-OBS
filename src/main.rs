use std::{env, sync::Arc, time::Duration};

use wizrust101_obs::{
    config::AppConfig, discovery, mapping, peer, server, state::SharedState, ui, watcher,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let demo = env::args().any(|arg| arg == "--demo");
    let catalog = Arc::new(mapping::runtime_catalog()?);
    let config = AppConfig::load()?;
    let shared = SharedState::new(config);
    if demo {
        let demo_state = shared.clone();
        tokio::spawn(async move {
            watcher::run_demo(demo_state, catalog).await;
        });
    } else {
        let state = shared.clone();
        tokio::spawn(async move {
            loop {
                match discovery::discover_system() {
                    Ok(candidates) => {
                        if let Some(candidate) = candidates.first() {
                            let _ = watcher::follow(
                                candidate.path.clone(),
                                state.clone(),
                                catalog.clone(),
                            )
                            .await;
                        }
                    }
                    Err(error) => eprintln!("log discovery failed: {error}"),
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }
    let http = tokio::net::TcpListener::bind("127.0.0.1:17841").await?;
    tokio::spawn(server::serve(http, shared.clone()));
    tokio::spawn(peer::supervise_server(shared.clone()));
    tokio::spawn(peer::run_client_links(shared.clone()));
    println!("WizRust101-OBS is running in the tray. OBS overlay: http://127.0.0.1:17841/overlay");
    ui::run(shared.clone())?;
    {
        let mut config = shared.config.lock().unwrap();
        config.collaboration_server_enabled = false;
        config.upnp_port_forward = false;
    }
    tokio::time::sleep(Duration::from_millis(700)).await;
    Ok(())
}
