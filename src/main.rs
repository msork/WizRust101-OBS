use std::{env, fs, sync::Arc, time::Duration};

use wizrust101_obs::{
    cli::{CliOptions, HELP},
    config::AppConfig,
    discovery, mapping, peer, server,
    state::SharedState,
    ui, watcher,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = CliOptions::parse(env::args().skip(1))?;
    if options.help {
        print!("{HELP}");
        return Ok(());
    }

    let config_path = if let Some(data_dir) = &options.data_dir {
        fs::create_dir_all(data_dir)?;
        fs::canonicalize(data_dir)?.join("config.json")
    } else {
        AppConfig::path()?
    };
    let mut config = AppConfig::load_from_path(&config_path)?;
    if let Some(peer_port) = options.peer_port {
        config.peer_port = peer_port;
    }
    if let Some(host) = &options.advertise_host {
        config.advertised_host = host.clone();
        config.manual_address_override = true;
    }

    let catalog = Arc::new(mapping::runtime_catalog()?);
    let shared = SharedState::new(config);
    if options.demo {
        let raw = format!("mock/{}/{}", options.demo_world, options.demo_zone);
        shared.set_demo_state(&options.demo_world, &options.demo_zone, &raw);
        eprintln!(
            "Mock Wizard101 state active: {} — {}. Log discovery is disabled.",
            options.demo_world, options.demo_zone
        );
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

    let http = tokio::net::TcpListener::bind(("127.0.0.1", options.http_port))
        .await
        .map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!(
                    "could not bind OBS HTTP port {}: {error}",
                    options.http_port
                ),
            )
        })?;
    tokio::spawn(server::serve(http, shared.clone()));
    tokio::spawn(peer::supervise_server(shared.clone()));
    tokio::spawn(peer::run_client_links(shared.clone()));
    let display_name = options
        .instance_name
        .unwrap_or_else(|| format!("HTTP {}", options.http_port));
    let overlay_url = format!("http://127.0.0.1:{}/overlay", options.http_port);
    println!("WizRust101-OBS [{display_name}] is running in the tray. OBS overlay: {overlay_url}");
    ui::run(
        shared.clone(),
        config_path,
        options.http_port,
        display_name,
        options.demo,
    )?;
    {
        let mut config = shared.config.lock().unwrap();
        config.collaboration_server_enabled = false;
        config.upnp_port_forward = false;
    }
    tokio::time::sleep(Duration::from_millis(700)).await;
    Ok(())
}
