use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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
pub const HOST_MEMBER_ID: &str = "party-host";
const INVITE_TTL_SECS: u64 = 24 * 60 * 60;
const INVITE_PREFIX: &str = "WIZPARTY1.";
const PROTOCOL: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
const MAX_CIPHERTEXT: usize = 16_384;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PairingInvite {
    pub version: u8,
    pub peer_id: String,
    pub secret: String,
    pub url: String,
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum PeerFrame {
    Join(WizardPresence),
    Welcome { member_id: String },
    PartySnapshot { members: Vec<WizardPresence> },
}

#[derive(Deserialize)]
struct PeerQuery {
    peer_id: String,
}

struct PeerServerState {
    shared: SharedState,
    capacity: Arc<Semaphore>,
    shutdown: broadcast::Sender<()>,
    active_members: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

pub fn create_pairing(peer_id: String) -> Result<(PeerCredential, [u8; 32]), String> {
    if peer_id.is_empty() || peer_id.len() > 64 || peer_id == HOST_MEMBER_ID {
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
            expires_at_unix: Some(unix_now().saturating_add(INVITE_TTL_SECS)),
        },
        secret,
    ))
}

pub fn invite_for(peer: &PeerCredential, host: &str, port: u16) -> Result<PairingInvite, String> {
    let host = host.trim();
    if host.is_empty() || host.contains('/') || host.contains('@') || port < 1024 {
        return Err("enter a hostname or IP address and a valid port".into());
    }
    let encoded_peer_id: String =
        url::form_urlencoded::byte_serialize(peer.peer_id.as_bytes()).collect();
    let issued_at_unix = unix_now();
    let expires_at_unix = peer
        .expires_at_unix
        .unwrap_or_else(|| issued_at_unix.saturating_add(INVITE_TTL_SECS));
    let url = format!("ws://{host}:{port}/peer?peer_id={encoded_peer_id}");
    validate_peer_url(&url, &peer.peer_id)?;
    Ok(PairingInvite {
        version: 1,
        peer_id: peer.peer_id.clone(),
        secret: peer.secret.clone(),
        url,
        issued_at_unix,
        expires_at_unix,
    })
}

pub fn suggested_host(use_upnp: bool) -> Result<String, String> {
    if use_upnp {
        let gateway = igd_next::search_gateway(igd_next::SearchOptions::default())
            .map_err(|error| format!("could not find a UPnP router: {error}"))?;
        return gateway
            .get_external_ip()
            .map(|address| address.to_string())
            .map_err(|error| format!("could not read the router's public address: {error}"));
    }
    let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| e.to_string())?;
    probe
        .connect(SocketAddr::from(([192, 0, 2, 1], 9)))
        .map_err(|e| e.to_string())?;
    let address = probe.local_addr().map_err(|e| e.to_string())?.ip();
    if address.is_unspecified() || address.is_loopback() {
        return Err("could not determine a reachable local network address".into());
    }
    Ok(address.to_string())
}

pub fn import_invite(value: &str, label: String) -> Result<PeerCredential, String> {
    if value.trim().len() > 2048 {
        return Err("party invite is too large".into());
    }
    let encoded = value
        .trim()
        .strip_prefix(INVITE_PREFIX)
        .ok_or("this is not a supported WizRust101-OBS party invite")?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "party invite is malformed")?;
    let invite: PairingInvite =
        serde_json::from_slice(&bytes).map_err(|_| "party invite is malformed")?;
    if invite.version != 1 {
        return Err("this party invite version is not supported".into());
    }
    if invite.peer_id.is_empty() || invite.peer_id.len() > 64 || invite.peer_id == HOST_MEMBER_ID {
        return Err("invite participant identity is invalid".into());
    }
    let now = unix_now();
    if invite.issued_at_unix > now.saturating_add(300)
        || invite.expires_at_unix <= now
        || invite.expires_at_unix <= invite.issued_at_unix
        || invite.expires_at_unix - invite.issued_at_unix > INVITE_TTL_SECS
    {
        return Err("this party invite has expired or has an invalid lifetime".into());
    }
    validate_peer_url(&invite.url, &invite.peer_id)?;
    let secret = URL_SAFE_NO_PAD
        .decode(&invite.secret)
        .map_err(|_| "invite secret is invalid")?;
    if secret.len() != 32 {
        return Err("invite secret is invalid".into());
    }
    if label.len() > 80 {
        return Err("party member label is too long".into());
    }
    Ok(PeerCredential {
        peer_id: invite.peer_id,
        secret: invite.secret,
        connect_url: Some(invite.url),
        label,
        expires_at_unix: Some(invite.expires_at_unix),
    })
}

pub fn encode_invite(invite: &PairingInvite) -> Result<String, String> {
    let encoded = serde_json::to_vec(invite).map_err(|_| "could not encode party invite")?;
    Ok(format!(
        "{INVITE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(encoded)
    ))
}

pub fn validate_peer_url(value: &str, peer_id: &str) -> Result<(), String> {
    if value.len() > 512 {
        return Err("party invite address is too long".into());
    }
    let url = url::Url::parse(value).map_err(|_| "party invite address is invalid")?;
    let query: Vec<_> = url.query_pairs().collect();
    let valid = url.scheme() == "ws"
        && url.host_str().is_some()
        && !matches!(url.host(), Some(url::Host::Ipv6(_)))
        && url.port().is_some_and(|port| port >= 1024)
        && url.path() == "/peer"
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && query.len() == 1
        && query[0].0 == "peer_id"
        && query[0].1 == peer_id;
    if valid {
        Ok(())
    } else {
        Err("party invite address is incompatible".into())
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn valid_presence(p: &WizardPresence) -> bool {
    !p.peer_id.trim().is_empty()
        && p.peer_id.len() <= 64
        && !p.name.trim().is_empty()
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
    let peer_port = { shared.config.lock().unwrap().peer_port };
    let listener = match TcpListener::bind((Ipv4Addr::UNSPECIFIED, peer_port)).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Peer listener could not start: {error}");
            return;
        }
    };
    let mut gateway = if shared.config.lock().unwrap().upnp_port_forward {
        match tokio::task::spawn_blocking(move || add_mapping(peer_port)).await {
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
                    if let Some(g)=gateway.take(){let _=tokio::task::spawn_blocking(move||remove_mapping(g, peer_port)).await;}
                    gateway=match tokio::task::spawn_blocking(move || add_mapping(peer_port)).await{Ok(Ok(g))=>Some(g),_=>None};
                }
                let (enabled, upnp, changed_port) = { let c=shared.config.lock().unwrap(); (c.collaboration_server_enabled, c.upnp_port_forward, c.peer_port != peer_port) }; if !enabled || changed_port || !upnp {break;}
            }
            _=tokio::time::sleep(Duration::from_millis(500))=>{
                let (enabled, upnp, changed_port) = { let c=shared.config.lock().unwrap(); (c.collaboration_server_enabled, c.upnp_port_forward, c.peer_port != peer_port) }; if !enabled || changed_port {break;}
                if !upnp && gateway.is_some(){if let Some(g)=gateway.take(){let _=tokio::task::spawn_blocking(move||remove_mapping(g, peer_port)).await;}}
                else if upnp && gateway.is_none(){gateway=match tokio::task::spawn_blocking(move || add_mapping(peer_port)).await{Ok(Ok(g))=>Some(g),_=>None};}
            }
            _=&mut server=>break,
        }
    }
    let _ = shutdown.send(());
    server.abort();
    if let Some(g) = gateway {
        let _ = tokio::task::spawn_blocking(move || remove_mapping(g, peer_port)).await;
    }
}

fn peer_router(shared: SharedState, shutdown: broadcast::Sender<()>) -> Router {
    Router::new()
        .route("/peer", get(peer_upgrade))
        .with_state(Arc::new(PeerServerState {
            shared,
            capacity: Arc::new(Semaphore::new(8)),
            shutdown,
            active_members: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        }))
}

fn add_mapping(peer_port: u16) -> Result<igd_next::Gateway, String> {
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
            peer_port,
            SocketAddr::new(local.ip(), peer_port),
            3600,
            "WizRust101-OBS peer presence",
        )
        .map_err(|e| e.to_string())?;
    Ok(gateway)
}
fn remove_mapping(gateway: igd_next::Gateway, peer_port: u16) {
    let _ = gateway.remove_port(igd_next::PortMappingProtocol::TCP, peer_port);
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
    if !server
        .active_members
        .lock()
        .unwrap()
        .insert(credential.peer_id.clone())
    {
        return StatusCode::CONFLICT.into_response();
    }
    let active_members = server.active_members.clone();
    let active_member = ActiveMember {
        id: credential.peer_id.clone(),
        members: active_members,
    };
    let shared = server.shared.clone();
    let id = credential.peer_id;
    let shutdown = server.shutdown.subscribe();
    ws.max_message_size(MAX_CIPHERTEXT)
        .max_frame_size(MAX_CIPHERTEXT)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            let _active_id = active_member;
            serve_socket(socket, shared, id, secret, false, shutdown).await;
        })
}

struct ActiveMember {
    id: String,
    members: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}
impl Drop for ActiveMember {
    fn drop(&mut self) {
        self.members.lock().unwrap().remove(&self.id);
    }
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
    if credential
        .expires_at_unix
        .is_some_and(|expires| expires <= unix_now())
    {
        return None;
    }
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
    if !send_peer_frame(
        &mut sink,
        &mut transport,
        &PeerFrame::Welcome {
            member_id: peer_id.clone(),
        },
    )
    .await
    {
        return;
    }
    let mut updates = shared.subscribe();
    let mut presence_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(1),
        Duration::from_secs(1),
    );
    if !send_party_snapshot(&mut sink, &mut transport, &shared).await {
        return;
    }
    let mut last = Instant::now() - Duration::from_secs(1);
    loop {
        tokio::select! {
            _ = presence_tick.tick() => {
                if last.elapsed() > Duration::from_secs(8) || !send_party_snapshot(&mut sink, &mut transport, &shared).await { break; }
            },
            _ = shutdown.recv() => break,
            incoming=stream.next()=>match incoming{
                Some(Ok(Message::Binary(data))) if data.len()<=MAX_CIPHERTEXT=>{
                    let mut plain=[0_u8;MAX_CIPHERTEXT];let Ok(n)=transport.read_message(&data,&mut plain) else{break};
                    if last.elapsed()<Duration::from_millis(500){continue;}last=Instant::now();
                    let Ok(PeerFrame::Join(mut presence))=serde_json::from_slice(&plain[..n]) else{break};
                    presence.peer_id=peer_id.clone();if !valid_presence(&presence){break;}shared.set_peer_presence(presence);
                }
                Some(Ok(Message::Ping(p)))=>if sink.send(Message::Pong(p)).await.is_err(){break},
                Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,
                _=>break,
            },
            update=updates.recv()=>match update{
                Ok(_)=>if !send_party_snapshot(&mut sink,&mut transport,&shared).await{break},
                Err(broadcast::error::RecvError::Lagged(_))=>if !send_party_snapshot(&mut sink,&mut transport,&shared).await{break},
                Err(_)=>break,
            }
        }
    }
    shared.remove_peer(&peer_id);
}

async fn send_party_snapshot(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    transport: &mut TransportState,
    shared: &SharedState,
) -> bool {
    let Some(members) = party_members(shared) else {
        return false;
    };
    send_peer_frame(sink, transport, &PeerFrame::PartySnapshot { members }).await
}

fn party_members(shared: &SharedState) -> Option<Vec<WizardPresence>> {
    let mut host = shared.local_presence()?;
    host.peer_id = HOST_MEMBER_ID.into();
    let mut members = vec![host];
    members.extend(shared.snapshot().party);
    (members.len() <= 9).then_some(members)
}

async fn send_peer_frame(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    transport: &mut TransportState,
    frame: &PeerFrame,
) -> bool {
    let Ok(payload) = serde_json::to_vec(frame) else {
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
                shared.clear_party();
                false
            } else {
                !handle.is_finished()
            }
        });
        for credential in config.peer_links.into_iter().filter(|p| {
            p.connect_url.is_some() && p.expires_at_unix.is_none_or(|expires| expires > unix_now())
        }) {
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
    let url = match url::Url::parse(base) {
        Ok(u) => u,
        Err(_) => return,
    };
    let Ok(secret_vec) = URL_SAFE_NO_PAD.decode(&credential.secret) else {
        return;
    };
    let Ok(psk): Result<[u8; 32], _> = secret_vec.try_into() else {
        return;
    };
    loop {
        if credential
            .expires_at_unix
            .is_some_and(|expires| expires <= unix_now())
        {
            return;
        }
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
        shared.clear_party();
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
    let mut presence_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(1),
        Duration::from_secs(1),
    );
    if !send_client_presence(socket, shared, credential, transport).await {
        return;
    }
    let mut welcomed = false;
    let mut last_snapshot = Instant::now();
    loop {
        tokio::select! {
            _=presence_tick.tick()=>{
                if last_snapshot.elapsed() > Duration::from_secs(8) || !send_client_presence(socket,shared,credential,transport).await { break; }
            },
            msg=socket.next()=>match msg {
                Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(data))) if data.len()<=MAX_CIPHERTEXT => {
                    let mut plain=[0_u8;MAX_CIPHERTEXT];
                    let Ok(n)=transport.read_message(&data,&mut plain) else { break };
                    let Ok(frame)=serde_json::from_slice::<PeerFrame>(&plain[..n]) else { break };
                    match frame {
                        PeerFrame::Welcome { member_id } if member_id==credential.peer_id => welcomed=true,
                        PeerFrame::PartySnapshot { members } if welcomed => {
                            if !shared.replace_party(members, &credential.peer_id) { break; }
                            last_snapshot = Instant::now();
                        }
                        _ => break,
                    }
                },
                Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(p)))=>{let _=socket.send(tokio_tungstenite::tungstenite::Message::Pong(p)).await;},
                Some(_)|None=>break,
            },
        }
    }
    shared.clear_party();
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
    let Ok(payload) = serde_json::to_vec(&PeerFrame::Join(p)) else {
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
    use crate::config::{AppConfig, CharacterProfile};
    #[test]
    fn invites_validate_secret_and_url() {
        let (mut link, key) = create_pairing("peer-a".into()).unwrap();
        link.secret = URL_SAFE_NO_PAD.encode(key);
        let invite = invite_for(&link, "192.0.2.1", PEER_PORT).unwrap();
        let code = encode_invite(&invite).unwrap();
        let imported = import_invite(&code, "friend".into()).unwrap();
        assert_eq!(imported.peer_id, link.peer_id);
        assert!(valid_secret(&imported.secret));
        assert_eq!(imported.connect_url.as_deref(), Some(invite.url.as_str()));
    }
    #[test]
    fn host_refuses_expired_invite_credentials() {
        let mut config = AppConfig::default();
        config.peer_links.push(PeerCredential {
            peer_id: "member-expired".into(),
            secret: URL_SAFE_NO_PAD.encode([9_u8; 32]),
            expires_at_unix: Some(unix_now().saturating_sub(1)),
            ..Default::default()
        });
        let shared = SharedState::new(config);
        assert!(paired_credential(&shared, "member-expired").is_none());
    }
    #[test]
    fn expired_malformed_and_legacy_invites_are_rejected() {
        let (credential, _) = create_pairing("member-a".into()).unwrap();
        let mut invite = invite_for(&credential, "192.0.2.4", PEER_PORT).unwrap();
        invite.expires_at_unix = unix_now().saturating_sub(1);
        assert!(
            import_invite(&encode_invite(&invite).unwrap(), String::new())
                .unwrap_err()
                .contains("expired")
        );
        invite.expires_at_unix = unix_now() + INVITE_TTL_SECS + 1;
        assert!(import_invite(&encode_invite(&invite).unwrap(), String::new()).is_err());
        assert!(import_invite("{\"peer_id\":\"old-m2-invite\"}", String::new()).is_err());
        assert!(import_invite("WIZPARTY1.not-base64", String::new()).is_err());
    }
    #[test]
    fn invite_address_must_match_member_and_use_peer_route() {
        let (mut credential, _) = create_pairing("member-a".into()).unwrap();
        credential.expires_at_unix = Some(unix_now() + INVITE_TTL_SECS);
        assert!(invite_for(&credential, "192.0.2.3/evil", PEER_PORT).is_err());
        assert!(invite_for(&credential, "192.0.2.3", 80).is_err());
        let mut invite = invite_for(&credential, "192.0.2.3", PEER_PORT).unwrap();
        invite.url = invite.url.replace("member-a", "somebody-else");
        assert!(encode_invite(&invite).is_ok());
        assert!(import_invite(&encode_invite(&invite).unwrap(), String::new()).is_err());
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
    fn host_relay_snapshots_are_full_and_clients_keep_their_own_wizard_primary() {
        use crate::config::{AppConfig, CharacterProfile};
        let mut host_config = AppConfig::default();
        host_config.profiles.push(CharacterProfile {
            id: "host-profile".into(),
            name: "Host Wizard".into(),
            school: "Fire".into(),
            ..Default::default()
        });
        host_config.active_profile = Some("host-profile".into());
        let host = SharedState::new(host_config);
        host.set_demo_state("Wizard City", "The Commons", "WC_Hub");
        for (id, name, school) in [
            ("member-a", "First Guest", "Life"),
            ("member-b", "Second Guest", "Storm"),
        ] {
            host.set_peer_presence(WizardPresence {
                peer_id: id.into(),
                name: name.into(),
                school: school.into(),
                active: true,
                world: Some("Krokotopia".into()),
                zone: Some("The Oasis".into()),
                session_seconds: 19,
            });
        }
        let members = party_members(&host).unwrap();
        assert_eq!(members.len(), 3);
        assert_eq!(members[0].peer_id, HOST_MEMBER_ID);

        let mut client_config = AppConfig::default();
        client_config.profiles.push(CharacterProfile {
            id: "client-profile".into(),
            name: "Local Guest".into(),
            school: "Ice".into(),
            ..Default::default()
        });
        client_config.active_profile = Some("client-profile".into());
        let client = SharedState::new(client_config);
        assert!(client.replace_party(members, "member-a"));
        let client_view = client.snapshot();
        assert_eq!(client_view.wizard.as_ref().unwrap().name, "Local Guest");
        assert_eq!(client_view.party.len(), 2);
        assert!(
            client_view
                .party
                .iter()
                .any(|member| member.name == "Host Wizard")
        );
        assert!(
            client_view
                .party
                .iter()
                .any(|member| member.name == "Second Guest")
        );
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

    #[tokio::test]
    async fn host_forwards_live_roster_updates_to_other_joined_clients() {
        use tokio_tungstenite::tungstenite::Message as ClientMessage;

        let mut config = AppConfig::default();
        config.profiles.push(CharacterProfile {
            id: "host-profile".into(),
            name: "Host Wizard".into(),
            school: "Fire".into(),
            ..Default::default()
        });
        config.active_profile = Some("host-profile".into());
        let mut invite_keys = Vec::new();
        for id in ["member-a", "member-b"] {
            let (credential, key) = create_pairing(id.into()).unwrap();
            config.peer_links.push(credential);
            invite_keys.push(key);
        }
        let host = SharedState::new(config);
        host.set_demo_state("Wizard City", "The Commons", "WC_Hub");
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (shutdown, _) = broadcast::channel(1);
        let app = peer_router(host.clone(), shutdown.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let mut first = connect_test_peer(address, "member-a", invite_keys[0]).await;
        assert!(matches!(
            read_test_frame(&mut first).await,
            PeerFrame::Welcome { .. }
        ));
        assert!(matches!(
            read_test_frame(&mut first).await,
            PeerFrame::PartySnapshot { .. }
        ));
        send_test_join(&mut first, "member-a", "First Guest", "Life").await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while host.snapshot().party.len() != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        let mut second = connect_test_peer(address, "member-b", invite_keys[1]).await;
        assert!(matches!(
            read_test_frame(&mut second).await,
            PeerFrame::Welcome { .. }
        ));
        let PeerFrame::PartySnapshot { members } = read_test_frame(&mut second).await else {
            panic!("expected initial roster snapshot");
        };
        assert!(members.iter().any(|member| member.name == "First Guest"));
        send_test_join(&mut second, "member-b", "Second Guest", "Storm").await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while host.snapshot().party.len() != 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        let update = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let PeerFrame::PartySnapshot { members } = read_test_frame(&mut first).await
                    && members.iter().any(|member| member.name == "Second Guest")
                {
                    break members;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(update.len(), 3);
        assert!(update.iter().any(|member| member.peer_id == HOST_MEMBER_ID));

        second.0.send(ClientMessage::Close(None)).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while host.snapshot().party.len() != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let after_leave = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let PeerFrame::PartySnapshot { members } = read_test_frame(&mut first).await
                    && !members.iter().any(|member| member.name == "Second Guest")
                {
                    break members;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(after_leave.len(), 2);

        let _ = first.0.send(ClientMessage::Close(None)).await;
        let _ = shutdown.send(());
        server.abort();
    }

    type TestSocket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;
    type TestPeer = (TestSocket, TransportState);

    async fn connect_test_peer(address: SocketAddr, id: &str, key: [u8; 32]) -> TestPeer {
        use tokio_tungstenite::tungstenite::Message as ClientMessage;
        let url = format!("ws://{address}/peer?peer_id={id}");
        let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        let params: NoiseParams = PROTOCOL.parse().unwrap();
        let mut handshake = Builder::new(params)
            .psk(0, &key)
            .unwrap()
            .prologue(id.as_bytes())
            .unwrap()
            .build_initiator()
            .unwrap();
        let mut buffer = [0_u8; MAX_CIPHERTEXT];
        let n = handshake.write_message(&[], &mut buffer).unwrap();
        socket
            .send(ClientMessage::Binary(buffer[..n].to_vec().into()))
            .await
            .unwrap();
        let Some(Ok(ClientMessage::Binary(reply))) = timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
        else {
            panic!("expected Noise handshake response");
        };
        let mut plain = [0_u8; MAX_CIPHERTEXT];
        handshake.read_message(&reply, &mut plain).unwrap();
        (socket, handshake.into_transport_mode().unwrap())
    }

    async fn read_test_frame(peer: &mut TestPeer) -> PeerFrame {
        use tokio_tungstenite::tungstenite::Message as ClientMessage;
        let Some(Ok(ClientMessage::Binary(data))) = timeout(Duration::from_secs(2), peer.0.next())
            .await
            .unwrap()
        else {
            panic!("expected encrypted peer frame");
        };
        let mut plain = [0_u8; MAX_CIPHERTEXT];
        let size = peer.1.read_message(&data, &mut plain).unwrap();
        serde_json::from_slice(&plain[..size]).unwrap()
    }

    async fn send_test_join(peer: &mut TestPeer, id: &str, name: &str, school: &str) {
        use tokio_tungstenite::tungstenite::Message as ClientMessage;
        let payload = serde_json::to_vec(&PeerFrame::Join(WizardPresence {
            peer_id: id.into(),
            name: name.into(),
            school: school.into(),
            active: true,
            world: Some("Krokotopia".into()),
            zone: Some("The Oasis".into()),
            session_seconds: 5,
        }))
        .unwrap();
        let mut encrypted = [0_u8; MAX_CIPHERTEXT];
        let size = peer.1.write_message(&payload, &mut encrypted).unwrap();
        peer.0
            .send(ClientMessage::Binary(encrypted[..size].to_vec().into()))
            .await
            .unwrap();
    }
}
