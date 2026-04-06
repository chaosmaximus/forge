// tls.rs — Self-signed TLS certificate generation and loading for localhost.
//
// When TLS is enabled, the daemon serves HTTPS on localhost using a self-signed
// certificate. Certs are stored in ~/.forge/tls/ and reused across restarts.

use std::fs;
use std::io::BufReader;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::sync::Arc;

use rcgen::{CertificateParams, KeyPair, SanType};
use rustls::ServerConfig;
use rustls_pemfile::{certs, pkcs8_private_keys};

/// Directory name under the forge home for TLS artifacts.
const TLS_DIR: &str = "tls";
const CERT_FILENAME: &str = "localhost.crt";
const KEY_FILENAME: &str = "localhost.key";

// ---------------------------------------------------------------------------
// Certificate generation / loading
// ---------------------------------------------------------------------------

/// Ensure a self-signed TLS certificate exists for localhost.
///
/// If `~/.forge/tls/localhost.crt` and `~/.forge/tls/localhost.key` already
/// exist, their paths are returned immediately. Otherwise a new self-signed
/// certificate is generated using `rcgen` and written to disk.
///
/// This overload uses the default forge home directory (`~/.forge`).
pub fn ensure_certs() -> Result<(PathBuf, PathBuf), String> {
    let home = dirs_for_forge_home()?;
    ensure_certs_in(home)
}

/// Same as [`ensure_certs`] but accepts an explicit base directory.
/// Useful for testing with a temp directory.
pub fn ensure_certs_in(base_dir: PathBuf) -> Result<(PathBuf, PathBuf), String> {
    let tls_dir = base_dir.join(TLS_DIR);
    let cert_path = tls_dir.join(CERT_FILENAME);
    let key_path = tls_dir.join(KEY_FILENAME);

    // Already generated — return existing paths.
    if cert_path.exists() && key_path.exists() {
        return Ok((cert_path, key_path));
    }

    // Create directory if needed.
    fs::create_dir_all(&tls_dir)
        .map_err(|e| format!("failed to create TLS directory {}: {}", tls_dir.display(), e))?;

    // Generate a new key pair and self-signed cert.
    let key_pair = KeyPair::generate()
        .map_err(|e| format!("failed to generate key pair: {e}"))?;

    let mut params = CertificateParams::default();
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String("Forge Localhost".into()),
    );
    params.distinguished_name.push(
        rcgen::DnType::OrganizationName,
        rcgen::DnValue::Utf8String("Forge".into()),
    );
    params.subject_alt_names = vec![
        SanType::DnsName("localhost".try_into().map_err(|e| format!("SAN DNS error: {e}"))?),
        SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        SanType::IpAddress(IpAddr::V6(Ipv6Addr::LOCALHOST)),
    ];

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| format!("failed to self-sign certificate: {e}"))?;

    // Write PEM files.
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    fs::write(&cert_path, cert_pem.as_bytes())
        .map_err(|e| format!("failed to write cert to {}: {e}", cert_path.display()))?;
    fs::write(&key_path, key_pem.as_bytes())
        .map_err(|e| format!("failed to write key to {}: {e}", key_path.display()))?;

    // Restrict key file permissions on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&key_path, perms)
            .map_err(|e| format!("failed to set key permissions: {e}"))?;
    }

    Ok((cert_path, key_path))
}

/// Build a `rustls::ServerConfig` from PEM certificate and key files.
pub fn build_rustls_config(
    cert_path: PathBuf,
    key_path: PathBuf,
) -> Result<Arc<ServerConfig>, String> {
    // Ensure a crypto provider is installed (idempotent — ignores if already set).
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Read and parse certificate chain.
    let cert_file = fs::File::open(&cert_path)
        .map_err(|e| format!("failed to open cert {}: {e}", cert_path.display()))?;
    let mut cert_reader = BufReader::new(cert_file);
    let cert_chain: Vec<_> = certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to parse certificates: {e}"))?;

    if cert_chain.is_empty() {
        return Err("no certificates found in PEM file".into());
    }

    // Read and parse private key.
    let key_file = fs::File::open(&key_path)
        .map_err(|e| format!("failed to open key {}: {e}", key_path.display()))?;
    let mut key_reader = BufReader::new(key_file);
    let keys: Vec<_> = pkcs8_private_keys(&mut key_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to parse private keys: {e}"))?;

    let key = keys
        .into_iter()
        .next()
        .ok_or_else(|| "no private key found in PEM file".to_string())?;

    // Build the ServerConfig.
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, rustls::pki_types::PrivateKeyDer::Pkcs8(key))
        .map_err(|e| format!("failed to build TLS server config: {e}"))?;

    Ok(Arc::new(config))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the forge home directory (~/.forge).
fn dirs_for_forge_home() -> Result<PathBuf, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "could not determine home directory".to_string())?;
    Ok(PathBuf::from(home).join(".forge"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_and_load_certs() {
        let tmp = TempDir::new().expect("failed to create temp dir");
        let base = tmp.path().to_path_buf();

        // Generate certs.
        let (cert_path, key_path) =
            ensure_certs_in(base.clone()).expect("ensure_certs_in failed");

        // Verify files exist.
        assert!(cert_path.exists(), "cert file should exist");
        assert!(key_path.exists(), "key file should exist");
        assert!(cert_path.ends_with("localhost.crt"));
        assert!(key_path.ends_with("localhost.key"));

        // Verify rustls can load them.
        let config = build_rustls_config(cert_path.clone(), key_path.clone());
        assert!(config.is_ok(), "rustls config should build successfully: {:?}", config.err());

        // Calling ensure_certs_in again should return the same paths (idempotent).
        let (cert2, key2) =
            ensure_certs_in(base).expect("second ensure_certs_in failed");
        assert_eq!(cert_path, cert2);
        assert_eq!(key_path, key2);
    }

    #[cfg(unix)]
    #[test]
    fn test_key_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().expect("failed to create temp dir");
        let base = tmp.path().to_path_buf();

        let (_cert_path, key_path) =
            ensure_certs_in(base).expect("ensure_certs_in failed");

        let perms = fs::metadata(&key_path)
            .expect("failed to read key metadata")
            .permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "key file should have 0600 permissions"
        );
    }
}
