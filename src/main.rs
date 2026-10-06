use std::{env, fs, io, sync::Arc, time::Duration};

use tokio::sync::watch;
use wizrust101_obs::{
    cli::{CliOptions, HELP},
    config::AppConfig,
    discovery, mapping, peer, server,
    state::SharedState,
    ui, watcher,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = CliOptions::parse(env::args().skip(1))?;
    if options.help {
        print!("{HELP}");
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run(options))
}

async fn run(options: CliOptions) -> Result<(), Box<dyn std::error::Error>> {
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
            "Mock Wizard101 state active: {} / {}. Log discovery is disabled.",
            options.demo_world, options.demo_zone
        );
    }

    let http = tokio::net::TcpListener::bind(("127.0.0.1", options.http_port))
        .await
        .map_err(|error| http_bind_error(options.http_port, error))?;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let http_task = tokio::spawn(server::serve(
        http,
        shared.clone(),
        shutdown_rx.clone(),
        shutdown_tx.clone(),
    ));
    let peer_task = tokio::spawn(peer::supervise_server(shared.clone(), shutdown_rx.clone()));
    let party_client_task = tokio::spawn(peer::run_client_links(
        shared.clone(),
        config_path.clone(),
        shutdown_rx.clone(),
    ));
    let log_task = if options.demo {
        None
    } else {
        let state = shared.clone();
        let catalog = catalog.clone();
        let mut stop = shutdown_rx.clone();
        Some(tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.changed() => break,
                    _ = async {
                        match discovery::discover_system() {
                            Ok(candidates) => {
                                if let Some(candidate) = candidates.first() {
                                    let _ = watcher::follow(
                                        candidate.path.clone(),
                                        state.clone(),
                                        catalog.clone(),
                                    ).await;
                                }
                            }
                            Err(error) => eprintln!("Log discovery failed: {error}"),
                        }
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    } => {}
                }
            }
        }))
    };

    let display_name = options
        .instance_name
        .unwrap_or_else(|| "WizRust101-OBS".into());
    let overlay_url = format!("http://127.0.0.1:{}/overlay", options.http_port);
    println!("WizRust101-OBS is running in the tray. OBS overlay: {overlay_url}");
    let ui_result = ui::run(
        shared.clone(),
        config_path,
        options.http_port,
        display_name,
        options.demo,
    );

    {
        let mut config = shared.config.lock().unwrap();
        config.collaboration_server_enabled = false;
        config.upnp_port_forward = false;
    }
    let _ = shutdown_tx.send(true);
    if let Some(task) = log_task {
        let _ = task.await;
    }
    let (http_result, _, _) = tokio::join!(http_task, peer_task, party_client_task);
    http_result??;
    ui_result?;
    Ok(())
}

fn http_bind_error(port: u16, error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::AddrInUse {
        io::Error::new(
            error.kind(),
            format!(
                "OBS HTTP port {port} is already in use. Another WizRust101-OBS instance may be running. Leave it running and choose another port with --http-port (for example, --http-port 17843)."
            ),
        )
    } else {
        io::Error::new(
            error.kind(),
            format!("could not bind OBS HTTP port {port}: {error}"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::http_bind_error;
    use std::io;

    #[test]
    fn occupied_http_port_gives_safe_actionable_guidance() {
        let error = http_bind_error(17841, io::Error::new(io::ErrorKind::AddrInUse, "occupied"));
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        let message = error.to_string();
        assert!(message.contains("Another WizRust101-OBS instance may be running"));
        assert!(message.contains("--http-port 17843"));
        assert!(message.contains("Leave it running"));
    }
}
