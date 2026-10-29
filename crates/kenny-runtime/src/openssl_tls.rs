//! TLS for `OpenSSL::SSL` on rustls (ring crypto): the engine under lib/kenny/kenny/openssl_ssl.rb,
//! which gives it openssl.so's API (SSLContext, SSLSocket, Session); CRuby's own openssl/ssl.rb
//! and openssl/buffering.rb load on top, as with CRuby.
//!
//! A connection runs over the socket's file descriptor: reads and writes don't block the
//! descriptor (MSG_DONTWAIT), and a blocking call parks the thread until the descriptor is
//! ready, like Kenny's other IO. Certificate verification is OpenSSL's (the Ruby X509::Store,
//! with the context's verify callback and hostname check): rustls asks for it during the
//! handshake and Kenny calls `SSLSocket#__kn_verify`, so a rejected certificate aborts the
//! handshake with the right alert, as OpenSSL does. TLS 1.2 and 1.3 only, with rustls' cipher
//! suites (AES-GCM, ChaCha20-Poly1305, ECDHE key exchange).
//!
//! The methods here return results the Ruby layer turns into exceptions: `true`, a String or
//! an Integer on success, `:wait_readable`/`:wait_writable` when a non-blocking call would
//! block, `nil` at the end of the stream and `[message]` for a TLS error (raised as
//! OpenSSL::SSL::SSLError with OpenSSL's wording).

use super::*;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{ClientSessionMemoryCache, ClientSessionStore, Resumption};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::server::{Acceptor, ClientHello, ResolvesServerCert, ServerSessionMemoryCache, StoresServerSessions};
use rustls::sign::CertifiedKey;
use rustls::{
    AlertDescription, CertificateError, CipherSuite, Connection, DigitallySignedStruct, DistinguishedName, Error,
    ProtocolVersion, SignatureScheme, SupportedCipherSuite,
};
use std::cell::UnsafeCell;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static CFG_CLASS: G<*mut Class> = G::new(std::ptr::null_mut());
static TLS_CLASS: G<*mut Class> = G::new(std::ptr::null_mut());
static STORE_CLASS: G<*mut Class> = G::new(std::ptr::null_mut());

macro_rules! okv {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return e,
        }
    };
}

// ---------- native objects ----------

fn wrap<T>(cls: *mut Class, t: T) -> V {
    unsafe extern "C" fn free<T>(p: *mut std::ffi::c_void) {
        unsafe { drop(Box::from_raw(p as *mut T)) };
    }
    alloc(RData {
        h: Header { klass: cls, kind: Kind::Data, flags: 0 },
        data: Box::into_raw(Box::new(t)) as *mut std::ffi::c_void,
        dmark: None,
        dfree: Some(free::<T>),
        roots: Vec::new(),
    })
}
fn unwrap<T>(cls: *mut Class, v: V) -> Result<&'static T, V> {
    if is_kind(v, Kind::Data) && hdr(v).klass == cls {
        Ok(unsafe { &*((*(v as *mut RData)).data as *const T) })
    } else {
        Err(type_error(&format!("wrong argument type {} (expected TLS state)", class_name(real_class(v)))))
    }
}

/// A Ruby value a rustls callback calls into (the SSLSocket). The socket is on the stack of
/// the call driving the handshake whenever rustls can call back, so it is alive and, the
/// collector being non-moving, in place.
#[derive(Clone, Copy)]
struct RubyRef(V);
unsafe impl Send for RubyRef {}
unsafe impl Sync for RubyRef {}
impl std::fmt::Debug for RubyRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RubyRef")
    }
}

// ---------- cipher suites ----------

/// OpenSSL's name of a suite, its protocol and its key bits (SSL_CIPHER_get_name & co.)
fn suite_info(s: CipherSuite) -> (&'static str, &'static str, i64) {
    match s {
        CipherSuite::TLS13_AES_256_GCM_SHA384 => ("TLS_AES_256_GCM_SHA384", "TLSv1.3", 256),
        CipherSuite::TLS13_AES_128_GCM_SHA256 => ("TLS_AES_128_GCM_SHA256", "TLSv1.3", 128),
        CipherSuite::TLS13_CHACHA20_POLY1305_SHA256 => ("TLS_CHACHA20_POLY1305_SHA256", "TLSv1.3", 256),
        CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384 => ("ECDHE-ECDSA-AES256-GCM-SHA384", "TLSv1.2", 256),
        CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 => ("ECDHE-ECDSA-AES128-GCM-SHA256", "TLSv1.2", 128),
        CipherSuite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256 => ("ECDHE-ECDSA-CHACHA20-POLY1305", "TLSv1.2", 256),
        CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 => ("ECDHE-RSA-AES256-GCM-SHA384", "TLSv1.2", 256),
        CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 => ("ECDHE-RSA-AES128-GCM-SHA256", "TLSv1.2", 128),
        CipherSuite::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256 => ("ECDHE-RSA-CHACHA20-POLY1305", "TLSv1.2", 256),
        _ => ("UNKNOWN", "TLSv1.2", 0),
    }
}

/// Whether one OpenSSL cipher-string word (`ECDHE`, `aRSA`, `AESGCM`, `HIGH`, ...; a `+`
/// joins words that must all hold) selects the TLS 1.2 suite named `name`.
fn cipher_word_matches(word: &str, name: &str) -> bool {
    if word == name {
        return true;
    }
    word.split('+').all(|w| match w {
        "ALL" | "DEFAULT" | "HIGH" | "COMPLEMENTOFDEFAULT" | "TLSv1.2" | "SSLv3" | "TLSv1" | "SHA2" | "AEAD" | "FIPS" => true,
        "ECDHE" | "EECDH" | "kECDHE" | "kEECDH" | "ECDH" => name.starts_with("ECDHE-"),
        "ECDSA" | "aECDSA" => name.contains("-ECDSA-"),
        "RSA" | "aRSA" => name.contains("-RSA-"),
        "AESGCM" => name.contains("-GCM-"),
        "AES" => name.contains("-AES"),
        "AES128" => name.contains("-AES128-"),
        "AES256" => name.contains("-AES256-"),
        "CHACHA20" => name.contains("CHACHA20"),
        "SHA256" => name.ends_with("-SHA256"),
        "SHA384" => name.ends_with("-SHA384"),
        _ => false,
    })
}

/// The suites an OpenSSL cipher list (`ciphers=`, TLS 1.2) and TLS 1.3 suite list
/// (`ciphersuites=`) leave enabled, in the provider's order.
fn select_suites(all: &[SupportedCipherSuite], ciphers: Option<&str>, suites13: Option<&str>) -> Vec<SupportedCipherSuite> {
    let mut on12: Vec<&'static str> = Vec::new();
    let names12: Vec<&'static str> = all
        .iter()
        .filter(|s| matches!(s, SupportedCipherSuite::Tls12(_)))
        .map(|s| suite_info(s.suite()).0)
        .collect();
    match ciphers {
        None => on12 = names12.clone(),
        Some(spec) => {
            let mut banned: Vec<&'static str> = Vec::new();
            for word in spec.split([':', ',', ' ']).filter(|w| !w.is_empty()) {
                let (op, w) = match word.as_bytes()[0] {
                    b'!' | b'-' | b'+' => (word.as_bytes()[0], &word[1..]),
                    _ => (0, word),
                };
                if w.starts_with('@') {
                    continue;
                }
                for &n in &names12 {
                    if !cipher_word_matches(w, n) {
                        continue;
                    }
                    match op {
                        b'!' => {
                            banned.push(n);
                            on12.retain(|x| *x != n);
                        }
                        b'-' => on12.retain(|x| *x != n),
                        b'+' => {}
                        _ => {
                            if !banned.contains(&n) && !on12.contains(&n) {
                                on12.push(n);
                            }
                        }
                    }
                }
            }
        }
    }
    let on13: Option<Vec<&str>> = suites13.map(|s| s.split(':').filter(|w| !w.is_empty()).collect());
    let mut out: Vec<SupportedCipherSuite> = Vec::new();
    for s in all {
        let n = suite_info(s.suite()).0;
        let on = match s {
            SupportedCipherSuite::Tls13(_) => on13.as_ref().is_none_or(|l| l.contains(&n)),
            SupportedCipherSuite::Tls12(_) => on12.contains(&n),
        };
        if on {
            out.push(*s);
        }
    }
    // (TLS 1.2 suites in the cipher string's order, as OpenSSL lists them)
    out.sort_by_key(|s| match s {
        SupportedCipherSuite::Tls13(_) => 0,
        SupportedCipherSuite::Tls12(_) => 1 + on12.iter().position(|x| *x == suite_info(s.suite()).0).unwrap_or(0),
    });
    out
}

fn opt_str(v: V) -> Result<Option<String>, V> {
    if v == QNIL { Ok(None) } else { Ok(Some(String::from_utf8_lossy(&bytes(v)?).into_owned())) }
}

fn suite_ary(suites: &[SupportedCipherSuite]) -> V {
    ary_new(
        suites
            .iter()
            .map(|s| {
                let (n, ver, bits) = suite_info(s.suite());
                ary_new(vec![str_from(n), str_from(ver), mkfix(bits), mkfix(bits)])
            })
            .collect(),
    )
}

// ---------- configuration (one per SSLContext#setup) ----------

struct Cfg {
    provider: Arc<CryptoProvider>,
    versions: Vec<&'static rustls::SupportedProtocolVersion>,
    /// the context's certificates and keys (add_certificate, cert/key), for a server or for
    /// client authentication
    keys: Vec<Arc<CertifiedKey>>,
    alpn: Vec<Vec<u8>>,
    verify_mode: i64,
    client_sessions: Arc<dyn ClientSessionStore>,
    server_sessions: Arc<dyn StoresServerSessions>,
    ticketer: Option<Arc<dyn rustls::server::ProducesTickets>>,
}

/// The session cache a Session object carries (SSLSocket#session=: the next connection
/// resumes from it, as Net::HTTP's reconnects do).
struct Sessions(Arc<dyn ClientSessionStore>);

fn cfg_new(a: &[V]) -> V {
    // (tls12, tls13, ciphers, ciphersuites, [[cert_ders, key_der], ...], alpn, verify_mode, tickets)
    let mut provider = rustls::crypto::ring::default_provider();
    let ciphers = okv!(opt_str(a[2]));
    let suites13 = okv!(opt_str(a[3]));
    provider.cipher_suites = select_suites(&provider.cipher_suites, ciphers.as_deref(), suites13.as_deref());
    let mut versions = Vec::new();
    if a[1] != QFALSE && a[1] != QNIL {
        versions.push(&rustls::version::TLS13);
    }
    if a[0] != QFALSE && a[0] != QNIL {
        versions.push(&rustls::version::TLS12);
    }
    let mut keys = Vec::new();
    if is_ary(a[4]) {
        for &pair in rary(a[4]).v.iter() {
            if !is_ary(pair) || rary(pair).v.len() != 2 {
                continue;
            }
            let (certs_v, key_v) = (rary(pair).v[0], rary(pair).v[1]);
            let mut certs = Vec::new();
            for &c in rary(certs_v).v.iter() {
                certs.push(CertificateDer::from(okv!(bytes(c))));
            }
            let key = match PrivateKeyDer::try_from(okv!(bytes(key_v))) {
                Ok(k) => k,
                Err(e) => return raise(ssl_error(), &format!("SSL_CTX_use_PrivateKey: {e}")),
            };
            let signer = match provider.key_provider.load_private_key(key) {
                Ok(s) => s,
                Err(e) => return raise(ssl_error(), &format!("SSL_CTX_use_PrivateKey: {}", err_text(&e))),
            };
            keys.push(Arc::new(CertifiedKey::new(certs, signer)));
        }
    }
    let mut alpn = Vec::new();
    if is_ary(a[5]) {
        for &p in rary(a[5]).v.iter() {
            alpn.push(okv!(bytes(p)));
        }
    }
    let verify_mode = if is_fix(a[6]) { fix(a[6]) } else { 0 };
    let ticketer = if a[7] != QFALSE {
        match rustls::crypto::ring::Ticketer::new() {
            Ok(t) => Some(t),
            Err(_) => None,
        }
    } else {
        None
    };
    wrap(
        *CFG_CLASS.get(),
        Cfg {
            provider: Arc::new(provider),
            versions,
            keys,
            alpn,
            verify_mode,
            client_sessions: Arc::new(ClientSessionMemoryCache::new(256)),
            server_sessions: ServerSessionMemoryCache::new(256),
            ticketer,
        },
    )
}

// ---------- certificate verification (OpenSSL's, in Ruby) ----------

/// Whether Kenny's verifier rejected the peer (so the handshake error is "certificate
/// verify failed"; the Ruby layer has the X509 error).
#[derive(Default)]
struct VerifyState {
    rejected: AtomicBool,
}
impl std::fmt::Debug for VerifyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VerifyState")
    }
}

#[derive(Debug)]
struct Verifier {
    sock: RubyRef,
    algs: WebPkiSupportedAlgorithms,
    state: Arc<VerifyState>,
    /// (server side) VERIFY_PEER / VERIFY_FAIL_IF_NO_PEER_CERT
    offer: bool,
    mandatory: bool,
}

impl Verifier {
    /// `SSLSocket#__kn_verify(ders)`: nil to go on, or the X509 error code to reject with.
    fn verify(&self, ee: &CertificateDer<'_>, rest: &[CertificateDer<'_>]) -> Result<(), Error> {
        let mut ders = vec![bin(ee.as_ref().to_vec())];
        ders.extend(rest.iter().map(|c| bin(c.as_ref().to_vec())));
        let r = send1(self.sock.0, "__kn_verify", ary_new(ders));
        if r == UNDEF {
            // (an exception from the verify callback: it stays pending and the handshake fails)
            return Err(Error::General("verify callback raised".into()));
        }
        if r == QNIL || r == QTRUE {
            return Ok(());
        }
        self.state.rejected.store(true, Ordering::SeqCst);
        let code = if is_fix(r) { fix(r) } else { 1 };
        // (the alert OpenSSL sends for that error: ssl_x509err2alert)
        Err(Error::InvalidCertificate(match code {
            2 | 18 | 19 | 20 | 21 => CertificateError::UnknownIssuer,
            10 => CertificateError::Expired,
            9 => CertificateError::NotValidYet,
            23 => CertificateError::Revoked,
            7 | 8 => CertificateError::BadSignature,
            62..=64 => CertificateError::NotValidForName,
            50 => CertificateError::ApplicationVerificationFailure,
            _ => CertificateError::Other(rustls::OtherError(Arc::new(std::io::Error::other("certificate verify failed")))),
        }))
    }
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        ee: &CertificateDer<'_>,
        rest: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        self.verify(ee, rest).map(|_| ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.algs)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

impl ClientCertVerifier for Verifier {
    fn offer_client_auth(&self) -> bool {
        self.offer
    }
    fn client_auth_mandatory(&self) -> bool {
        self.mandatory
    }
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(&self, ee: &CertificateDer<'_>, rest: &[CertificateDer<'_>], _now: UnixTime) -> Result<ClientCertVerified, Error> {
        self.verify(ee, rest).map(|_| ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.algs)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.algs)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

/// A server's certificate: the first of the context's keys the client can verify.
#[derive(Debug)]
struct Keys(Vec<Arc<CertifiedKey>>);
impl ResolvesServerCert for Keys {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let schemes = hello.signature_schemes();
        self.0.iter().find(|k| k.key.choose_scheme(schemes).is_some()).or(self.0.first()).cloned()
    }
}
impl rustls::client::ResolvesClientCert for Keys {
    fn resolve(&self, _hints: &[&[u8]], schemes: &[SignatureScheme]) -> Option<Arc<CertifiedKey>> {
        self.0.iter().find(|k| k.key.choose_scheme(schemes).is_some()).cloned()
    }
    fn has_certs(&self) -> bool {
        !self.0.is_empty()
    }
}

// ---------- connections ----------

enum Conn {
    /// a server waiting for the ClientHello (the context's servername/ALPN callbacks run
    /// on it before the configuration is chosen)
    Accepting(Box<Acceptor>, &'static Cfg),
    Ready(Connection),
    Failed,
}

struct St {
    conn: Conn,
    sock: RubyRef,
    verify: Arc<VerifyState>,
    sessions: Option<Arc<dyn ClientSessionStore>>,
    /// the TLS stream ended (close_notify, or the peer closed the connection)
    eof: bool,
}

/// One connection's state. Kenny's threads run in parallel, so it is behind a lock; a call
/// never holds the lock while it parks, and one that finds the lock taken by its own thread
/// (a verify callback asking about the socket) gets nothing instead of waiting forever.
struct Tls {
    fd: i32,
    busy: AtomicBool,
    owner: AtomicU64,
    st: UnsafeCell<St>,
}
unsafe impl Sync for Tls {}

struct Guard<'a>(&'a Tls);
impl std::ops::Deref for Guard<'_> {
    type Target = St;
    fn deref(&self) -> &St {
        unsafe { &*self.0.st.get() }
    }
}
impl std::ops::DerefMut for Guard<'_> {
    fn deref_mut(&mut self) -> &mut St {
        unsafe { &mut *self.0.st.get() }
    }
}
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.owner.store(0, Ordering::Release);
        self.0.busy.store(false, Ordering::Release);
    }
}
impl Tls {
    fn lock(&self) -> Option<Guard<'_>> {
        let me = crate::thread::current_token();
        loop {
            if self.busy.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                self.owner.store(me, Ordering::Release);
                return Some(Guard(self));
            }
            if self.owner.load(Ordering::Acquire) == me {
                return None;
            }
            crate::thread::pass();
        }
    }
}

fn tls_of(v: V) -> Result<&'static Tls, V> {
    unwrap::<Tls>(*TLS_CLASS.get(), v)
}

fn ssl_error() -> *mut Class {
    let ossl = const_lookup(rt().c.object, sym("OpenSSL")).unwrap() as *mut Class;
    let ssl = const_get_under(ossl, sym("SSL")).unwrap() as *mut Class;
    const_get_under(ssl, sym("SSLError")).map(|c| c as *mut Class).unwrap_or(*OSSL_ERROR.get())
}

/// The socket as an io::Read / io::Write that never blocks.
struct Fd(i32);
#[cfg(target_os = "linux")]
const SEND_FLAGS: libc::c_int = libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL;
#[cfg(not(target_os = "linux"))]
const SEND_FLAGS: libc::c_int = libc::MSG_DONTWAIT;
impl Read for Fd {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let n = unsafe { libc::recv(self.0, b.as_mut_ptr() as *mut libc::c_void, b.len(), libc::MSG_DONTWAIT) };
            if n >= 0 {
                return Ok(n as usize);
            }
            let e = std::io::Error::last_os_error();
            match e.raw_os_error() {
                Some(libc::EINTR) => continue,
                // (not a socket: a pipe or a file)
                Some(libc::ENOTSOCK) => return fd_nonblocking(self.0, || unsafe { libc::read(self.0, b.as_mut_ptr() as *mut libc::c_void, b.len()) }),
                _ => return Err(e),
            }
        }
    }
}
impl Write for Fd {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        loop {
            let n = unsafe { libc::send(self.0, b.as_ptr() as *const libc::c_void, b.len(), SEND_FLAGS) };
            if n >= 0 {
                return Ok(n as usize);
            }
            let e = std::io::Error::last_os_error();
            match e.raw_os_error() {
                Some(libc::EINTR) => continue,
                Some(libc::ENOTSOCK) => return fd_nonblocking(self.0, || unsafe { libc::write(self.0, b.as_ptr() as *const libc::c_void, b.len()) }),
                _ => return Err(e),
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn fd_nonblocking(fd: i32, f: impl FnOnce() -> isize) -> std::io::Result<usize> {
    let fl = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if fl >= 0 && fl & libc::O_NONBLOCK == 0 {
        unsafe { libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK) };
    }
    let n = f();
    let r = if n >= 0 { Ok(n as usize) } else { Err(std::io::Error::last_os_error()) };
    if fl >= 0 && fl & libc::O_NONBLOCK == 0 {
        unsafe { libc::fcntl(fd, libc::F_SETFL, fl) };
    }
    r
}

/// OpenSSL's text for a TLS alert (the reason strings of SSL_R_SSLV3_ALERT_* / SSL_R_TLSV1_ALERT_*)
fn alert_text(a: AlertDescription) -> String {
    use AlertDescription as A;
    match a {
        A::CloseNotify => "sslv3 alert close notify".into(),
        A::UnexpectedMessage => "sslv3 alert unexpected message".into(),
        A::BadRecordMac => "sslv3 alert bad record mac".into(),
        A::HandshakeFailure => "sslv3 alert handshake failure".into(),
        A::BadCertificate => "sslv3 alert bad certificate".into(),
        A::UnsupportedCertificate => "sslv3 alert unsupported certificate".into(),
        A::CertificateRevoked => "sslv3 alert certificate revoked".into(),
        A::CertificateExpired => "sslv3 alert certificate expired".into(),
        A::CertificateUnknown => "sslv3 alert certificate unknown".into(),
        A::IllegalParameter => "sslv3 alert illegal parameter".into(),
        A::UnknownCA => "tlsv1 alert unknown ca".into(),
        A::AccessDenied => "tlsv1 alert access denied".into(),
        A::DecodeError => "tlsv1 alert decode error".into(),
        A::DecryptError => "tlsv1 alert decrypt error".into(),
        A::ProtocolVersion => "tlsv1 alert protocol version".into(),
        A::InsufficientSecurity => "tlsv1 alert insufficient security".into(),
        A::InternalError => "tlsv1 alert internal error".into(),
        A::InappropriateFallback => "tlsv1 alert inappropriate fallback".into(),
        A::UserCanceled => "tlsv1 alert user cancelled".into(),
        A::NoRenegotiation => "tlsv1 alert no renegotiation".into(),
        A::MissingExtension => "tlsv13 alert missing extension".into(),
        A::UnsupportedExtension => "tlsv1 unsupported extension".into(),
        A::UnrecognisedName => "tlsv1 unrecognized name".into(),
        A::BadCertificateStatusResponse => "tlsv1 bad certificate status response".into(),
        A::UnknownPSKIdentity => "tlsv1 alert unknown psk identity".into(),
        A::CertificateRequired => "tlsv13 alert certificate required".into(),
        A::NoApplicationProtocol => "tlsv1 alert no application protocol".into(),
        other => format!("ssl/tls alert {other:?}"),
    }
}

/// OpenSSL's reason string for a rustls error, where there is a close one.
fn err_text(e: &Error) -> String {
    match e {
        Error::AlertReceived(a) => alert_text(*a),
        Error::InvalidCertificate(_) => "certificate verify failed".into(),
        Error::NoCertificatesPresented => "peer did not return a certificate".into(),
        Error::NoApplicationProtocol => "no application protocol".into(),
        Error::DecryptError => "decryption failed or bad record mac".into(),
        Error::PeerSentOversizedRecord => "record overflow".into(),
        Error::InvalidMessage(m) => {
            let d = format!("{m:?}");
            if d.contains("ContentType") || d.contains("ProtocolVersion") { "wrong version number".into() } else { format!("{e}") }
        }
        Error::PeerIncompatible(p) => {
            let d = format!("{p:?}");
            if d.contains("NoCipherSuitesInCommon") {
                "no shared cipher".into()
            } else if d.contains("Version") || d.contains("Tls12NotOffered") {
                "unsupported protocol".into()
            } else if d.contains("Group") || d.contains("Kx") {
                "no suitable key share".into()
            } else if d.contains("Signature") {
                "no suitable signature algorithm".into()
            } else {
                format!("{e}")
            }
        }
        Error::General(s) => s.clone(),
        _ => format!("{e}"),
    }
}

enum Step<T> {
    Done(T),
    /// park until the descriptor is readable (false) / writable (true)
    Wait(bool),
    /// a TLS error, as OpenSSL words it
    Fail(String),
    /// a system call error (errno), or an exception left pending (UNDEF)
    Sys(i32),
    Raised,
}

fn io_step<T>(e: std::io::Error) -> Step<T> {
    match e.kind() {
        std::io::ErrorKind::WouldBlock => Step::Wait(false),
        _ => Step::Sys(e.raw_os_error().unwrap_or(libc::EIO)),
    }
}

/// Send what rustls has queued. Done(true) when all of it went out.
fn flush(conn: &mut Connection, fd: i32) -> Result<bool, i32> {
    while conn.wants_write() {
        match conn.write_tls(&mut Fd(fd)) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) => return Err(e.raw_os_error().unwrap_or(libc::EIO)),
        }
    }
    Ok(true)
}

/// Feed what the descriptor has to rustls. Ok(false): nothing there yet.
fn fill(st: &mut St, fd: i32) -> Result<bool, Step<()>> {
    let Conn::Ready(conn) = &mut st.conn else { return Err(Step::Fail("SSL session is not started yet".into())) };
    match conn.read_tls(&mut Fd(fd)) {
        Ok(0) => st.eof = true,
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
        Err(e) => return Err(Step::Sys(e.raw_os_error().unwrap_or(libc::EIO))),
    }
    if let Err(e) = conn.process_new_packets() {
        let _ = flush(conn, fd);
        st.conn = Conn::Failed;
        if pst().pending_is_set() {
            return Err(Step::Raised);
        }
        return Err(Step::Fail(err_text(&e)));
    }
    Ok(true)
}

/// Runs `step` (with the lock) until it is done, parking between tries; a non-blocking
/// call returns the wait instead.
fn drive<T>(s: V, nonblock: bool, mut step: impl FnMut(&mut St, i32) -> Step<T>, done: impl FnOnce(T) -> V) -> V {
    let tls = okv!(tls_of(s));
    loop {
        let r = {
            let Some(mut g) = tls.lock() else {
                return ary_new(vec![str_from("SSL connection in use by a callback")]);
            };
            step(&mut g, tls.fd)
        };
        match r {
            Step::Done(t) => return done(t),
            Step::Wait(w) => {
                if nonblock {
                    return mksym(sym(if w { "wait_writable" } else { "wait_readable" }));
                }
                if let Err(e) = crate::io::wait_fd(tls.fd, if w { libc::POLLOUT } else { libc::POLLIN }) {
                    return e;
                }
            }
            Step::Fail(m) => return ary_new(vec![str_from(&m)]),
            Step::Sys(e) => return crate::io::raise_errno(e, None, b""),
            Step::Raised => return UNDEF,
        }
    }
}

/// The ClientHello is in: the Ruby layer picks the context (servername_cb) and the ALPN
/// protocol (alpn_select_cb), and the connection starts with that configuration.
fn accept_hello(st: &mut St, fd: i32) -> Step<()> {
    let Conn::Accepting(acc, cfg) = &mut st.conn else { return Step::Done(()) };
    let cfg: &'static Cfg = cfg;
    match acc.read_tls(&mut Fd(fd)) {
        Ok(0) => {
            st.conn = Conn::Failed;
            return Step::Fail("unexpected eof while reading".into());
        }
        Ok(_) => {}
        Err(e) => return io_step(e),
    }
    let accepted = match acc.accept() {
        Ok(None) => return Step::Wait(false),
        Ok(Some(a)) => a,
        Err((e, mut alert)) => {
            let _ = alert.write_all(&mut Fd(fd));
            st.conn = Conn::Failed;
            return Step::Fail(err_text(&e));
        }
    };
    let (sni, offered) = {
        let ch = accepted.client_hello();
        let sni = ch.server_name().map(str_from).unwrap_or(QNIL);
        let offered: Vec<V> = ch.alpn().map(|it| it.map(|p| bin(p.to_vec())).collect()).unwrap_or_default();
        (sni, offered)
    };
    let r = send(st.sock.0, sym("__kn_hello"), &[sni, ary_new(offered)], QNIL);
    if r == UNDEF {
        st.conn = Conn::Failed;
        return Step::Raised;
    }
    // [cfg, selected protocol or nil]
    let (cfg_v, sel) = if is_ary(r) && rary(r).v.len() == 2 { (rary(r).v[0], rary(r).v[1]) } else { (QNIL, QNIL) };
    let cfg: &'static Cfg = if cfg_v != QNIL { unwrap::<Cfg>(*CFG_CLASS.get(), cfg_v).unwrap_or(cfg) } else { cfg };
    if cfg.keys.is_empty() {
        st.conn = Conn::Failed;
        return Step::Fail("no shared cipher".into());
    }
    let algs = cfg.provider.signature_verification_algorithms;
    let verifier = Arc::new(Verifier {
        sock: st.sock,
        algs,
        state: st.verify.clone(),
        offer: cfg.verify_mode & 1 != 0,
        mandatory: cfg.verify_mode & 3 == 3,
    });
    let b = match rustls::ServerConfig::builder_with_provider(cfg.provider.clone()).with_protocol_versions(&cfg.versions) {
        Ok(b) => b,
        Err(e) => {
            st.conn = Conn::Failed;
            return Step::Fail(err_text(&e));
        }
    };
    let b = if verifier.offer { b.with_client_cert_verifier(verifier) } else { b.with_no_client_auth() };
    let mut sc = b.with_cert_resolver(Arc::new(Keys(cfg.keys.clone())));
    if sel != QNIL {
        sc.alpn_protocols = vec![rstr(sel).b.to_vec()];
    }
    sc.session_storage = cfg.server_sessions.clone();
    if let Some(t) = &cfg.ticketer {
        sc.ticketer = t.clone();
    }
    match accepted.into_connection(Arc::new(sc)) {
        Ok(c) => {
            st.conn = Conn::Ready(Connection::Server(c));
            Step::Done(())
        }
        Err((e, mut alert)) => {
            let _ = alert.write_all(&mut Fd(fd));
            st.conn = Conn::Failed;
            Step::Fail(err_text(&e))
        }
    }
}
fn handshake_step(st: &mut St, fd: i32) -> Step<()> {
    if matches!(st.conn, Conn::Accepting(..)) {
        match accept_hello(st, fd) {
            Step::Done(()) => {}
            other => return other,
        }
    }
    loop {
        let Conn::Ready(conn) = &mut st.conn else { return Step::Fail("SSL session failed".into()) };
        match flush(conn, fd) {
            Ok(true) => {}
            Ok(false) => return Step::Wait(true),
            Err(e) => return Step::Sys(e),
        }
        if !conn.is_handshaking() {
            return Step::Done(());
        }
        match fill(st, fd) {
            Ok(true) => {
                if st.eof && matches!(&st.conn, Conn::Ready(c) if c.is_handshaking()) {
                    st.conn = Conn::Failed;
                    return Step::Fail("unexpected eof while reading".into());
                }
            }
            Ok(false) => return Step::Wait(false),
            Err(Step::Fail(m)) => {
                if m == "certificate verify failed" || st.verify.rejected.load(Ordering::SeqCst) {
                    return Step::Fail("certificate verify failed".into());
                }
                return Step::Fail(m);
            }
            Err(Step::Sys(e)) => return Step::Sys(e),
            Err(_) => return Step::Raised,
        }
    }
}

fn read_step(st: &mut St, fd: i32, len: usize) -> Step<Option<Vec<u8>>> {
    loop {
        let Conn::Ready(conn) = &mut st.conn else { return Step::Fail("SSL session is not started yet".into()) };
        let mut buf = vec![0u8; len];
        match conn.reader().read(&mut buf) {
            Ok(0) => return Step::Done(None),
            Ok(n) => {
                buf.truncate(n);
                return Step::Done(Some(buf));
            }
            // (the peer closed the connection without a close_notify)
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Step::Done(None),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Step::Sys(e.raw_os_error().unwrap_or(libc::EIO)),
        }
        if st.eof {
            return Step::Done(None);
        }
        // (a TLS 1.3 key update needs an answer before more comes)
        match flush(conn, fd) {
            Ok(true) => {}
            Ok(false) => return Step::Wait(true),
            Err(e) => return Step::Sys(e),
        }
        match fill(st, fd) {
            Ok(true) => {}
            Ok(false) => return Step::Wait(false),
            Err(Step::Fail(m)) => return Step::Fail(m),
            Err(Step::Sys(e)) => return Step::Sys(e),
            Err(_) => return Step::Raised,
        }
    }
}

/// Write `data` from `*done` on; Done when it all went out. A non-blocking write takes
/// what fits once what was queued before is out (`partial`).
fn write_step(st: &mut St, fd: i32, data: &[u8], done: &mut usize, partial: bool) -> Step<usize> {
    let Conn::Ready(conn) = &mut st.conn else { return Step::Fail("SSL session is not started yet".into()) };
    loop {
        match flush(conn, fd) {
            Ok(true) => {}
            Ok(false) => {
                if partial && *done > 0 {
                    return Step::Done(*done);
                }
                return Step::Wait(true);
            }
            Err(e) => return Step::Sys(e),
        }
        if *done >= data.len() {
            return Step::Done(*done);
        }
        match conn.writer().write(&data[*done..]) {
            Ok(n) => *done += n,
            Err(e) => return Step::Sys(e.raw_os_error().unwrap_or(libc::EIO)),
        }
    }
}

fn new_tls(fd: i32, sock: V, conn: Conn, sessions: Option<Arc<dyn ClientSessionStore>>, verify: Arc<VerifyState>) -> V {
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    unsafe {
        let one: libc::c_int = 1;
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_NOSIGPIPE, &one as *const _ as *const libc::c_void, 4);
    }
    wrap(
        *TLS_CLASS.get(),
        Tls {
            fd,
            busy: AtomicBool::new(false),
            owner: AtomicU64::new(0),
            st: UnsafeCell::new(St { conn, sock: RubyRef(sock), verify, sessions, eof: false }),
        },
    )
}

fn version_name(v: Option<ProtocolVersion>) -> V {
    match v {
        Some(ProtocolVersion::TLSv1_3) => str_from("TLSv1.3"),
        Some(ProtocolVersion::TLSv1_2) => str_from("TLSv1.2"),
        Some(_) => str_from("unknown"),
        None => QNIL,
    }
}

pub(super) fn init_tls(ssl: *mut Class) {
    *CFG_CLASS.get() = new_class("OpenSSL::SSL::__Cfg", rt().c.object);
    *TLS_CLASS.get() = new_class("OpenSSL::SSL::__Tls", rt().c.object);
    *STORE_CLASS.get() = new_class("OpenSSL::SSL::__Sessions", rt().c.object);

    // SSLContext#ciphers: [name, version, bits, alg_bits] of the suites a cipher list enables
    defs(ssl, "__kn_ciphers", 2, 2, |_, a, _| {
        let p = rustls::crypto::ring::default_provider();
        let c = okv!(opt_str(a[0]));
        let s = okv!(opt_str(a[1]));
        suite_ary(&select_suites(&p.cipher_suites, c.as_deref(), s.as_deref()))
    });
    defs(ssl, "__kn_config", 8, 8, |_, a, _| cfg_new(a));

    // (cfg, fd, sock, hostname or nil, peer IP or nil, sessions or nil)
    defs(ssl, "__kn_client", 6, 6, |_, a, _| {
        let cfg: &'static Cfg = okv!(unwrap::<Cfg>(*CFG_CLASS.get(), a[0]));
        let fd = okv!(int(a[1])) as i32;
        let sock = a[2];
        let host = okv!(opt_str(a[3]));
        let ip = okv!(opt_str(a[4]));
        let sessions = if a[5] != QNIL { Some(okv!(unwrap::<Sessions>(*STORE_CLASS.get(), a[5])).0.clone()) } else { None };
        let verify = Arc::new(VerifyState::default());
        let verifier = Arc::new(Verifier {
            sock: RubyRef(sock),
            algs: cfg.provider.signature_verification_algorithms,
            state: verify.clone(),
            offer: false,
            mandatory: false,
        });
        let b = match rustls::ClientConfig::builder_with_provider(cfg.provider.clone()).with_protocol_versions(&cfg.versions) {
            Ok(b) => b,
            Err(e) => return raise(ssl_error(), &err_text(&e)),
        };
        let b = b.dangerous().with_custom_certificate_verifier(verifier);
        let mut cc = if cfg.keys.is_empty() { b.with_no_client_auth() } else { b.with_client_cert_resolver(Arc::new(Keys(cfg.keys.clone()))) };
        cc.alpn_protocols = cfg.alpn.clone();
        let store = sessions.unwrap_or_else(|| cfg.client_sessions.clone());
        cc.resumption = Resumption::store(store.clone());
        // SNI: the hostname (none for an IP address, or without one, as OpenSSL)
        let name: ServerName<'static> = match host.as_deref().map(|h| h.trim_end_matches('.').to_string()) {
            Some(h) if h.parse::<std::net::IpAddr>().is_err() && ServerName::try_from(h.clone()).is_ok() => ServerName::try_from(h).unwrap(),
            Some(h) if h.parse::<std::net::IpAddr>().is_ok() => ServerName::IpAddress(h.parse::<std::net::IpAddr>().unwrap().into()),
            _ => {
                let addr = ip.and_then(|i| i.parse::<std::net::IpAddr>().ok()).unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
                ServerName::IpAddress(addr.into())
            }
        };
        let conn = match rustls::ClientConnection::new(Arc::new(cc), name) {
            Ok(c) => c,
            Err(e) => return raise(ssl_error(), &err_text(&e)),
        };
        new_tls(fd, sock, Conn::Ready(Connection::Client(conn)), Some(store), verify)
    });
    // (cfg, fd, sock)
    defs(ssl, "__kn_server", 3, 3, |_, a, _| {
        let cfg: &'static Cfg = okv!(unwrap::<Cfg>(*CFG_CLASS.get(), a[0]));
        let fd = okv!(int(a[1])) as i32;
        new_tls(fd, a[2], Conn::Accepting(Box::default(), cfg), None, Arc::new(VerifyState::default()))
    });

    let t = *TLS_CLASS.get();
    // handshake(nonblock) -> true | :wait_readable | :wait_writable | [message]
    defn(t, "handshake", 1, 1, |s, a, _| drive(s, truthy(a[0]), handshake_step, |_| QTRUE));
    // read(maxlen, nonblock) -> String | nil (end of stream) | :wait_* | [message]
    defn(t, "read", 2, 2, |s, a, _| {
        let n = okv!(int(a[0])).max(1) as usize;
        drive(s, truthy(a[1]), |st, fd| read_step(st, fd, n.min(1 << 20)), |r| match r {
            Some(b) => bin(b),
            None => QNIL,
        })
    });
    // write(string, nonblock) -> bytes written | :wait_* | [message]
    defn(t, "write", 2, 2, |s, a, _| {
        let data = okv!(bytes(a[0]));
        let nb = truthy(a[1]);
        let mut done = 0usize;
        drive(s, nb, |st, fd| write_step(st, fd, &data, &mut done, nb), |n| mkfix(n as i64))
    });
    // close_notify: tell the peer the stream ends (best effort, never blocks)
    defn(t, "close_notify", 0, 0, |s, _, _| {
        let tls = okv!(tls_of(s));
        if let Some(mut g) = tls.lock()
            && let Conn::Ready(c) = &mut g.conn
        {
            c.send_close_notify();
            let _ = flush(c, tls.fd);
        }
        QNIL
    });
    // [version, cipher [name, version, bits, bits], alpn, resumed?, sni, peer certificate DERs, handshaking?]
    defn(t, "info", 0, 0, |s, _, _| {
        let tls = okv!(tls_of(s));
        let Some(g) = tls.lock() else { return QNIL };
        let Conn::Ready(c) = &g.conn else { return QNIL };
        let cipher = c
            .negotiated_cipher_suite()
            .map(|cs| {
                let (n, ver, bits) = suite_info(cs.suite());
                ary_new(vec![str_from(n), str_from(ver), mkfix(bits), mkfix(bits)])
            })
            .unwrap_or(QNIL);
        let alpn = c.alpn_protocol().map(|p| str_new(p.to_vec())).unwrap_or(QNIL);
        let resumed = bool_v(matches!(c.handshake_kind(), Some(rustls::HandshakeKind::Resumed)));
        let sni = match c {
            Connection::Server(sc) => sc.server_name().map(str_from).unwrap_or(QNIL),
            Connection::Client(_) => QNIL,
        };
        let peer = c.peer_certificates().map(|cs| ary_new(cs.iter().map(|d| bin(d.as_ref().to_vec())).collect())).unwrap_or(QNIL);
        ary_new(vec![version_name(c.protocol_version()), cipher, alpn, resumed, sni, peer, bool_v(c.is_handshaking())])
    });
    // plaintext received and not read yet
    defn(t, "pending", 0, 0, |s, _, _| {
        let tls = okv!(tls_of(s));
        let Some(mut g) = tls.lock() else { return mkfix(0) };
        let Conn::Ready(c) = &mut g.conn else { return mkfix(0) };
        match c.process_new_packets() {
            Ok(io) => mkfix(io.plaintext_bytes_to_read() as i64),
            Err(_) => mkfix(0),
        }
    });
    // the session cache this connection resumes from and saves to (SSLSocket#session)
    defn(t, "sessions", 0, 0, |s, _, _| {
        let tls = okv!(tls_of(s));
        let Some(g) = tls.lock() else { return QNIL };
        match &g.sessions {
            Some(st) => wrap(*STORE_CLASS.get(), Sessions(st.clone())),
            None => QNIL,
        }
    });
    defn(t, "export_keying_material", 3, 3, |s, a, _| {
        let tls = okv!(tls_of(s));
        let label = okv!(bytes(a[0]));
        let n = okv!(int(a[1])).max(0) as usize;
        let ctx = if a[2] == QNIL { None } else { Some(okv!(bytes(a[2]))) };
        let Some(g) = tls.lock() else { return QNIL };
        let Conn::Ready(c) = &g.conn else { return raise(ssl_error(), "SSL session is not started yet") };
        match c.export_keying_material(vec![0u8; n], &label, ctx.as_deref()) {
            Ok(out) => bin(out),
            Err(e) => raise(ssl_error(), &err_text(&e)),
        }
    });
}

fn truthy(v: V) -> bool {
    v != QNIL && v != QFALSE
}
