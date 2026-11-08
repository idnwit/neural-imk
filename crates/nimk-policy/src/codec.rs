//! Binary snapshot format shared by all learners.
//!
//! Layout (all integers and floats little-endian):
//!
//! ```text
//! "NIMK"            4 bytes magic
//! kind_len: u8      length of the kind string
//! kind: [u8]        e.g. "tiny_mlp"
//! version: u32      per-kind format version
//! payload           kind-specific, read with [`Reader`]
//! ```
//!
//! A reader must consume the payload exactly; trailing bytes are corruption.

use crate::PolicyError;

/// Magic bytes at the start of every snapshot.
const MAGIC: &[u8; 4] = b"NIMK";

/// Appends little-endian values to a byte buffer.
pub(crate) struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    /// Starts a snapshot with the common header. `kind` must be < 256 bytes.
    pub(crate) fn new(kind: &str, version: u32, payload_hint: usize) -> Self {
        let mut buf = Vec::with_capacity(4 + 1 + kind.len() + 4 + payload_hint);
        buf.extend_from_slice(MAGIC);
        let len = kind.len().min(usize::from(u8::MAX)) as u8;
        buf.push(len);
        buf.extend_from_slice(&kind.as_bytes()[..usize::from(len)]);
        buf.extend_from_slice(&version.to_le_bytes());
        Self { buf }
    }

    pub(crate) fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Writes a length/dimension as `u32`, saturating (never hit in practice:
    /// all lengths are model dimensions far below `u32::MAX`).
    pub(crate) fn len(&mut self, n: usize) {
        self.u32(sat_u32(n));
    }

    pub(crate) fn f32s(&mut self, vs: &[f32]) {
        self.buf.reserve(vs.len() * 4);
        for v in vs {
            self.buf.extend_from_slice(&v.to_le_bytes());
        }
    }

    /// Appends raw bytes (an opaque payload section).
    pub(crate) fn bytes(&mut self, bs: &[u8]) {
        self.buf.extend_from_slice(bs);
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// `usize -> u32`, saturating.
pub(crate) fn sat_u32(n: usize) -> u32 {
    n.min(u32::MAX as usize) as u32
}

/// Cursor over snapshot bytes; every read fails with [`PolicyError::Corrupt`]
/// instead of panicking.
pub(crate) struct Reader<'a> {
    rest: &'a [u8],
}

fn corrupt(msg: &str) -> PolicyError {
    PolicyError::Corrupt(msg.to_owned())
}

impl<'a> Reader<'a> {
    /// Validates magic, kind and version, returning a reader at the payload.
    pub(crate) fn open(bytes: &'a [u8], kind: &str, version: u32) -> Result<Self, PolicyError> {
        let mut r = Self { rest: bytes };
        if r.take(4)? != MAGIC {
            return Err(corrupt("bad magic"));
        }
        let len = usize::from(r.u8()?);
        let got = r.take(len)?;
        if got != kind.as_bytes() {
            return Err(PolicyError::Corrupt(format!(
                "kind mismatch: expected {kind}, got {}",
                String::from_utf8_lossy(got)
            )));
        }
        let v = r.u32()?;
        if v != version {
            return Err(PolicyError::Corrupt(format!(
                "version mismatch: expected {version}, got {v}"
            )));
        }
        Ok(r)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], PolicyError> {
        if self.rest.len() < n {
            return Err(corrupt("truncated snapshot"));
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], PolicyError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    /// Reads exactly `n` raw bytes.
    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8], PolicyError> {
        self.take(n)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, PolicyError> {
        Ok(self.array::<1>()?[0])
    }

    pub(crate) fn u32(&mut self) -> Result<u32, PolicyError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// Reads exactly `n` floats; non-finite values are rejected.
    pub(crate) fn f32s(&mut self, n: usize) -> Result<Vec<f32>, PolicyError> {
        let bytes = self.take(n.checked_mul(4).ok_or_else(|| corrupt("length overflow"))?)?;
        let mut out = Vec::with_capacity(n);
        for chunk in bytes.chunks_exact(4) {
            let mut a = [0u8; 4];
            a.copy_from_slice(chunk);
            let v = f32::from_le_bytes(a);
            if !v.is_finite() {
                return Err(corrupt("non-finite parameter"));
            }
            out.push(v);
        }
        Ok(out)
    }

    /// Reads a `u32` that must equal `expected` (a model dimension).
    pub(crate) fn expect_dim(&mut self, expected: u32, what: &str) -> Result<(), PolicyError> {
        let got = self.u32()?;
        if got == expected {
            Ok(())
        } else {
            Err(PolicyError::Corrupt(format!(
                "{what} mismatch: expected {expected}, got {got}"
            )))
        }
    }

    /// Fails if any payload bytes remain.
    pub(crate) fn finish(self) -> Result<(), PolicyError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(corrupt("trailing bytes"))
        }
    }
}

/// Numerically stable logistic function.
#[inline]
pub(crate) fn sigmoid(x: f32) -> f32 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

/// Binary cross-entropy of `p` against target `y` in {0, 1}, with `p` clamped
/// away from 0 and 1.
#[inline]
pub(crate) fn log_loss(p: f32, y: f32) -> f32 {
    let p = p.clamp(1e-7, 1.0 - 1e-7);
    -(y * p.ln() + (1.0 - y) * (1.0 - p).ln())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trip_and_rejections() {
        let mut w = Writer::new("k", 3, 8);
        w.u32(7);
        w.f32s(&[1.5]);
        let bytes = w.finish();
        let mut r = Reader::open(&bytes, "k", 3).expect("open");
        assert_eq!(r.u32().expect("u32"), 7);
        assert_eq!(r.f32s(1).expect("f32"), vec![1.5]);
        assert!(r.finish().is_ok());
        assert!(Reader::open(&bytes, "x", 3).is_err());
        assert!(Reader::open(&bytes, "k", 4).is_err());
        assert!(Reader::open(&bytes[..3], "k", 3).is_err());
        let mut r = Reader::open(&bytes, "k", 3).expect("open");
        assert!(r.f32s(3).is_err());
    }

    #[test]
    fn sigmoid_is_stable() {
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-6);
        assert!(sigmoid(-1000.0).is_finite());
        assert!(sigmoid(1000.0) <= 1.0);
        assert!(log_loss(0.0, 1.0).is_finite());
    }
}
