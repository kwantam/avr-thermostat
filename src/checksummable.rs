//! Checksummable trait

pub trait Checksummable {
    fn cksum(&self, digest: &mut crc::Digest<'_, u32, crc::NoTable>);
}
