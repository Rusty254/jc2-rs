//! FMOD sound banks: `sound\fmod\*.fsb`, FSB4. A 48-byte header (`FSB4`, the sample
//! count, the size of the sample headers, the size of the data, version `0x40000`,
//! flags — JC2's carry `0x20`, MPEG frames padded), then one header per sample (its
//! own size first, 80 bytes or more: a 30-byte name, length in samples, length in
//! bytes, loop points, mode flags, default frequency, volume, pan, priority, channel
//! count, min and max distance), then the samples' data back to back, each a multiple
//! of 32 bytes. Every sample in JC2's banks is MPEG audio (mode `0x200`), 16–44.1 kHz,
//! mono or stereo, its frames padded to an even length; `mpeg` returns a sample's frames
//! packed into a plain MP3 stream any decoder plays.
//!
//! The `.fev` event files that say which samples an event plays (with pitch and volume
//! randomisation, distance curves, layers) are `crate::fev`.

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// The name the sound designer gave the file (`Pistol_New_Full_2_Mid.wav`), cut to
    /// 30 characters.
    pub name: String,
    pub length_samples: u32,
    pub mode: u32,
    pub frequency: u32,
    pub channels: u16,
    pub loop_start: u32,
    pub loop_end: u32,
    pub min_distance: f32,
    pub max_distance: f32,
    /// Where its data is in the bank.
    pub offset: usize,
    pub size: usize,
}

pub const MODE_LOOP_NORMAL: u32 = 0x2;
pub const MODE_MPEG: u32 = 0x200;
pub const MODE_3D: u32 = 0x10_0000;

impl Sample {
    pub fn is_mpeg(&self) -> bool {
        self.mode & MODE_MPEG != 0
    }
}

#[derive(Clone, Debug)]
pub struct SoundBank {
    pub samples: Vec<Sample>,
}

fn u16_at(d: &[u8], o: usize) -> Result<u16> {
    d.get(o..o + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| Error::format("fsb: truncated"))
}

fn u32_at(d: &[u8], o: usize) -> Result<u32> {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| Error::format("fsb: truncated"))
}

impl SoundBank {
    pub fn parse(d: &[u8]) -> Result<Self> {
        Self::parse_with(d, d.len())
    }

    /// How many bytes at the start of the file hold the bank header and the sample
    /// headers: enough for [`SoundBank::parse_headers`]. The big streamed banks of the
    /// music and the cutscenes (over 100 MB) are read with this and one
    /// [`SoundBank::sample_range`] per sample.
    pub fn header_len(d: &[u8]) -> Result<usize> {
        if d.get(0..4) != Some(b"FSB4") {
            return Err(Error::format("not an FSB4 sound bank"));
        }
        Ok(48 + u32_at(d, 8)? as usize)
    }

    /// Parse a bank from its first [`SoundBank::header_len`] bytes; `file_len` is the
    /// size of the whole file. The samples' `offset`s are file offsets.
    pub fn parse_headers(d: &[u8], file_len: usize) -> Result<Self> {
        Self::parse_with(d, file_len)
    }

    /// Where sample `i`'s data lies in the file.
    pub fn sample_range(&self, i: usize) -> std::ops::Range<usize> {
        let s = &self.samples[i];
        s.offset..s.offset + s.size
    }

    fn parse_with(d: &[u8], file_len: usize) -> Result<Self> {
        if d.get(0..4) != Some(b"FSB4") {
            return Err(Error::format("not an FSB4 sound bank"));
        }
        let count = u32_at(d, 4)? as usize;
        let headers = u32_at(d, 8)? as usize;
        let flags = u32_at(d, 20)?;
        let basic = flags & 2 != 0;
        let mut pos = 48;
        let mut offset = 48 + headers;
        let mut samples: Vec<Sample> = Vec::with_capacity(count);
        for i in 0..count {
            let (mut s, size) = if basic && i > 0 {
                // only the lengths; the rest as the first
                let mut s = samples[0].clone();
                s.length_samples = u32_at(d, pos)?;
                let size = u32_at(d, pos + 4)? as usize;
                pos += 8;
                (s, size)
            } else {
                let len = u16_at(d, pos)? as usize;
                if len < 80 {
                    return Err(Error::format("fsb: short sample header"));
                }
                let name = d
                    .get(pos + 2..pos + 32)
                    .ok_or_else(|| Error::format("fsb: truncated"))?;
                let name = String::from_utf8_lossy(name.split(|&b| b == 0).next().unwrap_or(&[]))
                    .into_owned();
                let h = pos + 32;
                let s = Sample {
                    name,
                    length_samples: u32_at(d, h)?,
                    loop_start: u32_at(d, h + 8)?,
                    loop_end: u32_at(d, h + 12)?,
                    mode: u32_at(d, h + 16)?,
                    frequency: u32_at(d, h + 20)?,
                    channels: u16_at(d, h + 30)?,
                    min_distance: f32::from_bits(u32_at(d, h + 32)?),
                    max_distance: f32::from_bits(u32_at(d, h + 36)?),
                    offset: 0,
                    size: 0,
                };
                let size = u32_at(d, h + 4)? as usize;
                pos += len;
                (s, size)
            };
            s.offset = offset;
            s.size = size;
            if offset + size > file_len {
                return Err(Error::format("fsb: sample data past the end"));
            }
            offset += size.div_ceil(32) * 32;
            samples.push(s);
        }
        Ok(Self { samples })
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.samples.iter().position(|s| s.name == name)
    }

    /// Every sample whose name starts with `prefix` (a set of variations:
    /// `Footstep_Dirt_Run_` → `_01` … `_05`).
    pub fn starting_with<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = usize> + 'a {
        self.samples
            .iter()
            .enumerate()
            .filter(move |(_, s)| s.name.starts_with(prefix))
            .map(|(i, _)| i)
    }

    pub fn data<'a>(&self, bank: &'a [u8], i: usize) -> &'a [u8] {
        let s = &self.samples[i];
        &bank[s.offset..s.offset + s.size]
    }

    /// Sample `i` as a plain MP3 stream: its MPEG frames with the bank's padding taken
    /// out. `None` if it is not MPEG audio or no frame parses.
    pub fn mpeg(&self, bank: &[u8], i: usize) -> Option<Vec<u8>> {
        if !self.samples[i].is_mpeg() {
            return None;
        }
        pack_mpeg(self.data(bank, i))
    }

    /// Like [`SoundBank::mpeg`] for a sample whose bytes (`sample_range`) were read
    /// from the file on their own.
    pub fn mpeg_of(&self, i: usize, sample: &[u8]) -> Option<Vec<u8>> {
        if !self.samples[i].is_mpeg() {
            return None;
        }
        pack_mpeg(sample)
    }
}

impl SoundBank {
    /// A multichannel MPEG sample (the music's stems: 6 channels = three stereo pairs) as
    /// one plain MP3 stream per pair. The frames of the pairs alternate in the data, each
    /// padded to 16 bytes (a 418-byte frame is followed by 14 bytes that look like a header
    /// but are padding), so the data is walked frame by frame instead of scanned.
    pub fn mpeg_pairs(&self, i: usize, sample: &[u8]) -> Option<Vec<Vec<u8>>> {
        let s = &self.samples[i];
        if !s.is_mpeg() || s.channels < 2 {
            return None;
        }
        let pairs = (s.channels as usize / 2).max(1);
        let mut out = vec![Vec::new(); pairs];
        let (mut pos, mut n) = (0usize, 0usize);
        while pos + 4 <= sample.len() {
            if sample[pos] != 0xff || sample[pos + 1] & 0xe0 != 0xe0 {
                break;
            }
            let Some(len) = frame_length(&sample[pos..pos + 4]) else {
                break;
            };
            let end = (pos + len).min(sample.len());
            out[n % pairs].extend_from_slice(&sample[pos..end]);
            n += 1;
            pos = (pos + len).next_multiple_of(16);
        }
        out.iter().all(|s| !s.is_empty()).then_some(out)
    }
}

/// The MPEG frames of padded sample data, back to back.
fn pack_mpeg(d: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(d.len());
    let mut pos = 0;
    while pos + 4 <= d.len() {
        if d[pos] != 0xff || d[pos + 1] & 0xe0 != 0xe0 {
            pos += 1;
            continue;
        }
        let Some(len) = frame_length(&d[pos..pos + 4]) else {
            pos += 1;
            continue;
        };
        let end = (pos + len).min(d.len());
        out.extend_from_slice(&d[pos..end]);
        pos = end;
    }
    (!out.is_empty()).then_some(out)
}

/// The length in bytes of the MPEG audio frame with this header (layers II and III).
pub fn frame_length(h: &[u8]) -> Option<usize> {
    let version = (h[1] >> 3) & 3; // 0 = 2.5, 2 = 2, 3 = 1
    let layer = (h[1] >> 1) & 3; // 1 = III, 2 = II, 3 = I
    let bitrate_index = (h[2] >> 4) as usize;
    let rate_index = ((h[2] >> 2) & 3) as usize;
    let padding = ((h[2] >> 1) & 1) as usize;
    if version == 1 || layer == 0 || bitrate_index == 0 || bitrate_index == 15 || rate_index == 3 {
        return None;
    }
    const V1_L3: [usize; 15] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    const V1_L2: [usize; 15] = [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
    ];
    const V1_L1: [usize; 15] = [
        0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
    ];
    const V2_L23: [usize; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    const V2_L1: [usize; 15] = [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
    ];
    let mpeg1 = version == 3;
    let kbps = match (mpeg1, layer) {
        (true, 1) => V1_L3[bitrate_index],
        (true, 2) => V1_L2[bitrate_index],
        (true, _) => V1_L1[bitrate_index],
        (false, 3) => V2_L1[bitrate_index],
        (false, _) => V2_L23[bitrate_index],
    };
    let rate = match version {
        3 => [44100, 48000, 32000][rate_index],
        2 => [22050, 24000, 16000][rate_index],
        _ => [11025, 12000, 8000][rate_index],
    };
    let bps = kbps * 1000;
    Some(match layer {
        3 => (12 * bps / rate + padding) * 4,
        1 if !mpeg1 => 72 * bps / rate + padding,
        _ => 144 * bps / rate + padding,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_lengths() {
        // MPEG-1 layer III, 128 kbit/s, 44.1 kHz: 417 bytes (418 padded)
        assert_eq!(frame_length(&[0xff, 0xfb, 0x90, 0x00]), Some(417));
        assert_eq!(frame_length(&[0xff, 0xfb, 0x92, 0x00]), Some(418));
        // MPEG-2 layer III, 64 kbit/s, 22.05 kHz: 208
        assert_eq!(frame_length(&[0xff, 0xf3, 0x80, 0x00]), Some(208));
    }

    /// Needs JC2_TEST_DIR with `weapons_common.fsb`.
    #[test]
    fn weapons_bank() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read("weapons_common.fsb") else {
            return;
        };
        let bank = SoundBank::parse(&data).unwrap();
        assert_eq!(bank.samples.len(), 132);
        let first = &bank.samples[0];
        assert_eq!(first.name, "Mag_In_01.wav");
        assert_eq!((first.frequency, first.channels), (22050, 1));
        assert_eq!(first.length_samples, 4608);
        let last = bank.samples.last().unwrap();
        assert_eq!(last.offset + last.size.div_ceil(32) * 32, data.len());
        let pistol = bank.find("Pistol_New_Full_2_Mid.wav").unwrap();
        let mp3 = bank.mpeg(&data, pistol).unwrap();
        assert!(mp3.len() > 1000 && mp3.len() <= bank.samples[pistol].size);
        assert_eq!(&mp3[..2], &[0xff, mp3[1]]);
        assert!(bank.samples.iter().all(Sample::is_mpeg));
    }
}
