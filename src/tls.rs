use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

/// SHA-256 of the TV's leaf certificate.
///
/// Every webOS TV serves its own self-signed cert, so no CA can vouch for it.
/// Pinning this is the only thing standing between the client key and anyone
/// else on the LAN pretending to be the TV.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CertFingerprint([u8; 32]);

impl CertFingerprint {
    pub fn of(cert: &CertificateDer<'_>) -> Self {
        Self(Sha256::digest(cert.as_ref()).into())
    }
}

impl fmt::Display for CertFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(":")?;
            }
            write!(f, "{byte:02X}")?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("expected 32 hex bytes, optionally colon separated")]
pub struct InvalidFingerprint;

impl FromStr for CertFingerprint {
    type Err = InvalidFingerprint;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex: String = s.chars().filter(|c| *c != ':').collect();
        if hex.len() != 64 {
            return Err(InvalidFingerprint);
        }
        let mut bytes = [0u8; 32];
        for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks(2)) {
            let pair = std::str::from_utf8(pair).map_err(|_| InvalidFingerprint)?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| InvalidFingerprint)?;
        }
        Ok(Self(bytes))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CertPolicy {
    Pinned(CertFingerprint),
    /// Only for pairing, before there's a fingerprint to pin.
    AcceptAny,
}

#[derive(Debug)]
struct Verifier {
    policy: CertPolicy,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.policy {
            CertPolicy::Pinned(pin) if CertFingerprint::of(end_entity) != pin => {
                Err(rustls::Error::General(format!(
                    "TV certificate is {}, pinned {pin}",
                    CertFingerprint::of(end_entity)
                )))
            }
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub(crate) fn connector(policy: CertPolicy) -> tokio_tungstenite::Connector {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Verifier { policy, provider }))
        .with_no_client_auth();
    tokio_tungstenite::Connector::Rustls(Arc::new(config))
}
