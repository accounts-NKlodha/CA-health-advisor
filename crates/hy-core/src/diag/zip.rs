//! A minimal zip file of stored (uncompressed) entries: what the diagnostics zip needs,
//! without a compression dependency. Windows Explorer and every unzip tool open it. Names
//! are ASCII (the bundle's fixed names); sizes stay well under 4 GB, so no Zip64.

/// The CRC-32 (IEEE 802.3, as zip uses) of `data`.
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// A date and time for the entries, as the zip format stores it (local time, two-second
/// steps, years 1980 to 2107).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DosTime {
    /// The DOS time field.
    pub time: u16,
    /// The DOS date field.
    pub date: u16,
}

impl DosTime {
    /// From calendar parts (clamped into the format's range).
    #[must_use]
    pub fn new(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> Self {
        let y = u16::try_from(year.clamp(1980, 2107) - 1980).unwrap_or(0);
        let part = |v: u32, max: u32| u16::try_from(v.min(max)).unwrap_or(0);
        Self {
            time: (part(hour, 23) << 11) | (part(min, 59) << 5) | (part(sec, 59) / 2),
            date: (y << 9) | (part(month.max(1), 12) << 5) | part(day.max(1), 31),
        }
    }
}

fn u16le(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn u32le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn len32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A zip holding `entries` (name, contents) in order, stored.
#[must_use]
pub fn write(entries: &[(&str, &[u8])], at: DosTime) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = len32(out.len());
        let crc = crc32(data);
        let size = len32(data.len());
        let name = name.as_bytes();
        // Local file header.
        u32le(&mut out, 0x0403_4b50);
        u16le(&mut out, 20); // version needed
        u16le(&mut out, 0); // flags
        u16le(&mut out, 0); // stored
        u16le(&mut out, at.time);
        u16le(&mut out, at.date);
        u32le(&mut out, crc);
        u32le(&mut out, size);
        u32le(&mut out, size);
        u16le(&mut out, u16::try_from(name.len()).unwrap_or(u16::MAX));
        u16le(&mut out, 0); // extra
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        // Central directory entry.
        u32le(&mut central, 0x0201_4b50);
        u16le(&mut central, 20); // made by
        u16le(&mut central, 20); // needed
        u16le(&mut central, 0);
        u16le(&mut central, 0);
        u16le(&mut central, at.time);
        u16le(&mut central, at.date);
        u32le(&mut central, crc);
        u32le(&mut central, size);
        u32le(&mut central, size);
        u16le(&mut central, u16::try_from(name.len()).unwrap_or(u16::MAX));
        u16le(&mut central, 0); // extra
        u16le(&mut central, 0); // comment
        u16le(&mut central, 0); // disk
        u16le(&mut central, 0); // internal attributes
        u32le(&mut central, 0); // external attributes
        u32le(&mut central, offset);
        central.extend_from_slice(name);
    }
    let cd_offset = len32(out.len());
    let cd_size = len32(central.len());
    let count = u16::try_from(entries.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&central);
    u32le(&mut out, 0x0605_4b50);
    u16le(&mut out, 0);
    u16le(&mut out, 0);
    u16le(&mut out, count);
    u16le(&mut out, count);
    u32le(&mut out, cd_size);
    u32le(&mut out, cd_offset);
    u16le(&mut out, 0);
    out
}

fn rd16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn rd32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The entries of a stored zip as [`write`] makes it, read through its central
/// directory, each checked against its CRC. `None` if it is not such a zip.
#[must_use]
pub fn read(bytes: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let eocd = bytes.len().checked_sub(22)?;
    if rd32(bytes, eocd)? != 0x0605_4b50 {
        return None;
    }
    let count = usize::from(rd16(bytes, eocd + 10)?);
    let mut at = usize::try_from(rd32(bytes, eocd + 16)?).ok()?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if rd32(bytes, at)? != 0x0201_4b50 || rd16(bytes, at + 10)? != 0 {
            return None;
        }
        let crc = rd32(bytes, at + 16)?;
        let size = usize::try_from(rd32(bytes, at + 20)?).ok()?;
        let name_len = usize::from(rd16(bytes, at + 28)?);
        let extra = usize::from(rd16(bytes, at + 30)?);
        let comment = usize::from(rd16(bytes, at + 32)?);
        let local = usize::try_from(rd32(bytes, at + 42)?).ok()?;
        let name = String::from_utf8(bytes.get(at + 46..at + 46 + name_len)?.to_vec()).ok()?;
        if rd32(bytes, local)? != 0x0403_4b50 {
            return None;
        }
        let data_at = local
            + 30
            + usize::from(rd16(bytes, local + 26)?)
            + usize::from(rd16(bytes, local + 28)?);
        let data = bytes.get(data_at..data_at + size)?.to_vec();
        if crc32(&data) != crc {
            return None;
        }
        out.push((name, data));
        at += 46 + name_len + extra + comment;
    }
    Some(out)
}
