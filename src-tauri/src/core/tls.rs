//! Local certificate authority + server certificate.
//!
//! The CA is name-constrained to local names (`localhost`, `*.local`) and
//! private IP ranges, so even if its key leaked it could not impersonate
//! public websites. The server certificate is re-issued when the machine's
//! LAN addresses change or it nears expiry, and hot-swapped without restart.

use anyhow::{Context, Result, bail};
use axum_server::tls_rustls::RustlsConfig;
use rcgen::string::Ia5String;
use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType, ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer,
    KeyPair, KeyUsagePurpose, NameConstraints, PublicKeyData, SanType, SerialNumber,
};
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once, RwLock};
use time::{Duration, OffsetDateTime};
use x509_parser::extensions::GeneralName;
use x509_parser::pem::parse_x509_pem;

use super::util::{random_bytes, sha256_hex};

const CA_DAYS: i64 = 3650;
/// Apple only trusts server certificates valid for at most 398 days.
const LEAF_DAYS: i64 = 397;
const RENEW_BEFORE_DAYS: i64 = 30;

pub fn install_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
    });
}

struct CaInfo {
    cert_pem: String,
    fingerprint: String,
}

pub struct Tls {
    dir: PathBuf,
    host_dns: Option<String>,
    config: RustlsConfig,
    ca: RwLock<CaInfo>,
    leaf_ips: RwLock<Vec<IpAddr>>,
}

fn random_serial() -> SerialNumber {
    let mut b = random_bytes::<16>();
    b[0] = (b[0] & 0x7f) | 0x01; // positive, no leading zero byte
    SerialNumber::from_slice(&b)
}

/// `Mac-mini.local` style DNS name for this computer, if the hostname is usable.
pub fn local_dns_name(hostname: &str) -> Option<String> {
    let base = hostname.trim().trim_end_matches(".local").to_ascii_lowercase();
    let label: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
    let label = label.trim_matches('-').to_string();
    if label.is_empty() || label.len() > 63 { None } else { Some(format!("{label}.local")) }
}

#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(contents.as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> Result<()> {
    // %APPDATA% is already private to the user on Windows.
    std::fs::write(path, contents)?;
    Ok(())
}

impl Tls {
    fn ca_cert_path(dir: &Path) -> PathBuf {
        dir.join("ca.pem")
    }
    fn ca_key_path(dir: &Path) -> PathBuf {
        dir.join("ca.key.pem")
    }
    fn leaf_cert_path(dir: &Path) -> PathBuf {
        dir.join("server.pem")
    }
    fn leaf_key_path(dir: &Path) -> PathBuf {
        dir.join("server.key.pem")
    }

    /// Loads (or creates) the CA and a server certificate covering `ips`.
    pub fn init(dir: &Path, host_label: &str, host_dns: Option<String>, ips: &[IpAddr]) -> Result<Tls> {
        install_crypto_provider();
        std::fs::create_dir_all(dir)?;
        let (ca_pem, issuer) = match Self::load_ca(dir) {
            Ok(v) => v,
            Err(e) => {
                if Self::ca_cert_path(dir).exists() {
                    tracing::warn!("existing CA unusable ({e:#}); creating a new one");
                }
                Self::create_ca(dir, host_label)?
            }
        };
        let wanted = Self::wanted_ips(ips);
        let (chain, key) = Self::ensure_leaf(dir, &ca_pem, &issuer, host_dns.as_deref(), &wanted, false)?;
        let config = RustlsConfig::from_config(Arc::new(Self::server_config(&chain, &key)?));
        Ok(Tls {
            dir: dir.to_path_buf(),
            host_dns,
            config,
            ca: RwLock::new(CaInfo { fingerprint: Self::fingerprint(&ca_pem)?, cert_pem: ca_pem }),
            leaf_ips: RwLock::new(wanted),
        })
    }

    pub fn rustls_config(&self) -> RustlsConfig {
        self.config.clone()
    }

    pub fn ca_pem(&self) -> String {
        self.ca.read().unwrap().cert_pem.clone()
    }

    pub fn ca_fingerprint(&self) -> String {
        self.ca.read().unwrap().fingerprint.clone()
    }

    /// Re-issues the server certificate if the LAN addresses changed.
    pub fn ensure_ips(&self, ips: &[IpAddr]) -> Result<bool> {
        let wanted = Self::wanted_ips(ips);
        if wanted.iter().all(|ip| self.leaf_ips.read().unwrap().contains(ip)) {
            return Ok(false);
        }
        let ca_pem = self.ca_pem();
        let (_, issuer) = Self::load_ca(&self.dir)?;
        let (chain, key) = Self::ensure_leaf(&self.dir, &ca_pem, &issuer, self.host_dns.as_deref(), &wanted, true)?;
        self.config.reload_from_config(Arc::new(Self::server_config(&chain, &key)?));
        *self.leaf_ips.write().unwrap() = wanted;
        tracing::info!("server certificate re-issued for new LAN addresses");
        Ok(true)
    }

    /// Throws away the CA and server certificate and starts fresh.
    pub fn reset(&self, host_label: &str, ips: &[IpAddr]) -> Result<()> {
        let (ca_pem, issuer) = Self::create_ca(&self.dir, host_label)?;
        let wanted = Self::wanted_ips(ips);
        let (chain, key) = Self::ensure_leaf(&self.dir, &ca_pem, &issuer, self.host_dns.as_deref(), &wanted, true)?;
        self.config.reload_from_config(Arc::new(Self::server_config(&chain, &key)?));
        *self.ca.write().unwrap() = CaInfo { fingerprint: Self::fingerprint(&ca_pem)?, cert_pem: ca_pem };
        *self.leaf_ips.write().unwrap() = wanted;
        Ok(())
    }

    fn wanted_ips(ips: &[IpAddr]) -> Vec<IpAddr> {
        let mut v: Vec<IpAddr> = vec![IpAddr::V4(Ipv4Addr::LOCALHOST)];
        // The CA only permits IPv4 subtrees, so IPv6 SANs would be rejected.
        for ip in ips {
            if ip.is_ipv4() && !v.contains(ip) {
                v.push(*ip);
            }
        }
        v
    }

    fn fingerprint(cert_pem: &str) -> Result<String> {
        let der = CertificateDer::from_pem_slice(cert_pem.as_bytes()).context("parse CA PEM")?;
        Ok(sha256_hex(der.as_ref()))
    }

    fn create_ca(dir: &Path, host_label: &str) -> Result<(String, Issuer<'static, KeyPair>)> {
        let mut p = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        let label: String = host_label.chars().take(40).collect();
        dn.push(DnType::CommonName, format!("LanShare Local CA ({label})"));
        dn.push(DnType::OrganizationName, "LanShare");
        p.distinguished_name = dn;
        p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
        let now = OffsetDateTime::now_utc();
        p.not_before = now - Duration::days(1);
        p.not_after = now + Duration::days(CA_DAYS);
        p.name_constraints = Some(NameConstraints {
            permitted_subtrees: vec![
                GeneralSubtree::DnsName("localhost".into()),
                GeneralSubtree::DnsName("local".into()),
                GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([10, 0, 0, 0], 8)),
                GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([172, 16, 0, 0], 12)),
                GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([192, 168, 0, 0], 16)),
                GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([127, 0, 0, 0], 8)),
                GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([169, 254, 0, 0], 16)),
            ],
            excluded_subtrees: vec![],
        });
        p.serial_number = Some(random_serial());
        let key = KeyPair::generate()?;
        let cert = p.self_signed(&key)?;
        let cert_pem = cert.pem();
        write_private(&Self::ca_key_path(dir), &key.serialize_pem())?;
        std::fs::write(Self::ca_cert_path(dir), &cert_pem)?;
        // Any existing server certificate was signed by the old CA.
        let _ = std::fs::remove_file(Self::leaf_cert_path(dir));
        let issuer = Issuer::from_ca_cert_pem(&cert_pem, key)?;
        tracing::info!("created a new local CA");
        Ok((cert_pem, issuer))
    }

    fn load_ca(dir: &Path) -> Result<(String, Issuer<'static, KeyPair>)> {
        let cert_pem = std::fs::read_to_string(Self::ca_cert_path(dir))?;
        let key = KeyPair::from_pem(&std::fs::read_to_string(Self::ca_key_path(dir))?)?;
        let (_, pem) = parse_x509_pem(cert_pem.as_bytes()).map_err(|e| anyhow::anyhow!("CA PEM: {e}"))?;
        let x509 = pem.parse_x509().map_err(|e| anyhow::anyhow!("CA cert: {e}"))?;
        if x509.public_key().raw != key.subject_public_key_info().as_slice() {
            bail!("CA key does not match CA certificate");
        }
        if x509.validity().not_after.to_datetime() - OffsetDateTime::now_utc() < Duration::days(RENEW_BEFORE_DAYS) {
            bail!("CA certificate is about to expire");
        }
        let issuer = Issuer::from_ca_cert_pem(&cert_pem, key)?;
        Ok((cert_pem, issuer))
    }

    /// Returns (chain PEM = leaf + CA, key PEM), re-issuing if needed or forced.
    fn ensure_leaf(
        dir: &Path,
        ca_pem: &str,
        issuer: &Issuer<'static, KeyPair>,
        host_dns: Option<&str>,
        ips: &[IpAddr],
        force: bool,
    ) -> Result<(String, String)> {
        if !force
            && let (Ok(cert), Ok(key)) = (std::fs::read_to_string(Self::leaf_cert_path(dir)), std::fs::read_to_string(Self::leaf_key_path(dir)))
            && Self::leaf_usable(&cert, ca_pem, ips).unwrap_or(false)
        {
            return Ok((format!("{cert}{ca_pem}"), key));
        }
        let mut p = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "LanShare");
        p.distinguished_name = dn;
        let mut sans = vec![SanType::DnsName(Ia5String::try_from("localhost")?)];
        if let Some(dns) = host_dns {
            sans.push(SanType::DnsName(Ia5String::try_from(dns)?));
        }
        sans.extend(ips.iter().map(|ip| SanType::IpAddress(*ip)));
        p.subject_alt_names = sans;
        p.is_ca = IsCa::ExplicitNoCa;
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        p.use_authority_key_identifier_extension = true;
        let not_before = OffsetDateTime::now_utc() - Duration::days(1);
        p.not_before = not_before;
        p.not_after = not_before + Duration::days(LEAF_DAYS);
        p.serial_number = Some(random_serial());
        let key = KeyPair::generate()?;
        let cert = p.signed_by(&key, issuer)?;
        let cert_pem = cert.pem();
        let key_pem = key.serialize_pem();
        write_private(&Self::leaf_key_path(dir), &key_pem)?;
        std::fs::write(Self::leaf_cert_path(dir), &cert_pem)?;
        Ok((format!("{cert_pem}{ca_pem}"), key_pem))
    }

    fn leaf_usable(leaf_pem: &str, ca_pem: &str, ips: &[IpAddr]) -> Result<bool> {
        let (_, pem) = parse_x509_pem(leaf_pem.as_bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;
        let leaf = pem.parse_x509().map_err(|e| anyhow::anyhow!("{e}"))?;
        if leaf.validity().not_after.to_datetime() - OffsetDateTime::now_utc() < Duration::days(RENEW_BEFORE_DAYS) {
            return Ok(false);
        }
        let mut have: Vec<IpAddr> = Vec::new();
        if let Some(san) = leaf.subject_alternative_name().map_err(|e| anyhow::anyhow!("{e}"))? {
            for gn in &san.value.general_names {
                if let GeneralName::IPAddress(b) = gn
                    && let Ok(v4) = <[u8; 4]>::try_from(*b)
                {
                    have.push(IpAddr::from(v4));
                }
            }
        }
        if ips.iter().any(|ip| !have.contains(ip)) {
            return Ok(false);
        }
        let (_, ca) = parse_x509_pem(ca_pem.as_bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;
        let ca = ca.parse_x509().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(leaf.verify_signature(Some(ca.public_key())).is_ok())
    }

    fn server_config(chain_pem: &str, key_pem: &str) -> Result<ServerConfig> {
        let certs: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(chain_pem.as_bytes()).collect::<Result<_, _>>().context("parse certificate chain")?;
        let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).context("parse private key")?;
        let mut cfg = ServerConfig::builder().with_no_client_auth().with_single_cert(certs, key)?;
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_reuses_and_reissues() {
        let dir = tempfile::tempdir().unwrap();
        let ip1: IpAddr = "192.168.1.5".parse().unwrap();
        let ip2: IpAddr = "10.0.0.9".parse().unwrap();
        let tls = Tls::init(dir.path(), "test", local_dns_name("Test-Mac.local"), &[ip1]).unwrap();
        let fp = tls.ca_fingerprint();
        let leaf1 = std::fs::read_to_string(dir.path().join("server.pem")).unwrap();

        // Restart: same CA and same leaf.
        let again = Tls::init(dir.path(), "test", local_dns_name("Test-Mac"), &[ip1]).unwrap();
        assert_eq!(again.ca_fingerprint(), fp);
        assert_eq!(std::fs::read_to_string(dir.path().join("server.pem")).unwrap(), leaf1);

        // New address: leaf re-issued, CA kept.
        assert!(again.ensure_ips(&[ip1, ip2]).unwrap());
        assert!(!again.ensure_ips(&[ip2]).unwrap());
        assert_ne!(std::fs::read_to_string(dir.path().join("server.pem")).unwrap(), leaf1);
        assert_eq!(again.ca_fingerprint(), fp);

        // Reset: new CA.
        again.reset("test", &[ip1]).unwrap();
        assert_ne!(again.ca_fingerprint(), fp);
        let leaf = std::fs::read_to_string(dir.path().join("server.pem")).unwrap();
        assert!(Tls::leaf_usable(&leaf, &again.ca_pem(), &[ip1]).unwrap());
    }

    #[test]
    fn dns_names() {
        assert_eq!(local_dns_name("My-MacBook-Pro.local").unwrap(), "my-macbook-pro.local");
        assert!(local_dns_name("我的电脑").is_none());
        assert_eq!(local_dns_name("DESKTOP_AB12").unwrap(), "desktop-ab12.local");
    }
}
