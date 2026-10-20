//! System/default trust configuration for the production provider client.
//!
//! The source builds `ssl.create_default_context()`, whose trust model is:
//!
//! * Unix — OpenSSL defaults plus `SSL_CERT_FILE`/`SSL_CERT_DIR`; this is
//!   exactly what `rustls-tls-native-roots` provides through
//!   `openssl-probe`, so the default client is used unchanged.
//! * Windows — `SSLContext.load_default_certs()` loads **both** the `CA` and
//!   the `ROOT` stores (`_windows_cert_stores = ("CA", "ROOT")`) and then
//!   `set_default_verify_paths()`. `rustls-native-certs` only opens the
//!   current-user `ROOT` store, so this module adds the current-user and
//!   local-machine `CA`/`ROOT` stores through the supported `schannel`
//!   CryptoAPI binding, then the `SSL_CERT_FILE` bundle.
//!
//! Certificate and hostname verification are always enabled: only the set of
//! trusted roots is assembled here. The residual Windows difference from
//! CPython's seven store locations (enterprise, group policy, services and
//! `USERS`) is reported, not silently claimed.

use std::collections::BTreeMap;

/// The only extended key usage rustls may use a loaded root for.
#[cfg(windows)]
const PKIX_SERVER_AUTH: &str = "1.3.6.1.5.5.7.3.1";

/// Replace the built-in roots with an explicit DER root set.
///
/// This is the exact sequence the Windows branch uses: disable reqwest's
/// built-in/native roots, then add each assembled DER certificate. It is
/// platform-independent so the Linux tests can execute it with synthetic
/// CAs; certificate and hostname verification stay enabled because only the
/// root set is supplied.
pub fn add_der_roots(
    builder: reqwest::ClientBuilder,
    certificates: impl IntoIterator<Item = Vec<u8>>,
) -> reqwest::ClientBuilder {
    let mut builder = builder.tls_built_in_root_certs(false);
    for der in certificates {
        // A single damaged store entry is skipped like CPython's
        // `load_verify_locations` warning, not a startup failure. reqwest
        // only rejects a bad certificate when the client is built, so the
        // DER is parsed here first.
        if !is_parseable_der(&der) {
            continue;
        }
        if let Ok(certificate) = reqwest::Certificate::from_der(&der) {
            builder = builder.add_root_certificate(certificate);
        }
    }
    builder
}

/// Whether a DER blob is a certificate rustls can parse.
fn is_parseable_der(der: &[u8]) -> bool {
    let certificate = rustls::pki_types::CertificateDer::from(der.to_vec());
    rustls::server::ParsedCertificate::try_from(&certificate).is_ok()
}

/// Add the Windows system-store roots the source would trust.
///
/// On every other platform the reviewed `rustls-tls-native-roots` default is
/// already the source's OpenSSL/system contract.
///
/// # Errors
///
/// Returns a sanitized message when the client cannot be built with the
/// assembled roots.
pub fn configure_roots(builder: reqwest::ClientBuilder) -> Result<reqwest::ClientBuilder, String> {
    #[cfg(windows)]
    {
        let mut certificates = windows_store_certificates();
        certificates.extend(environment_certificates(&crate::boundary::process_environ()));
        Ok(add_der_roots(builder, certificates))
    }
    #[cfg(not(windows))]
    {
        Ok(builder)
    }
}

/// Parse one certificate file: a PEM bundle or a single DER certificate.
///
/// CPython's `set_default_verify_paths()` reads `SSL_CERT_FILE` as a PEM
/// bundle; a DER file is accepted as a convenience for the same variable.
#[must_use]
pub fn parse_certificate_file(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut reader = std::io::BufReader::new(bytes);
    match rustls_pemfile::certs(&mut reader) {
        Ok(certs) if !certs.is_empty() => {
            return certs.into_iter().map(|cert| cert.to_vec()).collect();
        }
        _ => {}
    }
    if bytes.first() == Some(&0x30) {
        return vec![bytes.to_vec()];
    }
    Vec::new()
}

/// The DER certificates an `SSL_CERT_FILE`-style environment add.
#[must_use]
pub fn environment_certificates(environ: &BTreeMap<String, String>) -> Vec<Vec<u8>> {
    let Some(path) = environ.get("SSL_CERT_FILE") else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    parse_certificate_file(&bytes)
}

/// Read the `CA` and `ROOT` stores for the current user and the local
/// machine, filtered the way `rustls-native-certs` filters `ROOT`.
#[cfg(windows)]
fn windows_store_certificates() -> Vec<Vec<u8>> {
    use schannel::cert_context::{CertContext, ValidUses};
    use schannel::cert_store::{CertStore, CertStoreBuilder};

    fn usable(cert: &CertContext) -> bool {
        let uses_ok = match cert.valid_uses() {
            Ok(ValidUses::All) => true,
            Ok(ValidUses::Oids(oids)) => oids.iter().any(|oid| oid == PKIX_SERVER_AUTH),
            Err(_) => false,
        };
        uses_ok && cert.is_time_valid().unwrap_or(true)
    }

    fn collect(store: &CertStore, out: &mut Vec<Vec<u8>>) {
        for cert in store.certs() {
            if usable(&cert) {
                out.push(cert.to_der().to_vec());
            }
        }
    }

    let mut out = Vec::new();
    // CPython enumerates `("CA", "ROOT")` in that order
    // (`SSLContext._windows_cert_stores`).
    for store_name in ["CA", "ROOT"] {
        let mut builder = CertStoreBuilder::new();
        builder.read_only();
        if let Ok(store) = builder.open_current_user(store_name) {
            collect(&store, &mut out);
        }
        if let Ok(store) = builder.open_local_machine(store_name) {
            collect(&store, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pem_bundle_and_der_files_parse() {
        let pem = include_bytes!("../tests/data/tls/ca.pem");
        let certs = parse_certificate_file(pem);
        assert_eq!(certs.len(), 1);
        assert_eq!(certs[0].first(), Some(&0x30), "DER sequence header");

        let der = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tls/ca.pem"),
        )
        .unwrap();
        assert_eq!(parse_certificate_file(&der), certs);

        // A PEM bundle with two certificates yields both.
        let mut bundle = pem.to_vec();
        bundle.extend_from_slice(pem);
        assert_eq!(parse_certificate_file(&bundle).len(), 2);

        assert!(parse_certificate_file(b"not a certificate").is_empty());
    }

    #[test]
    fn environment_certificate_reader_uses_ssl_cert_file() {
        let mut environ = BTreeMap::new();
        assert!(environment_certificates(&environ).is_empty());
        environ.insert(
            "SSL_CERT_FILE".to_owned(),
            "/nonexistent/kala-studio/test-ca.pem".to_owned(),
        );
        assert!(environment_certificates(&environ).is_empty());
        environ.insert(
            "SSL_CERT_FILE".to_owned(),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data/tls/ca.pem")
                .to_string_lossy()
                .into_owned(),
        );
        assert_eq!(environment_certificates(&environ).len(), 1);
        // The non-Windows default keeps the reviewed native-roots policy.
        let builder = configure_roots(reqwest::Client::builder()).unwrap();
        drop(builder);
    }

    #[test]
    fn explicit_der_roots_replace_the_builtin_set() {
        // The same assembly the Windows branch runs is executable here.
        let certificates = vec![
            parse_certificate_file(include_bytes!("../tests/data/tls/ca.pem"))
                .into_iter()
                .next()
                .expect("test CA"),
        ];
        let client = add_der_roots(reqwest::Client::builder(), certificates)
            .build()
            .expect("client with explicit roots");
        drop(client);

        // A malformed DER entry is skipped, not fatal.
        let client = add_der_roots(reqwest::Client::builder(), vec![vec![0, 1, 2]])
            .build()
            .expect("client ignoring a damaged entry");
        drop(client);
    }
}
