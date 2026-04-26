use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result, bail};
use symphonia::core::audio::{AudioBufferRef, SampleBuffer};
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::{get_codecs, get_probe};

#[derive(Debug, Clone)]
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub channels: usize,
    pub sample_rate: u32,
}

impl DecodedAudio {
    pub fn frame_count(&self) -> usize {
        self.samples.len().checked_div(self.channels).unwrap_or(0)
    }

    pub fn into_output_format(
        self,
        target_sample_rate: u32,
        target_channels: usize,
    ) -> Result<Self> {
        if target_channels == 0 {
            bail!("audio device has zero output channels");
        }

        let remapped = remap_channels(&self.samples, self.channels, target_channels);
        let samples = if self.sample_rate == target_sample_rate {
            remapped
        } else {
            resample_linear(
                &remapped,
                target_channels,
                self.sample_rate,
                target_sample_rate,
            )
        };

        Ok(Self {
            samples,
            channels: target_channels,
            sample_rate: target_sample_rate,
        })
    }
}

pub fn decode_file(path: &Path) -> Result<DecodedAudio> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }

    let probed = get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .with_context(|| format!("unrecognized audio format: {}", path.display()))?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .context("no decodable audio track found")?;

    let track_id = track.id;
    let mut decoder = get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("failed to create audio decoder")?;

    let mut samples = Vec::new();
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut channels = track
        .codec_params
        .channels
        .map(|channels| channels.count())
        .unwrap_or(0);

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(error) => return Err(error).context("error while reading audio packet"),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(error).context("error while decoding audio"),
        };

        append_decoded(&mut samples, &mut sample_rate, &mut channels, decoded)?;
    }

    if samples.is_empty() {
        bail!("audio file has no decodable samples: {}", path.display());
    }
    if sample_rate == 0 || channels == 0 {
        bail!(
            "audio file has no valid sample rate or channel count: {}",
            path.display()
        );
    }

    Ok(DecodedAudio {
        samples,
        channels,
        sample_rate,
    })
}

fn append_decoded(
    samples: &mut Vec<f32>,
    sample_rate: &mut u32,
    channels: &mut usize,
    decoded: AudioBufferRef<'_>,
) -> Result<()> {
    let spec = *decoded.spec();
    if *sample_rate == 0 {
        *sample_rate = spec.rate;
    }
    if *channels == 0 {
        *channels = spec.channels.count();
    }
    if *sample_rate != spec.rate || *channels != spec.channels.count() {
        bail!("audio streams with variable format are not supported");
    }

    let mut sample_buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
    sample_buffer.copy_interleaved_ref(decoded);
    samples.extend_from_slice(sample_buffer.samples());
    Ok(())
}

fn remap_channels(samples: &[f32], source_channels: usize, target_channels: usize) -> Vec<f32> {
    if source_channels == target_channels {
        return samples.to_vec();
    }

    let frame_count = samples.len() / source_channels;
    let mut out = Vec::with_capacity(frame_count * target_channels);

    for frame in 0..frame_count {
        let offset = frame * source_channels;
        let mono = if source_channels == 1 {
            samples[offset]
        } else {
            let sum: f32 = samples[offset..offset + source_channels].iter().sum();
            sum / source_channels as f32
        };

        for channel in 0..target_channels {
            let sample = if target_channels < source_channels {
                mono
            } else if channel < source_channels {
                samples[offset + channel]
            } else {
                mono
            };
            out.push(sample);
        }
    }

    out
}

fn resample_linear(
    samples: &[f32],
    channels: usize,
    source_rate: u32,
    target_rate: u32,
) -> Vec<f32> {
    let source_frames = samples.len() / channels;
    if source_frames == 0 {
        return Vec::new();
    }

    let target_frames = ((source_frames as u64 * target_rate as u64) / source_rate as u64) as usize;
    let mut out = Vec::with_capacity(target_frames * channels);
    let ratio = source_rate as f64 / target_rate as f64;

    for target_frame in 0..target_frames {
        let source_position = target_frame as f64 * ratio;
        let left_frame = source_position.floor() as usize;
        let right_frame = (left_frame + 1).min(source_frames - 1);
        let fraction = (source_position - left_frame as f64) as f32;

        for channel in 0..channels {
            let left = samples[left_frame * channels + channel];
            let right = samples[right_frame * channels + channel];
            out.push(left + (right - left) * fraction);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaps_mono_to_stereo() {
        let remapped = remap_channels(&[0.25, 0.5], 1, 2);
        assert_eq!(remapped, vec![0.25, 0.25, 0.5, 0.5]);
    }

    #[test]
    fn remaps_stereo_to_mono_average() {
        let remapped = remap_channels(&[1.0, 0.0, 0.25, 0.75], 2, 1);
        assert_eq!(remapped, vec![0.5, 0.5]);
    }
}
