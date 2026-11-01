//! Strict bounded canonical prototype codec. Not a privacy-proof wire format.
use crate::{Error, Result};

#[derive(Default)]
pub struct Writer(pub Vec<u8>);
impl Writer {
    pub fn u8(&mut self, n: u8) {
        self.0.push(n);
    }
    pub fn u16(&mut self, n: u16) {
        self.raw(&n.to_be_bytes());
    }
    pub fn u32(&mut self, n: u32) {
        self.raw(&n.to_be_bytes());
    }
    pub fn u64(&mut self, n: u64) {
        self.raw(&n.to_be_bytes());
    }
    pub fn raw(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
    pub fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.u32(bytes.len().try_into().map_err(|_| Error::Limit)?);
        self.raw(bytes);
        Ok(())
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8], limit: usize) -> Result<Self> {
        if bytes.len() > limit {
            return Err(Error::Limit);
        }
        Ok(Self { bytes, cursor: 0 })
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.cursor.checked_add(n).ok_or(Error::Encoding)?;
        let slice = self.bytes.get(self.cursor..end).ok_or(Error::Encoding)?;
        self.cursor = end;
        Ok(slice)
    }
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| Error::Encoding)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    pub fn bytes(&mut self, max: usize) -> Result<&'a [u8]> {
        let len = self.u32()? as usize;
        if len > max {
            return Err(Error::Limit);
        }
        self.take(len)
    }
    pub fn end(self) -> Result<()> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err(Error::Encoding)
        }
    }
}
