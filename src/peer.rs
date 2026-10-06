use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Router,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use snow::{Builder, TransportState, params::NoiseParams};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, broadcast},
    task::JoinHandle,
    time::timeout,
};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::{
    config::{PeerCredential, SCHOOLS},
    state::{SharedState, WizardPresence},
};

pub const PEER_PORT: u16 = 17842;
const PROTOCOL: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
const MAX_CIPHERTEXT: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PairingInvite {
    pub peer_id: String,
    pub secret: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum PeerFrame {
    Presence(WizardPresence),
}

#[derive(Deserialize)]
struct PeerQuery {
    peer_id: String,
}

struct PeerServerState {
    shared: SharedState,
    capacity: Arc<Semaphore>,
    shutdown: broadcast::Sender<()>,
}

pub fn create_pairing(peer_id: String) -> Result<(PeerCredential, [u8; 32]), String> {
    if peer_id.is_empty() || peer_id.len() > 64 {
        return Err("peer ID must be 1–64 characters".into());
    }
    let mut secret = [0_u8; 32];
    rand::rng().fill_bytes(&mut secret);
    let encoded = URL_SAFE_NO_PAD.encode(secret);
    Ok((
        PeerCredential {
            peer_id,
            secret: encoded,
            connect_url: None,
            label: String::new(),
        },
        secret,
    ))
}

pub fn invite_for(peer: &PeerCredential, host: &str) -> Result<PairingInvite, String> {
    let host = host.trim();
    if host.is_empty() || host.contains('/') || host.contains('@') {
        return Err("enter a hostname or IP address without a scheme or path".into());
    }
    let encoded_peer_id: String =
        url::form_urlencoded::byte_serialize(peer.peer_id.as_bytes()).collect();
    Ok(PairingInvite {
        peer_id: peer.peer_id.clone(),
        secret: peer.secret.clone(),
        url: format!("ws://{host}:{PEER_PORT}/peer?peer_id={}", encoded_peer_id),
    })
}

pub fn import_invite(value: &str, label: String) -> Result<PeerCredential, String> {
    let invite: PairingInvite =
        serde_json::from_str(value).map_err(|_| "invite JSON is invalid")?;
    if invite.url.len() > 512 || !invite.url.starts_with("ws://") {
        return Err("invite must contain a ws:// peer URL".into());
    }
    let secret = URL_SAFE_NO_PAD
        .decode(&invite.secret)
        .map_err(|_| "invite secret is invalid")?;
    if invite.peer_id.is_empty() || invite.peer_id.len() > 64 || secret.len() != 32 {
        return Err("invite fields are invalid".into());
    }
    Ok(PeerCredential {
        peer_id: invite.peer_id,
        secret: invite.secret,
        connect_url: Some(invite.url),
        label,
    })
}

pub fn valid_presence(p: &WizardPresence) -> bool {
    !p.name.trim().is_empty()
        && p.name.len() <= 80
        && SCHOOLS.contains(&p.school.as_str())
        && p.world.as_ref().is_none_or(|v| v.len() <= 128)
        && p.zone.as_ref().is_none_or(|v| v.len() <= 128)
        && p.session_seconds <= 30 * 24 * 60 * 60
}

pub async fn supervise_server(shared: SharedState) {
    loop {
        let enabled = { shared.config.lock().unwrap().collaboration_server_enabled };
        if enabled {
            serve_until_disabled(shared.clone()).await;
        }
        tokio::time::sleep(Duration::from_millis(900)).await;
    }
}

async fn serve_until_disabled(shared: SharedState) {
    let listener = match TcpListener::bind((Ipv4Addr::UNSPECIFIED, PEER_PORT)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Peer listener could not start: {error}");
            return;
        }
    };
    let mut gateway = if shared.config.lock().unwrap().upnp_port_forward {
        match tokio::task::spawn_blocking(add_mapping).await {
            Ok(Ok(g)) => Some(g),
            Ok(Err(e)) => {
                eprintln!("UPnP mapping failed: {e}");
                None
            }
            Err(_) => None,
        }
    } else {
        None
    };
    let (shutdown, _) = broadcast::channel(1);
    let app = peer_router(shared.clone(), shutdown.clone());
    let mut server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let mut renew = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(1800),
        Duration::from_secs(1800),
    );
    loop {
        tokio::select! {
            _=renew.tick()=>{
                if gateway.is_some() && shared.config.lock().unwrap().upnp_port_forward {
                    if let Some(g)=gateway.take(){let _=tokio::task::spawn_blocking(move||remove_mapping(g)).await;}
                    gateway=match tokio::task::spawn_blocking(add_mapping).await{Ok(Ok(g))=>Some(g),_=>None};
                }
                let (enabled, upnp) = { let c=shared.config.lock().unwrap(); (c.collaboration_server_enabled, c.upnp_port_forward) }; if !enabled || !upnp {break;}
            }
            _=tokio::time::sleep(Duration::from_millis(500))=>{
                let (enabled, upnp) = { let c=shared.config.lock().unwrap(); (c.collaboration_server_enabled, c.upnp_port_forward) }; if !enabled {break;}
                if !upnp && gateway.is_some(){if let Some(g)=gateway.take(){let _=tokio::task::spawn_blocking(move||remove_mapping(g)).await;}}
                else if upnp && gateway.is_none(){gateway=match tokio::task::spawn_blocking(add_mapping).await{Ok(Ok(g))=>Some(g),_=>None};}
            }
            _=&mut server=>break,
        }
    }
    let _ = shutdown.send(());
    server.abort();
    if let Some(g) = gateway {
        let _ = tokio::task::spawn_blocking(move || remove_mapping(g)).await;
    }
}

fn peer_router(shared: SharedState, shutdown: broadcast::Sender<()>) -> Router {
    Router::new()
        .route("/peer", get(peer_upgrade))
        .with_state(Arc::new(PeerServerState {
            shared,
            capacity: Arc::new(Semaphore::new(8)),
            shutdown,
        }))
}

fn add_mapping() -> Result<igd_next::Gateway, String> {
    use igd_next::{PortMappingProtocol, SearchOptions, search_gateway};
    let gateway = search_gateway(SearchOptions::default()).map_err(|e| e.to_string())?;
    let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| e.to_string())?;
    probe
        .connect(SocketAddr::new(gateway.addr.ip(), 9))
        .map_err(|e| e.to_string())?;
    let local = probe.local_addr().map_err(|e| e.to_string())?;
    gateway
        .add_port(
            PortMappingProtocol::TCP,
            PEER_PORT,
            SocketAddr::new(local.ip(), PEER_PORT),
            3600,
            "WizRust101-OBS peer presence",
        )
        .map_err(|e| e.to_string())?;
    Ok(gateway)
}
fn remove_mapping(gateway: igd_next::Gateway) {
    let _ = gateway.remove_port(igd_next::PortMappingProtocol::TCP, PEER_PORT);
}

async fn peer_upgrade(
    Query(query): Query<PeerQuery>,
    State(server): State<Arc<PeerServerState>>,
    ws: WebSocketUpgrade,
) -> Response {
    let Some((credential, secret)) = paired_credential(&server.shared, &query.peer_id) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(permit) = server.capacity.clone().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let shared = server.shared.clone();
    let shutdown = server.shutdown.subscribe();
    let id = credential.peer_id;
    ws.max_message_size(MAX_CIPHERTEXT)
        .max_frame_size(MAX_CIPHERTEXT)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            serve_socket(socket, shared, id, secret, false, shutdown).await;
        })
}

fn paired_credential(shared: &SharedState, peer_id: &str) -> Option<(PeerCredential, [u8; 32])> {
    let credential = shared
        .config
        .lock()
        .unwrap()
        .peer_links
        .iter()
        .find(|p| p.peer_id == peer_id)
        .cloned()?;
    let secret: [u8; 32] = URL_SAFE_NO_PAD
        .decode(&credential.secret)
        .ok()?
        .try_into()
        .ok()?;
    Some((credential, secret))
}

async fn serve_socket(
    socket: WebSocket,
    shared: SharedState,
    peer_id: String,
    psk: [u8; 32],
    initiator: bool,
    mut shutdown: broadcast::Receiver<()>,
) {
    let (mut sink, mut stream) = socket.split();
    let params: NoiseParams = match PROTOCOL.parse() {
        Ok(p) => p,
        Err(_) => return,
    };
    let builder = match Builder::new(params)
        .psk(0, &psk)
        .and_then(|b| b.prologue(peer_id.as_bytes()))
    {
        Ok(b) => b,
        Err(_) => return,
    };
    let mut handshake = match if initiator {
        builder.build_initiator()
    } else {
        builder.build_responder()
    } {
        Ok(h) => h,
        Err(_) => return,
    };
    let mut buf = [0_u8; MAX_CIPHERTEXT];
    if initiator {
        let Ok(n) = handshake.write_message(&[], &mut buf) else {
            return;
        };
        if sink
            .send(Message::Binary(buf[..n].to_vec().into()))
            .await
            .is_err()
        {
            return;
        }
    }
    let incoming = match timeout(Duration::from_secs(8), stream.next()).await {
        Ok(Some(Ok(Message::Binary(data)))) if data.len() <= MAX_CIPHERTEXT => data,
        _ => return,
    };
    if handshake.read_message(&incoming, &mut buf).is_err() {
        return;
    }
    if !initiator {
        let Ok(n) = handshake.write_message(&[], &mut buf) else {
            return;
        };
        if sink
            .send(Message::Binary(buf[..n].to_vec().into()))
            .await
            .is_err()
        {
            return;
        }
    }
    let Ok(mut transport) = handshake.into_transport_mode() else {
        return;
    };
    let mut updates = shared.subscribe();
    let mut presence_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(1),
        Duration::from_secs(1),
    );
    if !send_presence(&mut sink, &mut transport, &shared, &peer_id).await {
        return;
    }
    let mut last = Instant::now() - Duration::from_secs(1);
    loop {
        tokio::select! {
            _ = presence_tick.tick() => if !send_presence(&mut sink, &mut transport, &shared, &peer_id).await { break; },
            _ = shutdown.recv() => break,
            incoming=stream.next()=>match incoming{
                Some(Ok(Message::Binary(data))) if data.len()<=MAX_CIPHERTEXT=>{
                    let mut plain=[0_u8;MAX_CIPHERTEXT];let Ok(n)=transport.read_message(&data,&mut plain) else{break};
                    if last.elapsed()<Duration::from_millis(500){continue;}last=Instant::now();
                    let Ok(PeerFrame::Presence(mut presence))=serde_json::from_slice(&plain[..n]) else{break};
                    presence.peer_id=peer_id.clone();if !valid_presence(&presence){break;}shared.set_peer_presence(presence);
                }
                Some(Ok(Message::Ping(p)))=>if sink.send(Message::Pong(p)).await.is_err(){break},
                Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,
                _=>break,
            },
            update=updates.recv()=>match update{
                Ok(_)=>if !send_presence(&mut sink,&mut transport,&shared,&peer_id).await{break},
                Err(broadcast::error::RecvError::Lagged(_))=>if !send_presence(&mut sink,&mut transport,&shared,&peer_id).await{break},
                Err(_)=>break,
            }
        }
    }
    shared.remove_peer(&peer_id);
}

async fn send_presence(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    transport: &mut TransportState,
    shared: &SharedState,
    peer_id: &str,
) -> bool {
    let Some(mut p) = shared.local_presence() else {
        return false;
    };
    p.peer_id = peer_id.to_owned();
    let Ok(payload) = serde_json::to_vec(&PeerFrame::Presence(p)) else {
        return false;
    };
    if payload.len() > MAX_CIPHERTEXT - 32 {
        return false;
    }
    let mut encrypted = [0_u8; MAX_CIPHERTEXT];
    let Ok(n) = transport.write_message(&payload, &mut encrypted) else {
        return false;
    };
    sink.send(Message::Binary(encrypted[..n].to_vec().into()))
        .await
        .is_ok()
}

pub async fn run_client_links(shared: SharedState) {
    let mut links: Vec<(String, JoinHandle<()>)> = Vec::new();
    loop {
        let config = shared.config.lock().unwrap().clone();
        let desired: BTreeSet<String> = config
            .peer_links
            .iter()
            .filter(|p| p.connect_url.is_some())
            .map(|p| p.peer_id.clone())
            .collect();
        links.retain(|(id, handle)| {
            if !desired.contains(id) {
                handle.abort();
                shared.remove_peer(id);
                false
            } else {
                !handle.is_finished()
            }
        });
        for credential in config
            .peer_links
            .into_iter()
            .filter(|p| p.connect_url.is_some())
        {
            if !links.iter().any(|(id, _)| id == &credential.peer_id) {
                let state = shared.clone();
                let id = credential.peer_id.clone();
                links.push((
                    id,
                    tokio::spawn(async move { client_loop(state, credential).await }),
                ));
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn client_loop(shared: SharedState, credential: PeerCredential) {
    let Some(base) = credential.connect_url.as_deref() else {
        return;
    };
    let mut url = match url::Url::parse(base) {
        Ok(u) => u,
        Err(_) => return,
    };
    url.query_pairs_mut()
        .append_pair("peer_id", &credential.peer_id);
    let Ok(secret_vec) = URL_SAFE_NO_PAD.decode(&credential.secret) else {
        return;
    };
    let Ok(psk): Result<[u8; 32], _> = secret_vec.try_into() else {
        return;
    };
    loop {
        if let Ok(request) = url.as_str().into_client_request()
            && let Ok((mut socket, _)) = tokio_tungstenite::connect_async(request).await
        {
            let params: NoiseParams = PROTOCOL.parse().expect("fixed Noise suite");
            if let Ok(mut handshake) = Builder::new(params)
                .psk(0, &psk)
                .and_then(|b| b.prologue(credential.peer_id.as_bytes()))
                .and_then(|b| b.build_initiator())
            {
                let mut buf = [0_u8; MAX_CIPHERTEXT];
                if let Ok(n) = handshake.write_message(&[], &mut buf)
                    && socket
                        .send(tokio_tungstenite::tungstenite::Message::Binary(
                            buf[..n].to_vec().into(),
                        ))
                        .await
                        .is_ok()
                    && let Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(reply)))) =
                        timeout(Duration::from_secs(8), socket.next()).await
                {
                    let mut plain = [0_u8; MAX_CIPHERTEXT];
                    if handshake.read_message(&reply, &mut plain).is_ok()
                        && let Ok(mut transport) = handshake.into_transport_mode()
                    {
                        client_session(&mut socket, &shared, &credential, &mut transport).await;
                    }
                }
            }
        }
        shared.remove_peer(&credential.peer_id);
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

async fn client_session(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    shared: &SharedState,
    credential: &PeerCredential,
    transport: &mut TransportState,
) {
    let mut updates = shared.subscribe();
    let mut presence_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(1),
        Duration::from_secs(1),
    );
    let mut last = Instant::now() - Duration::from_secs(1);
    if !send_client_presence(socket, shared, credential, transport).await {
        return;
    }
    loop {
        tokio::select! {
            _=presence_tick.tick()=>if !send_client_presence(socket,shared,credential,transport).await{break},
            msg=socket.next()=>match msg{Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(data))) if data.len()<=MAX_CIPHERTEXT=>{let mut plain=[0_u8;MAX_CIPHERTEXT];let Ok(n)=transport.read_message(&data,&mut plain)else{break};if last.elapsed()<Duration::from_millis(500){continue;}last=Instant::now();let Ok(PeerFrame::Presence(mut p))=serde_json::from_slice(&plain[..n])else{break};p.peer_id=credential.peer_id.clone();if !valid_presence(&p){break;}shared.set_peer_presence(p);},Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(p)))=>{let _=socket.send(tokio_tungstenite::tungstenite::Message::Pong(p)).await;},Some(_)=>break,None=>break},
            update=updates.recv()=>match update{Ok(_)|Err(broadcast::error::RecvError::Lagged(_))=>if !send_client_presence(socket,shared,credential,transport).await{break},Err(_)=>break}
        }
    }
    shared.remove_peer(&credential.peer_id);
}

async fn send_client_presence(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    shared: &SharedState,
    credential: &PeerCredential,
    transport: &mut TransportState,
) -> bool {
    let Some(mut p) = shared.local_presence() else {
        return false;
    };
    p.peer_id = credential.peer_id.clone();
    if !valid_presence(&p) {
        return false;
    }
    let Ok(payload) = serde_json::to_vec(&PeerFrame::Presence(p)) else {
        return false;
    };
    let mut encrypted = [0_u8; MAX_CIPHERTEXT];
    let Ok(n) = transport.write_message(&payload, &mut encrypted) else {
        return false;
    };
    socket
        .send(tokio_tungstenite::tungstenite::Message::Binary(
            encrypted[..n].to_vec().into(),
        ))
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invites_validate_secret_and_url() {
        let (mut link, key) = create_pairing("peer-a".into()).unwrap();
        link.secret = URL_SAFE_NO_PAD.encode(key);
        let invite = invite_for(&link, "192.0.2.1").unwrap();
        let imported =
            import_invite(&serde_json::to_string(&invite).unwrap(), "friend".into()).unwrap();
        assert_eq!(imported.peer_id, link.peer_id);
        assert!(valid_secret(&imported.secret));
    }
    #[test]
    fn arbitrary_presence_is_rejected() {
        let p = WizardPresence {
            peer_id: "p".into(),
            name: "Guest".into(),
            school: "Fake".into(),
            ..Default::default()
        };
        assert!(!valid_presence(&p));
    }
    #[test]
    fn noise_psk_handshake_authenticates_and_encrypts_payloads() {
        let key = [7_u8; 32];
        let params: NoiseParams = PROTOCOL.parse().unwrap();
        let mut initiator = Builder::new(params.clone())
            .psk(0, &key)
            .unwrap()
            .prologue(b"peer-a")
            .unwrap()
            .build_initiator()
            .unwrap();
        let mut responder = Builder::new(params)
            .psk(0, &key)
            .unwrap()
            .prologue(b"peer-a")
            .unwrap()
            .build_responder()
            .unwrap();
        let mut first = [0_u8; 128];
        let n = initiator.write_message(&[], &mut first).unwrap();
        let mut plain = [0_u8; 128];
        responder.read_message(&first[..n], &mut plain).unwrap();
        let n = responder.write_message(&[], &mut first).unwrap();
        initiator.read_message(&first[..n], &mut plain).unwrap();
        let mut initiator = initiator.into_transport_mode().unwrap();
        let mut responder = responder.into_transport_mode().unwrap();
        let n = initiator.write_message(b"presence", &mut first).unwrap();
        let plain_len = responder.read_message(&first[..n], &mut plain).unwrap();
        assert_eq!(&plain[..plain_len], b"presence");

        let wrong = [8_u8; 32];
        let mut forged = Builder::new(PROTOCOL.parse().unwrap())
            .psk(0, &wrong)
            .unwrap()
            .prologue(b"peer-a")
            .unwrap()
            .build_initiator()
            .unwrap();
        let n = forged.write_message(&[], &mut first).unwrap();
        assert!(
            Builder::new(PROTOCOL.parse().unwrap())
                .psk(0, &key)
                .unwrap()
                .prologue(b"peer-a")
                .unwrap()
                .build_responder()
                .unwrap()
                .read_message(&first[..n], &mut plain)
                .is_err()
        );
    }
    fn valid_secret(secret: &str) -> bool {
        URL_SAFE_NO_PAD.decode(secret).is_ok_and(|s| s.len() == 32)
    }
    #[tokio::test]
    async fn peer_listener_only_has_the_authenticated_peer_route() {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;
        let shared = SharedState::new(crate::config::AppConfig::default());
        let (shutdown, _) = broadcast::channel(1);
        let app = peer_router(shared, shutdown);
        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/peer?peer_id=unknown")
                    .header("connection", "upgrade")
                    .header("upgrade", "websocket")
                    .header("sec-websocket-version", "13")
                    .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UPGRADE_REQUIRED);
        assert!(
            paired_credential(
                &SharedState::new(crate::config::AppConfig::default()),
                "unknown"
            )
            .is_none()
        );
        let no_state = app
            .oneshot(
                Request::builder()
                    .uri("/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(no_state.status(), StatusCode::NOT_FOUND);
    }
}
