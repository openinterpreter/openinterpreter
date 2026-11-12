//! Verifies overlapping root loads, fresh trust generations, and panic recovery.

use super::*;
use pretty_assertions::assert_eq;
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;
use std::sync::mpsc;
use std::thread;

#[test]
fn concurrent_loads_share_roots_but_completed_loads_are_never_reused() {
    let loader = NativeRootsLoader::default();
    let (entered, started) = mpsc::channel();
    let (release, released) = mpsc::channel();
    thread::scope(|scope| {
        let first_loader = &loader;
        let first = scope.spawn(move || {
            first_loader.load(|| {
                entered.send(()).unwrap();
                released.recv().unwrap();
                RootCertStore::empty()
            })
        });
        started.recv().unwrap();
        let pending = loader.in_flight.lock().unwrap().upgrade().unwrap();
        let second = scope.spawn(|| loader.load(|| panic!("overlapping load must be shared")));
        // Wait for the second caller to own this generation before allowing it to finish.
        while Arc::strong_count(&pending) < 3 {
            thread::yield_now();
        }
        drop(loader.in_flight.lock().unwrap());
        release.send(()).unwrap();
        let first = first.join().unwrap();
        let second = second.join().unwrap();
        assert!(Arc::ptr_eq(&first, &second));

        // Keep the old generation alive: completion, not refcount expiry, must force a reload.
        let updated = loader.load(|| {
            let mut roots = RootCertStore::empty();
            roots.add_parsable_certificates(
                CertificateDer::pem_slice_iter(include_bytes!("../tests/fixtures/test-ca.pem"))
                    .map(Result::unwrap),
            );
            roots
        });
        assert_eq!((first.len(), second.len(), updated.len()), (0, 0, 1));
    });
}

#[test]
fn panicking_load_does_not_poison_later_trust_reads() {
    let loader = NativeRootsLoader::default();
    assert!(std::panic::catch_unwind(|| loader.load(|| panic!("interrupted load"))).is_err());
    assert!(loader.load(RootCertStore::empty).is_empty());
}
