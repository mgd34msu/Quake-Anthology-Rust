//! CRT startup, sorting and math from windows_crt.c, using owned child bytes.
use super::{NativeError, Runtime};
use crate::native::{
    NativeAbi,
    NativeScalar::{self, I32, Void, Word},
    runtime::{Crt, Math},
};

impl Runtime {
    fn onexit_register(&self, table: u64, callback: u64) -> Result<u64, NativeError> {
        let m = &self.memory;
        if table == 0 || callback == 0 {
            return Err(NativeError::Extent);
        }
        let mut begin = m.unsigned(table, 8)?;
        let mut end = m.unsigned(table + 8, 8)?;
        let mut capacity = m.unsigned(table + 16, 8)?;
        if begin == 0 || end == 0 || capacity == 0 || end == capacity {
            let existing = if begin == 0 || end == 0 {
                0
            } else {
                end.checked_sub(begin).ok_or(NativeError::Extent)? as usize
            };
            if (begin != 0 && end < begin) || existing > 0x8000000 {
                return Err(NativeError::Extent);
            }
            let bytes = (existing * 2).max(256);
            let next = self.allocate(bytes)?;
            if next == 0 {
                return Ok(u32::MAX as u64);
            }
            if begin != 0 {
                m.copy(next, begin, existing)?;
                self.free(begin)?;
            }
            begin = next;
            end = next + existing as u64;
            capacity = next + bytes as u64;
            m.put(table, 8, begin)?;
            m.put(table + 16, 8, capacity)?;
        }
        m.put(end, 8, callback)?;
        m.put(table + 8, 8, end.checked_add(8).ok_or(NativeError::Extent)?)?;
        Ok(0)
    }
    fn onexit_execute(&self, table: u64, abi: NativeAbi) -> Result<(), NativeError> {
        let m = &self.memory;
        let begin = m.unsigned(table, 8)?;
        let mut end = m.unsigned(table + 8, 8)?;
        if begin != 0 && end != 0 {
            if end < begin || (end - begin) % 8 != 0 {
                return Err(NativeError::Extent);
            }
            while end > begin {
                end -= 8;
                let callback = m.unsigned(end, 8)?;
                m.put(end, 8, 0)?;
                if callback != 0 {
                    self.foreign(callback, abi, &[], Void, &[])?;
                }
            }
            self.free(begin)?;
        }
        m.fill(table, 0, 24)
    }
    fn sort_compare(
        &self,
        base: u64,
        bytes: usize,
        comparator: u64,
        abi: NativeAbi,
        left: usize,
        right: usize,
    ) -> Result<i32, NativeError> {
        Ok(self.foreign(
            comparator,
            abi,
            &[Word, Word],
            I32,
            &[base + (left * bytes) as u64, base + (right * bytes) as u64],
        )? as u32 as i32)
    }
    fn sort_sift(
        &self,
        base: u64,
        bytes: usize,
        comparator: u64,
        abi: NativeAbi,
        mut root: usize,
        end: usize,
    ) -> Result<(), NativeError> {
        while root < end / 2 {
            let mut child = root * 2 + 1;
            if child + 1 < end
                && self.sort_compare(base, bytes, comparator, abi, child, child + 1)? < 0
            {
                child += 1;
            }
            if self.sort_compare(base, bytes, comparator, abi, root, child)? >= 0 {
                break;
            }
            self.memory.swap(
                base + (root * bytes) as u64,
                base + (child * bytes) as u64,
                bytes,
            )?;
            root = child;
        }
        Ok(())
    }
    pub(super) fn crt(
        &self,
        operation: Crt,
        a: [u64; 13],
        abi: NativeAbi,
    ) -> Result<u64, NativeError> {
        let m = &self.memory;
        Ok(match operation {
            Crt::Argv => {
                if !(0..=2).contains(&(a[0] as i32)) {
                    return Err(NativeError::Unsupported);
                }
                0
            }
            Crt::Zero => 0,
            Crt::OnexitInit => {
                m.fill(a[0], 0, 24)?;
                0
            }
            Crt::OnexitRegister => self.onexit_register(a[0], a[1])?,
            Crt::Atexit => self.onexit_register(self.state()? + 608, a[0])?,
            Crt::OnexitExecute | Crt::Cexit => {
                self.onexit_execute(
                    if matches!(operation, Crt::Cexit) {
                        self.state()? + 608
                    } else {
                        a[0]
                    },
                    abi,
                )?;
                0
            }
            Crt::InitTerm | Crt::InitTermError => {
                let (mut at, end) = (a[0], a[1]);
                if at == 0 || end == 0 || end < at || (end - at) % 8 != 0 || end - at > 1048576 {
                    return Err(NativeError::Extent);
                }
                let error = matches!(operation, Crt::InitTermError);
                while at < end {
                    let callback = m.unsigned(at, 8)?;
                    if callback != 0 {
                        let result =
                            self.foreign(callback, abi, &[], if error { I32 } else { Void }, &[])?;
                        if error && result != 0 {
                            return Ok(result);
                        }
                    }
                    at += 8;
                }
                0
            }
            Crt::TypeInfo => {
                if m.unsigned(a[0], 8)? != 0 || m.unsigned(a[0] + 8, 8)? != 0 {
                    return Err(NativeError::Unsupported);
                }
                0
            }
            Crt::Sort => {
                let (base, count, bytes, comparator) = (a[0], a[1], a[2], a[3]);
                if count > 0x10000000 || bytes > 0x10000000 {
                    return Err(NativeError::Extent);
                }
                if count < 2 {
                    return Ok(0);
                }
                if bytes == 0 || count > 0x10000000 / bytes {
                    return Err(NativeError::Extent);
                }
                let (count, bytes) = (count as usize, bytes as usize);
                m.range(base, count * bytes, 3)?;
                for start in (0..count / 2).rev() {
                    self.sort_sift(base, bytes, comparator, abi, start, count)?;
                }
                for end in (1..count).rev() {
                    m.swap(base, base + (end * bytes) as u64, bytes)?;
                    self.sort_sift(base, bytes, comparator, abi, 0, end)?;
                }
                0
            }
            Crt::Errno => self.state()? + 632,
            Crt::Locale => self.state()? + 416,
        })
    }
    pub(super) fn math(
        &self,
        operation: Math,
        precision: NativeScalar,
        a: [u64; 13],
    ) -> Result<u64, NativeError> {
        let number = |bits| {
            if precision == NativeScalar::Float {
                f64::from(f32::from_bits(bits as u32))
            } else {
                f64::from_bits(bits)
            }
        };
        let (x, y) = (number(a[0]), number(a[1]));
        let value = match operation {
            Math::Sin => x.sin(),
            Math::Cos => x.cos(),
            Math::Atan2 => x.atan2(y),
            Math::Sqrt => x.sqrt(),
            Math::Floor => x.floor(),
            Math::Ceil => x.ceil(),
            Math::Acos => x.acos(),
            Math::Absolute => x.abs(),
            Math::Trunc => x.trunc(),
            Math::Log2 => x.log2(),
            Math::Tan => x.tan(),
            Math::Fmod => x % y,
            Math::Pow => {
                if x.abs() == 1.0 && y.is_infinite() {
                    f64::NAN
                } else {
                    x.powf(y)
                }
            }
            Math::Nextafter => {
                let (x, y) = (x as f32, y as f32);
                let next = if x.is_nan() || y.is_nan() {
                    x + y
                } else if x == y {
                    y
                } else if x == 0.0 {
                    f32::from_bits(if y < 0.0 { 0x80000001 } else { 1 })
                } else {
                    f32::from_bits(if (y > x) == (x > 0.0) {
                        x.to_bits().wrapping_add(1)
                    } else {
                        x.to_bits().wrapping_sub(1)
                    })
                };
                f64::from(next)
            }
            Math::Modf => {
                let whole = x.trunc();
                self.memory.put(a[1], 8, whole.to_bits())?;
                let fraction = if x.is_finite() {
                    x - whole
                } else if x.is_nan() {
                    f64::NAN
                } else {
                    0.0
                };
                if fraction == 0.0 {
                    fraction.copysign(x)
                } else {
                    fraction
                }
            }
            Math::Class => {
                return Ok(if x.is_nan() {
                    2
                } else if x.is_infinite() {
                    1
                } else if x == 0.0 {
                    0
                } else if x.abs()
                    < if precision == NativeScalar::Float {
                        f64::from(f32::MIN_POSITIVE)
                    } else {
                        f64::MIN_POSITIVE
                    }
                {
                    (-2i64) as u64
                } else {
                    u64::MAX
                });
            }
            Math::Sign => {
                return Ok(if x < 0.0 || (x == 0.0 && x.is_sign_negative()) {
                    0x8000
                } else {
                    0
                });
            }
        };
        Ok(if precision == NativeScalar::Float {
            u64::from((value as f32).to_bits())
        } else {
            value.to_bits()
        })
    }
}
