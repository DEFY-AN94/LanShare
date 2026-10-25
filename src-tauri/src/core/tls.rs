//! Local certificate authority + server certificate.
//!
//! The CA is name-constrained to local names (`localhost`, `*.local`) and
//! private IP ranges, and (from v0.3.1) limited to server authentication, so
//! even if its key leaked it could not impersonate public websites or sign
//! anything but a server. The server certificate is re-issued when the machine's
//! LAN addresses or `.local` name change or it nears expiry, and hot-swapped
//! without restart.

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
/// Every local CA's common name starts with this.
const CA_NAME_PREFIX: &str = "LanShare Local CA";

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
    name: String,
}

impl CaInfo {
    fn new(cert_pem: String) -> Result<CaInfo> {
        Ok(CaInfo { fingerprint: Tls::fingerprint(&cert_pem)?, name: Tls::common_name(&cert_pem)?, cert_pem })
    }
}

pub struct Tls {
    dir: PathBuf,
    config: RustlsConfig,
    ca: RwLock<CaInfo>,
    leaf_dns: RwLock<Option<String>>,
    leaf_ips: RwLock<Vec<IpAddr>>,
    /// `init` had to make a new CA (none, or the one there was unusable).
    new_ca: bool,
}

fn random_serial() -> SerialNumber {
    let mut b = random_bytes::<16>();
    b[0] = (b[0] & 0x7f) | 0x01; // positive, no leading zero byte
    SerialNumber::from_slice(&b)
}

/// Common name of a new CA, e.g. "LanShare Local CA (Mac) 2DD31B". iPhone
/// settings list profiles and trusted roots by this name only, so the tag keeps
/// CAs from other installs (or from before a reset) apart from the current one.
pub fn ca_common_name(host_label: &str, tag: &str) -> String {
    let label: String = host_label.chars().take(40).collect();
    format!("{CA_NAME_PREFIX} ({label}) {tag}")
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

    /// Loads (or creates) the CA and a server certificate covering `host_dns` and `ips`.
    /// An existing CA is kept as it is, including a name from an older version.
    pub fn init(dir: &Path, host_label: &str, host_dns: Option<String>, ips: &[IpAddr]) -> Result<Tls> {
        install_crypto_provider();
        std::fs::create_dir_all(dir)?;
        let (ca_pem, issuer, new_ca) = match Self::load_ca(dir) {
            Ok((pem, issuer)) => (pem, issuer, false),
            Err(e) => {
                if Self::ca_cert_path(dir).exists() {
                    tracing::warn!("existing CA unusable ({e:#}); creating a new one");
                }
                let (pem, issuer) = Self::create_ca(dir, host_label)?;
                (pem, issuer, true)
            }
        };
        let wanted = Self::wanted_ips(ips);
        let (chain, key) = Self::ensure_leaf(dir, &ca_pem, &issuer, host_dns.as_deref(), &wanted, false)?;
        let config = RustlsConfig::from_config(Arc::new(Self::server_config(&chain, &key)?));
        Ok(Tls {
            dir: dir.to_path_buf(),
            config,
            ca: RwLock::new(CaInfo::new(ca_pem)?),
            leaf_dns: RwLock::new(host_dns),
            leaf_ips: RwLock::new(wanted),
            new_ca,
        })
    }

    /// Whether this start made a new CA: devices paired before it cannot be
    /// taken to have paired under it.
    pub fn made_new_ca(&self) -> bool {
        self.new_ca
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

    /// The CA's common name, as iPhone settings show it.
    pub fn ca_name(&self) -> String {
        self.ca.read().unwrap().name.clone()
    }

    /// The `.local` name in the current server certificate.
    pub fn leaf_dns(&self) -> Option<String> {
        self.leaf_dns.read().unwrap().clone()
    }

    /// Re-issues the server certificate if the LAN addresses or the `.local`
    /// name changed. A name that is not usable (`None`) keeps the current one.
    pub fn ensure_names(&self, dns: Option<String>, ips: &[IpAddr]) -> Result<bool> {
        let wanted = Self::wanted_ips(ips);
        let dns = dns.or_else(|| self.leaf_dns());
        if dns == self.leaf_dns() && wanted.iter().all(|ip| self.leaf_ips.read().unwrap().contains(ip)) {
            return Ok(false);
        }
        let ca_pem = self.ca_pem();
        let (_, issuer) = Self::load_ca(&self.dir)?;
        let (chain, key) = Self::ensure_leaf(&self.dir, &ca_pem, &issuer, dns.as_deref(), &wanted, true)?;
        self.config.reload_from_config(Arc::new(Self::server_config(&chain, &key)?));
        tracing::info!("server certificate re-issued for {} and {} addresses", dns.as_deref().unwrap_or("no .local name"), wanted.len());
        *self.leaf_dns.write().unwrap() = dns;
        *self.leaf_ips.write().unwrap() = wanted;
        Ok(true)
    }

    /// Re-issues the server certificate when it nears expiry (for apps that
    /// run for months without a restart).
    pub fn ensure_fresh(&self) -> Result<bool> {
        let pem = std::fs::read_to_string(Self::leaf_cert_path(&self.dir))?;
        let (_, parsed) = parse_x509_pem(pem.as_bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;
        let leaf = parsed.parse_x509().map_err(|e| anyhow::anyhow!("{e}"))?;
        if leaf.validity().not_after.to_datetime() - OffsetDateTime::now_utc() >= Duration::days(RENEW_BEFORE_DAYS) {
            return Ok(false);
        }
        let ips = self.leaf_ips.read().unwrap().clone();
        let (_, issuer) = Self::load_ca(&self.dir)?;
        let (chain, key) = Self::ensure_leaf(&self.dir, &self.ca_pem(), &issuer, self.leaf_dns().as_deref(), &ips, true)?;
        self.config.reload_from_config(Arc::new(Self::server_config(&chain, &key)?));
        tracing::info!("server certificate renewed before expiry");
        Ok(true)
    }

    /// Throws away the CA and server certificate and starts fresh.
    pub fn reset(&self, host_label: &str, ips: &[IpAddr]) -> Result<()> {
        let (ca_pem, issuer) = Self::create_ca(&self.dir, host_label)?;
        // On disk now, so the current one from here on, also if the server
        // certificate cannot be issued below (the next start issues it).
        *self.ca.write().unwrap() = CaInfo::new(ca_pem.clone())?;
        let wanted = Self::wanted_ips(ips);
        let (chain, key) = Self::ensure_leaf(&self.dir, &ca_pem, &issuer, self.leaf_dns().as_deref(), &wanted, true)?;
        self.config.reload_from_config(Arc::new(Self::server_config(&chain, &key)?));
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

    pub(super) fn common_name(cert_pem: &str) -> Result<String> {
        let (_, pem) = parse_x509_pem(cert_pem.as_bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;
        let x509 = pem.parse_x509().map_err(|e| anyhow::anyhow!("{e}"))?;
        let cn = x509.subject().iter_common_name().next().and_then(|cn| cn.as_str().ok()).unwrap_or_default();
        Ok(cn.to_string())
    }

    fn create_ca(dir: &Path, host_label: &str) -> Result<(String, Issuer<'static, KeyPair>)> {
        let key = KeyPair::generate()?;
        // The first bytes of the CA's Subject Key Identifier: fixed per key, so
        // every new CA (each install, each reset) gets a different name.
        let tag = hex::encode_upper(&CertificateParams::default().key_identifier(&key)[..3]);
        Self::write_ca(dir, key, &ca_common_name(host_label, &tag))
    }

    pub(super) fn write_ca(dir: &Path, key: KeyPair, common_name: &str) -> Result<(String, Issuer<'static, KeyPair>)> {
        let mut p = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, common_name);
        dn.push(DnType::OrganizationName, "LanShare");
        p.distinguished_name = dn;
        p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
        // Roots made before v0.3.1 have none and are kept as they are.
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
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
        let cert = p.self_signed(&key)?;
        let cert_pem = cert.pem();
        // Into place only once both are written: a failed write (a full disk)
        // leaves the CA that was there.
        let (key_new, cert_new) = (dir.join("ca.key.pem.new"), dir.join("ca.pem.new"));
        write_private(&key_new, &key.serialize_pem())?;
        std::fs::write(&cert_new, &cert_pem)?;
        std::fs::rename(&key_new, Self::ca_key_path(dir))?;
        std::fs::rename(&cert_new, Self::ca_cert_path(dir))?;
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
            && Self::leaf_usable(&cert, ca_pem, host_dns, ips).unwrap_or(false)
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

    pub(super) fn leaf_usable(leaf_pem: &str, ca_pem: &str, dns: Option<&str>, ips: &[IpAddr]) -> Result<bool> {
        let (_, pem) = parse_x509_pem(leaf_pem.as_bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;
        let leaf = pem.parse_x509().map_err(|e| anyhow::anyhow!("{e}"))?;
        if leaf.validity().not_after.to_datetime() - OffsetDateTime::now_utc() < Duration::days(RENEW_BEFORE_DAYS) {
            return Ok(false);
        }
        let mut have: Vec<IpAddr> = Vec::new();
        let mut have_dns: Vec<&str> = Vec::new();
        if let Some(san) = leaf.subject_alternative_name().map_err(|e| anyhow::anyhow!("{e}"))? {
            for gn in &san.value.general_names {
                match gn {
                    GeneralName::IPAddress(b) => {
                        if let Ok(v4) = <[u8; 4]>::try_from(*b) {
                            have.push(IpAddr::from(v4));
                        }
                    }
                    GeneralName::DNSName(name) => have_dns.push(name),
                    _ => {}
                }
            }
        }
        if ips.iter().any(|ip| !have.contains(ip)) || dns.is_some_and(|d| !have_dns.contains(&d)) {
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
        assert!(again.ensure_names(local_dns_name("Test-Mac"), &[ip1, ip2]).unwrap());
        assert!(!again.ensure_names(local_dns_name("Test-Mac"), &[ip2]).unwrap());
        assert_ne!(std::fs::read_to_string(dir.path().join("server.pem")).unwrap(), leaf1);
        assert_eq!(again.ca_fingerprint(), fp);

        // Still fresh: nothing to renew.
        assert!(!again.ensure_fresh().unwrap());

        // Reset: new CA.
        again.reset("test", &[ip1]).unwrap();
        assert_ne!(again.ca_fingerprint(), fp);
        let leaf = std::fs::read_to_string(dir.path().join("server.pem")).unwrap();
        assert!(Tls::leaf_usable(&leaf, &again.ca_pem(), Some("test-mac.local"), &[ip1]).unwrap());
    }

    #[test]
    fn dns_names() {
        assert_eq!(local_dns_name("My-MacBook-Pro.local").unwrap(), "my-macbook-pro.local");
        assert!(local_dns_name("我的电脑").is_none());
        assert_eq!(local_dns_name("DESKTOP_AB12").unwrap(), "desktop-ab12.local");
    }
}
