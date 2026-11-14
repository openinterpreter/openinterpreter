//! Coalesces overlapping platform-root reads without retaining completed trust snapshots.
//! Later callers always reload. File/directory overrides bypass coalescing so their contents
//! are reread independently; custom Codex CA bundles are handled by the caller.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::Weak;

use rustls::RootCertStore;

#[derive(Default)]
struct NativeRootsLoader {
    in_flight: Mutex<Weak<OnceLock<Arc<RootCertStore>>>>,
}

impl NativeRootsLoader {
    fn load(&self, load: impl FnOnce() -> RootCertStore) -> Arc<RootCertStore> {
        let pending = {
            let mut in_flight = self
                .in_flight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match in_flight
                .upgrade()
                .filter(|pending| pending.get().is_none())
            {
                Some(pending) => pending,
                None => {
                    let pending = Arc::new(OnceLock::new());
                    *in_flight = Arc::downgrade(&pending);
                    pending
                }
            }
        };
        Arc::clone(pending.get_or_init(|| Arc::new(load())))
    }
}

pub(crate) fn load() -> Arc<RootCertStore> {
    // rustls-native-certs treats these as overrides of the platform store.
    // Preserve fresh file reads even when several callers construct clients together.
    if std::env::var_os("SSL_CERT_FILE").is_some() || std::env::var_os("SSL_CERT_DIR").is_some() {
        return Arc::new(load_roots());
    }
    static LOADER: NativeRootsLoader = NativeRootsLoader {
        in_flight: Mutex::new(Weak::new()),
    };
    LOADER.load(load_roots)
}

fn load_roots() -> RootCertStore {
    let rustls_native_certs::CertificateResult { certs, errors, .. } =
        rustls_native_certs::load_native_certs();
    if !errors.is_empty() {
        tracing::warn!(
            native_root_error_count = errors.len(),
            "encountered errors while loading native root certificates"
        );
    }
    let mut roots = RootCertStore::empty();
    let _ = roots.add_parsable_certificates(certs);
    roots
}

#[cfg(test)]
#[path = "native_roots_tests.rs"]
mod tests;
