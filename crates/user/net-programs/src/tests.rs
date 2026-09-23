// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crypto_rng::{ChaChaRng, Entropy, EntropyError, Rng};
use driver_virtio_net::frames::Frames;
use virtio_queue::memory::QueueMemory;

use crate::net_dma::{NetDma, REGION_BYTES};
use crate::reseed::ReseedCounter;

struct Unavailable;

impl Entropy for Unavailable {
    fn fill(&mut self, _out: &mut [u8]) -> Result<(), EntropyError> {
        Err(EntropyError::Unavailable)
    }
}

#[test]
fn device_written_memory_is_read_after_a_raw_write() {
    let mut storage = std::vec![0u128; REGION_BYTES.div_ceil(16)];
    let pointer = storage.as_mut_ptr().cast::<u8>();
    // SAFETY: storage stays live; the test accesses device-written pieces
    // only through raw pointers while dma exists.
    let mut dma = unsafe { NetDma::new(pointer, storage.len().saturating_mul(16), 0x1000) }
        .expect("DMA region");
    dma.clear();
    let used = usize::try_from(dma.receive.rings().used_ring - 0x1000).expect("offset");
    // SAFETY: the ring address is inside storage and no reference covers it.
    unsafe { pointer.add(used + 2).write_volatile(7) };
    let mut index = [0u8; 2];
    dma.receive.read_used(2, &mut index).expect("used index");
    assert_eq!(index, [7, 0]);
    assert_eq!(dma.receive.read_used_u16(2), Ok(7));

    let frame = usize::try_from(dma.taken.address(0).expect("frame") - 0x1000).expect("offset");
    // SAFETY: the receive area is inside storage and no reference covers it.
    unsafe { pointer.add(frame + 12).write_volatile(0xAB) };
    let mut copied = [0];
    assert_eq!(dma.taken.copy(0, 12, &mut copied), Some(()));
    assert_eq!(copied, [0xAB]);
    assert!(dma.taken.bytes_mut(0).is_none());
    assert!(dma.given.bytes_mut(0).is_some());
}

#[test]
fn repeated_connect_draws_do_not_exhaust_the_generator() {
    let mut counter = ReseedCounter::default();
    let mut rng = ChaChaRng::from_seed(&[1; 32], Unavailable);
    for _request in 0..=262_144 {
        if counter.due() {
            rng = ChaChaRng::from_seed(&[2; 32], Unavailable);
        }
        rng.fill(&mut [0; 4]).expect("TCP sequence number");
    }
}

#[test]
fn program_catalog_names_all_four_binaries() {
    let manifest = include_str!("../Cargo.toml");
    let readme = include_str!("../README.md");
    let catalog = include_str!("../../../../docs/05-code-organization.md");
    assert_eq!(manifest.matches("[[bin]]").count(), 4);
    assert!(manifest.contains("description = \"The four network programs"));
    assert!(readme.contains("The four programs of the network"));
    assert!(readme.contains("under this package"));
    assert!(catalog.contains("server-net, app-net, app-ssh and app-tls"));
    for source in [
        include_str!("bin/server_net.rs"),
        include_str!("bin/app_net.rs"),
        include_str!("bin/app_ssh.rs"),
        include_str!("bin/app_tls.rs"),
    ] {
        assert!(source.contains("The package holds four programs"));
    }
}
