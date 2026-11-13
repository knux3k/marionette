//! Tiny little-endian byte writer and reader for packets. No allocation beyond the output
//! buffer; a reader never panics on short or hostile input, it returns `None`.

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with(kind: u8) -> Self {
        Self { buf: vec![kind] }
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    /// The exact bits of `v`, so every peer sees the same value.
    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.u32(v.to_bits())
    }
    /// Length (u32) then the bytes.
    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.u32(v.len() as u32);
        self.buf.extend_from_slice(v);
        self
    }
    pub fn raw(&mut self, v: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(v);
        self
    }
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }
    pub fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }
    pub fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|s| u16::from_le_bytes([s[0], s[1]]))
    }
    pub fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
    }
    pub fn f32(&mut self) -> Option<f32> {
        self.u32().map(f32::from_bits)
    }
    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    /// Everything not read yet.
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.data[self.pos.min(self.data.len())..];
        self.pos = self.data.len();
        s
    }
    pub fn is_done(&self) -> bool {
        self.pos >= self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_short_input() {
        let mut w = Writer::with(7);
        w.u16(513).u32(70000).u64(u64::MAX).f32(-0.0).bytes(b"hi");
        let mut r = Reader::new(&w.buf);
        assert_eq!(r.u8(), Some(7));
        assert_eq!(r.u16(), Some(513));
        assert_eq!(r.u32(), Some(70000));
        assert_eq!(r.u64(), Some(u64::MAX));
        assert_eq!(r.f32().map(f32::to_bits), Some((-0.0f32).to_bits()));
        assert_eq!(r.bytes(), Some(&b"hi"[..]));
        assert!(r.is_done());
        assert_eq!(r.u8(), None);
        // A length that runs past the end is refused, not a panic.
        let mut r = Reader::new(&[255, 255, 255, 255, 1]);
        assert_eq!(r.bytes(), None);
    }
}
