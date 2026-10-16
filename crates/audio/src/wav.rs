//! Minimal WAV decoder for the embedded sound samples.
//!
//! Reads RIFF/WAVE files with PCM 8/16/24/32-bit integer or 32-bit float
//! samples, mixes all channels down to mono and returns `f32` samples in
//! `-1.0..=1.0`. Enough for short one-shot drum hits; no external crate.

/// A decoded mono sample.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SampleData {
    pub(crate) rate: u32,
    pub(crate) frames: Vec<f32>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum WavError {
    NotWav,
    Unsupported { format: u16, bits: u16 },
    Truncated,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> { Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?)) }

fn u32_at(b: &[u8], i: usize) -> Option<u32> { Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?)) }

pub(crate) fn decode(bytes: &[u8]) -> Result<SampleData, WavError> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(WavError::NotWav);
    }
    let mut fmt: Option<(u16, u16, u32, u16)> = None; // format, channels, rate, bits
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let size = u32_at(bytes, i + 4).ok_or(WavError::Truncated)? as usize;
        let body = i + 8;
        match id {
            b"fmt " => {
                let mut format = u16_at(bytes, body).ok_or(WavError::Truncated)?;
                let channels = u16_at(bytes, body + 2).ok_or(WavError::Truncated)?;
                let rate = u32_at(bytes, body + 4).ok_or(WavError::Truncated)?;
                let bits = u16_at(bytes, body + 14).ok_or(WavError::Truncated)?;
                // WAVE_FORMAT_EXTENSIBLE: the real format is the first two
                // bytes of the sub-format GUID.
                if format == 0xFFFE {
                    format = u16_at(bytes, body + 24).ok_or(WavError::Truncated)?;
                }
                fmt = Some((format, channels.max(1), rate, bits));
            }
            b"data" => {
                let (format, channels, rate, bits) = fmt.ok_or(WavError::NotWav)?;
                let end = (body + size).min(bytes.len());
                let data = &bytes[body..end];
                let frames = decode_frames(data, format, channels as usize, bits)?;
                return Ok(SampleData { rate, frames });
            }
            _ => {}
        }
        // Chunks are padded to an even size.
        i = body + size + (size & 1);
    }
    Err(WavError::Truncated)
}

fn decode_frames(data: &[u8], format: u16, channels: usize, bits: u16) -> Result<Vec<f32>, WavError> {
    let width = (bits as usize).div_ceil(8);
    let read: fn(&[u8]) -> f32 = match (format, bits) {
        (1, 8) => |b| (b[0] as f32 - 128.0) / 128.0,
        (1, 16) => |b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
        (1, 24) => |b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
        (1, 32) => |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
        (3, 32) => |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        _ => return Err(WavError::Unsupported { format, bits }),
    };
    let frame = width * channels;
    Ok(data
        .chunks_exact(frame)
        .map(|f| f.chunks_exact(width).map(read).sum::<f32>() / channels as f32)
        .collect())
}

#[cfg(test)]
pub(crate) fn encode_pcm16(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    b.extend_from_slice(&(channels * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_mono_pcm16() {
        let wav = encode_pcm16(44_100, 1, &[0, 16384, -32768]);
        let s = decode(&wav).unwrap();
        assert_eq!(s.rate, 44_100);
        assert_eq!(s.frames, vec![0.0, 0.5, -1.0]);
    }

    #[test]
    fn mixes_stereo_down_to_mono() {
        let wav = encode_pcm16(48_000, 2, &[16384, 0, -16384, -16384]);
        assert_eq!(decode(&wav).unwrap().frames, vec![0.25, -0.5]);
    }

    #[test]
    fn skips_unknown_chunks_and_rejects_non_wav() {
        let mut wav = encode_pcm16(22_050, 1, &[100]);
        // Insert an odd-sized "LIST" chunk before "fmt ".
        let list = [b"LIST".as_slice(), &3u32.to_le_bytes(), &[1, 2, 3, 0]].concat();
        wav.splice(12..12, list);
        assert_eq!(decode(&wav).unwrap().frames.len(), 1);
        assert_eq!(decode(b"not a wav file"), Err(WavError::NotWav));
    }
}
