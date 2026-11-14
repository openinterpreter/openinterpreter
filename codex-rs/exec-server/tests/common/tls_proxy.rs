//! Local TLS CONNECT proxy shared by executor integration tests.

use std::collections::BTreeMap;
use std::io;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rcgen::BasicConstraints;
use rcgen::CertificateParams;
use rcgen::CertifiedIssuer;
use rcgen::DistinguishedName;
use rcgen::DnType;
use rcgen::ExtendedKeyUsagePurpose;
use rcgen::IsCa;
use rcgen::KeyPair;
use rcgen::KeyUsagePurpose;
use rcgen::PKCS_ECDSA_P256_SHA256;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;

#[derive(Debug)]
pub(crate) struct CapturedRequest {
    pub(crate) connect_authority: String,
    pub(crate) request_line: String,
    pub(crate) headers: BTreeMap<String, Vec<String>>,
}

struct TlsMaterial {
    ca_cert_pem: String,
    server_cert: CertificateDer<'static>,
    server_key: PrivateKeyDer<'static>,
}

pub(crate) struct TlsInterceptingProxy {
    pub(crate) ca_cert_pem: String,
    request_rx: mpsc::Receiver<Result<CapturedRequest, String>>,
    thread: thread::JoinHandle<Result<(), String>>,
    pub(crate) url: String,
}

impl TlsInterceptingProxy {
    pub(crate) fn start(expected_requests: usize) -> anyhow::Result<Self> {
        codex_utils_rustls_provider::ensure_rustls_crypto_provider();
        let material = generate_tls_material()?;
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let config = Arc::new(
            rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_no_client_auth()
                .with_single_cert(vec![material.server_cert], material.server_key)?,
        );
        let (request_tx, request_rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            run_tls_intercepting_proxy(listener, config, request_tx, expected_requests)
                .map_err(|error| error.to_string())
        });

        Ok(Self {
            ca_cert_pem: material.ca_cert_pem,
            request_rx,
            thread,
            url: format!("http://{address}"),
        })
    }

    pub(crate) fn next_request(&self) -> anyhow::Result<CapturedRequest> {
        self.request_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(anyhow::Error::from)?
            .map_err(anyhow::Error::msg)
    }

    pub(crate) fn finish(self) -> anyhow::Result<()> {
        self.thread
            .join()
            .map_err(|_| anyhow::anyhow!("TLS proxy thread panicked"))?
            .map_err(anyhow::Error::msg)
    }
}

fn generate_tls_material() -> anyhow::Result<TlsMaterial> {
    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let mut ca_distinguished_name = DistinguishedName::new();
    ca_distinguished_name.push(DnType::CommonName, "Codex affinity test CA");
    ca_params.distinguished_name = ca_distinguished_name;
    let ca_key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    let ca = CertifiedIssuer::self_signed(ca_params, ca_key_pair)?;

    let mut server_params = CertificateParams::new(vec![
        "chatgpt.com".to_string(),
        "api.openai.com".to_string(),
    ])?;
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    server_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    let server_key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    let server_cert = server_params.signed_by(&server_key_pair, &ca)?;

    Ok(TlsMaterial {
        ca_cert_pem: ca.pem(),
        server_cert: server_cert.der().clone(),
        server_key: PrivateKeyDer::from(server_key_pair),
    })
}

fn run_tls_intercepting_proxy(
    listener: TcpListener,
    config: Arc<rustls::ServerConfig>,
    request_tx: mpsc::Sender<Result<CapturedRequest, String>>,
    expected_requests: usize,
) -> io::Result<()> {
    for request_index in 0..expected_requests {
        let (mut stream, _) = listener.accept()?;
        configure_stream(&stream)?;
        let connect_head = read_http_head(&mut stream)?;
        let connect_authority = connect_head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .ok_or_else(|| io::Error::other("CONNECT request line should include an authority"))?
            .to_string();
        stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
        stream.flush()?;

        let connection =
            rustls::ServerConnection::new(Arc::clone(&config)).map_err(io::Error::other)?;
        let mut tls = rustls::StreamOwned::new(connection, stream);
        let request = capture_http_request(&mut tls, connect_authority);
        match request {
            Ok(request) => request_tx
                .send(Ok(request))
                .map_err(|_| io::Error::other("request receiver was dropped"))?,
            Err(error) => {
                let message = error.to_string();
                let _ = request_tx.send(Err(message));
                return Err(error);
            }
        }

        let response = match request_index {
            0 => concat!(
                "HTTP/1.1 200 OK\r\n",
                "content-length: 2\r\n",
                "connection: close\r\n",
                "set-cookie: __cflb=west; Path=/; Secure; HttpOnly\r\n",
                "set-cookie: chatgpt_session=secret; Path=/; Secure; HttpOnly\r\n",
                "\r\n",
                "ok",
            ),
            1 => concat!(
                "HTTP/1.1 200 OK\r\n",
                "content-length: 2\r\n",
                "connection: close\r\n",
                "set-cookie: __cflb=central; Path=/; Secure; HttpOnly\r\n",
                "set-cookie: chatgpt_session=still-secret; Path=/; Secure; HttpOnly\r\n",
                "\r\n",
                "ok",
            ),
            _ => concat!(
                "HTTP/1.1 200 OK\r\n",
                "content-length: 2\r\n",
                "connection: close\r\n",
                "\r\n",
                "ok",
            ),
        };
        tls.write_all(response.as_bytes())?;
        tls.flush()?;
    }
    Ok(())
}

fn configure_stream(stream: &TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))
}

fn capture_http_request(
    stream: &mut impl Read,
    connect_authority: String,
) -> io::Result<CapturedRequest> {
    let request_head = read_http_head(stream)?;
    let mut lines = request_head.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| io::Error::other("HTTP request should include a request line"))?
        .to_string();
    let mut headers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::other(format!("invalid HTTP header: {line}")))?;
        headers
            .entry(name.to_ascii_lowercase())
            .or_default()
            .push(value.trim().to_string());
    }
    Ok(CapturedRequest {
        connect_authority,
        request_line,
        headers,
    })
}

fn read_http_head(stream: &mut impl Read) -> io::Result<String> {
    const MAX_HEADER_BYTES: usize = 64 * 1024;
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() == MAX_HEADER_BYTES {
            return Err(io::Error::other("HTTP headers exceeded test limit"));
        }
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).map_err(io::Error::other)
}
