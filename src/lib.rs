//! A statically sized ring buffer holding fixed-size records.
#![no_std]
#![warn(
    missing_docs,
    nonstandard_style,
    rust_2021_compatibility,
    rust_2018_idioms,
    clippy::unnested_or_patterns,
    clippy::redundant_closure_for_method_calls,
    unsafe_code
)]

use crc::{CRC_32_ISO_HDLC, Crc, NoTable};
use hex_literal::hex;

/// Magic value for detecting zeroed-out ring buffer
const RINGBUF_MAGIC: [u8; 8] = hex!("d2cfe82fa2c9ce75");

/// Random initialization vector for the AdHash construction
const RINGBUF_ADHASH_CONST: u32 = 0x741e2c97;

/// Domain separator for ring buffer checksum
#[repr(u8)]
enum RingbufStatus {
    BufferEntry = 0,
    Metadata = 1, // FIXME placeholder for actual status flags
}

/// A trait for checksumming the
pub trait Checksummable {
    /// Update `digest` with the checksum of `self`.
    fn cksum(&self, digest: &mut crc::Digest<'_, u32, crc::NoTable>);
}

/// A ring buffer and state register with checksumming behavior to help survive
/// resets and power glitches.
///
/// T: Checksummable is needed for computing the checksum. T: Copy is needed
/// because we cannot take references like &buf[offset] since this struct has
/// a packed repr (the reference might be unaligned), so instead we return
/// copies of the stored values. These values should anyway be short, so that
/// is fine.
#[repr(C, packed)]
pub struct StaticRingbuf<T: Checksummable + Copy, const L: usize> {
    /// An identifier corresponding to the next item to be added to the ring
    next_id: u32,

    /// Offset to the first element in the ring buffer.
    ///
    /// INVARIANT: self.head < L
    head: usize,

    /// Offset to the next element to be overwritten in the ring buffer.
    ///
    /// INVARIANT: self.next < L
    next: usize,

    /// Underlying data
    buf: [T; L],

    /// Checksum of the buffer used to check integrity at startup
    checksum: u32,

    /// Magic value used to identify a StaticRingbuf in memory
    magic: [u8; 8],
}

// BUG currently next=head is treated as length 0. should be length L,
// and a valid ring buffer never has 0 entries in it

#[allow(clippy::len_without_is_empty)]
impl<T: Checksummable + Copy, const L: usize> StaticRingbuf<T, L> {
    /// Get the number of valid elements in the ring buffer
    pub fn len(&self) -> usize {
        // self.head == self.next => buffer is full
        // the API maintains the invariant that the buffer is never empty
        if self.head < self.next {
            self.next - self.head
        } else {
            self.next + L - self.head
        }
    }

    /// Get the first valid element of the ring buffer along with its identifier
    pub fn first(&self) -> (T, u32) {
        (
            self.buf[self.head],
            self.next_id.wrapping_sub(self.len() as u32),
        )
    }

    /// Get the last valid element of the ring buffer along with its identifier
    pub fn last(&self) -> (T, u32) {
        // (self.next - 1) mod L.
        let offset = if self.next == 0 { L } else { self.next } - 1;
        (self.buf[offset], self.next_id.wrapping_sub(1u32))
    }

    /// Return an iterator over all items in the ring buffer
    pub fn items(&self) -> StaticRingbufItems<'_, T, L> {
        StaticRingbufItems {
            buf: self,
            next: self.next,
            remaining: self.len(),
        }
    }

    /// Compute the checksum of the StaticRingbuf, which is defined as an
    /// [AdHash] of the metadata and buffer content.
    ///
    /// Our hash function is `fmix32(CRC32-ISO/HDLC(data))`. The use of CRC32 gives
    /// us the nice error-detection properties of a CRC. `fmix32` is defined in the
    /// [murmur3] hash function. It's a weak "randomizing" bijection that breaks the
    /// GF(2) affine behavior of CRC32. This is a heuristic attempt to meet the
    /// random-oracle requirement of the AdHash analysis, and it ensures that certain
    /// error patterns (e.g., an even number of flipped MSBs) are detected.
    ///
    /// [AdHash]: https://link.springer.com/content/pdf/10.1007/3-540-69053-0_13.pdf
    /// [murmur3]: https://en.wikipedia.org/wiki/MurmurHash
    pub fn cksum(&self) -> u32 {
        let mut sum = RINGBUF_ADHASH_CONST;

        // compute digest of metadata
        let crc = Crc::<u32, NoTable>::new(&CRC_32_ISO_HDLC);
        let mut digest = crc.digest();
        self.metadata_cksum(&mut digest);
        accumulate_cksum(&mut sum, digest);

        for (entry, id) in self.items() {
            let mut digest = crc.digest();
            digest.update(&[RingbufStatus::BufferEntry as u8]);
            digest.update(id.to_be_bytes().as_ref());
            entry.cksum(&mut digest);
            accumulate_cksum(&mut sum, digest);
        }

        sum
    }

    /// Compute the checksum of the metadata
    fn metadata_cksum(&self, digest: &mut crc::Digest<'_, u32, crc::NoTable>) {
        // FIXME this should be the actual status flags, not just a placeholder
        digest.update(&[RingbufStatus::Metadata as u8]);

        digest.update(self.next_id.to_be_bytes().as_ref());
        digest.update((self.head as u16).to_be_bytes().as_ref());
        digest.update((self.next as u16).to_be_bytes().as_ref());
    }
}

/// Fmix32 is a cheap to evaluate quasi-quasi-quasi-random bijection
/// that is part of the murmur3 hash function.
fn murmur3_fmix32(mut h: u32) -> u32 {
    const C1: u32 = 0x85ebca6b;
    const C2: u32 = 0xc2b2ae35;

    h ^= h >> 16;
    h = h.wrapping_mul(C1);
    h ^= h >> 13;
    h = h.wrapping_mul(C2);
    h ^ (h >> 16)
}

/// Accumulate a CRC32 value into the running checksum by computing
/// `sum = sum + fmix32(crc_value) mod 2^32`.
fn accumulate_cksum(sum: &mut u32, digest: crc::Digest<'_, u32, crc::NoTable>) {
    *sum = sum.wrapping_add(murmur3_fmix32(digest.finalize()))
}

/// Iterator over the `(entry, id)` in the ring buffer
pub struct StaticRingbufItems<'a, T: Checksummable + Copy, const L: usize> {
    /// A reference to a StaticRingbuf whose items we are iterating
    buf: &'a StaticRingbuf<T, L>,

    /// The offset of the next item
    ///
    /// INVARIANT: self.next < L
    next: usize,

    /// The number of items remaining
    ///
    /// INVARIANT: self.remaining <= L
    remaining: usize,
}

impl<'a, T: Checksummable + Copy, const L: usize> Iterator for StaticRingbufItems<'a, T, L> {
    type Item = (T, u32);

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let entry = self.buf.buf[self.next];
        let id = self.buf.next_id.wrapping_sub(self.remaining as u32);

        // update iterator state
        self.remaining -= 1;

        // self.next = self.next + 1 mod L
        self.next += 1;
        if self.next == L {
            self.next = 0;
        }

        Some((entry, id))
    }
}

impl<'a, T: Checksummable + Copy, const L: usize> core::iter::FusedIterator
    for StaticRingbufItems<'a, T, L>
{
}
