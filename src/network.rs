use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use quinn::{ClientConfig as QuinnClientConfig, Endpoint, ServerConfig};
use quinn::crypto::rustls::QuicClientConfig as QuinnRustlsClientConfig;
use rcgen::generate_simple_self_signed;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerifier, ServerCertVerified};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime};
use rustls::{ClientConfig as RustlsClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::interval;

use crate::action::Action;
use crate::physics::PhysicsLayer;
use crate::protocol::ProtocolMessage;

/// Creates a QUIC endpoint bound to `addr` that can both accept incoming connections and initiate
/// outgoing ones.
///
/// Why QUIC / `quinn`?
/// - QUIC gives us multiplexed streams with TLS built-in, avoiding head-of-line blocking and making
///   it natural to model “messages” as uni-directional streams.
/// - `quinn` is a mature async QUIC implementation in Rust that integrates cleanly with Tokio.
///
/// Security trade-off (intentional for simulation):
/// - We generate a fresh self-signed certificate and configure the client side to *skip certificate
///   verification*. This keeps local multi-node simulations frictionless (no PKI ceremony), but it
///   is **not** appropriate for real networks.
pub fn make_server_endpoint(addr: &str) -> Result<Endpoint> {
    let server_config = make_server_config()?;
    let addr: SocketAddr = addr.parse()?;
    let mut endpoint = Endpoint::server(server_config, addr)?;

    // Simulation convenience: accept self-signed certs without verification.
    let mut client_config = RustlsClientConfig::builder()
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();
    client_config
        .dangerous()
        .set_certificate_verifier(Arc::new(SkipServerVerification));

    let client_crypto = QuinnRustlsClientConfig::try_from(Arc::new(client_config))?;
    endpoint.set_default_client_config(QuinnClientConfig::new(Arc::new(client_crypto)));
    Ok(endpoint)
}

fn make_server_config() -> Result<ServerConfig> {
    let cert = generate_simple_self_signed(["localhost".to_string()])?;
    let cert_der: CertificateDer<'static> = CertificateDer::from(cert.cert.der().clone());
    let key_der: PrivateKeyDer<'static> = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into();

    let mut server_config = quinn::ServerConfig::with_single_cert(vec![cert_der], key_der)?;
    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(Duration::from_secs(10)));
    server_config.transport_config(Arc::new(transport));

    Ok(server_config)
}

#[derive(Debug)]
struct SkipServerVerification;

impl ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PKCS1_SHA256,
        ]
    }
}

pub struct Network {
    /// QUIC endpoint used for both listening and dialing.
    pub endpoint: Endpoint,
    /// Shared physics gate that buffers messages until their causal arrival time.
    pub physics: Arc<tokio::sync::Mutex<PhysicsLayer>>,
    /// Channel back into the application event loop.
    pub app_tx: UnboundedSender<Action>,
    /// Local (x, y) position in meters (2D simplification used by the current simulation).
    pub my_coords: (f64, f64),
}

impl Network {
    /// Constructs the network task.
    ///
    /// The network layer’s responsibility is deliberately narrow:
    /// 1) move bytes via QUIC,
    /// 2) decode protocol messages,
    /// 3) compute sender/receiver separation,
    /// 4) pass messages into `PhysicsLayer` so causality is enforced *outside* the transport.
    pub fn new(endpoint: Endpoint, physics: Arc<tokio::sync::Mutex<PhysicsLayer>>, app_tx: UnboundedSender<Action>, my_coords: (f64, f64)) -> Self {
        Self { endpoint, physics, app_tx, my_coords }
    }

    /// Runs the network loop.
    ///
    /// This loop interleaves two concerns:
    /// - Accept inbound QUIC connections and ingest any received protocol messages.
    /// - Periodically poll the physics buffer and forward any causally-arrived messages to the app.
    ///
    /// Design note: QUIC delivery is *not* treated as “arrival”. Arrival is defined by the
    /// relativistic model: events outside the light cone must be buffered until
    /// $t_{arrival} = t_{received} + d/c$.
    pub async fn run(self) -> Result<()> {
        let mut tick_interval = interval(Duration::from_millis(50));
        loop {
            tokio::select! {
                _ = tick_interval.tick() => {
                    let mut physics = self.physics.lock().await;
                    for msg in physics.drain_arrived() {
                        let _ = self.app_tx.send(Action::NewEvent(msg));
                    }
                }
                connecting = self.endpoint.accept() => {
                    if let Some(connecting) = connecting {
                        let physics = self.physics.clone();
                        let app_tx = self.app_tx.clone();
                        let my_coords = self.my_coords;
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(connecting, physics.clone(), my_coords).await {
                                eprintln!("[network] connection error: {e:?}");
                            }
                            let mut physics = physics.lock().await;
                            for msg in physics.drain_arrived() {
                                let _ = app_tx.send(Action::NewEvent(msg));
                            }
                        });
                    }
                }
            }
        }
    }
}

async fn handle_connection(connecting: quinn::Incoming, physics: Arc<tokio::sync::Mutex<PhysicsLayer>>, my_coords: (f64, f64)) -> Result<()> {
    let connection = connecting.await?;
    println!("[network] connected: {}", connection.remote_address());

    while let Ok(mut uni) = connection.accept_uni().await {
        let data = uni.read_to_end(64 * 1024).await?;
        let msg: ProtocolMessage = bincode::deserialize(&data)?;
        let dist = match &msg {
            ProtocolMessage::Gossip(event) => {
                let dx = event.coords.x - my_coords.0;
                let dy = event.coords.y - my_coords.1;
                (dx * dx + dy * dy).sqrt()
            }
            _ => 0.0,
        };
        let mut physics = physics.lock().await;
        println!("[network] ingest message: {:?} (dist={:.2})", msg, dist);
        physics.ingest(msg, dist);
    }

    Ok(())
}

#[derive(Clone)]
pub struct NetworkHandle {
    endpoint: Endpoint,
}

impl NetworkHandle {
    /// Convenience wrapper for sending protocol messages using the shared endpoint.
    pub fn new(endpoint: Endpoint) -> Self {
        Self { endpoint }
    }

    /// Sends a gossip message to a local target.
    ///
    /// This is intentionally minimal: Minkowski-KV’s “interesting” behavior is in the DAG and the
    /// physics gate, not in elaborate transport routing.
    pub async fn send_gossip(&self, target_port: u16, msg: ProtocolMessage) -> Result<()> {
        let addr: SocketAddr = format!("127.0.0.1:{target_port}").parse()?;
        let conn = self.endpoint.connect(addr, "localhost")?.await?;
        let mut stream = conn.open_uni().await?;
        let bytes = bincode::serialize(&msg)?;
        stream.write_all(&bytes).await?;
        stream.finish()?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok(())
    }
}
