use anyhow::Result;
use cpal::traits::DeviceTrait;
use std::sync::{Arc, Mutex};

pub fn build_input_stream_f32(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    buf: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream> {
    Ok(match sample_format {
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| {
                let mut b = buf.lock().unwrap();
                b.reserve(data.len());
                for &s in data {
                    b.push(s as f32 / 32768.0);
                }
            },
            move |e| eprintln!("cpal error: {e}"),
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _| {
                let mut b = buf.lock().unwrap();
                b.reserve(data.len());
                for &s in data {
                    b.push((s as f32 - 32768.0) / 32768.0);
                }
            },
            move |e| eprintln!("cpal error: {e}"),
            None,
        )?,
        _ => device.build_input_stream(
            &config,
            move |data: &[f32], _| {
                let mut b = buf.lock().unwrap();
                b.extend_from_slice(data);
            },
            move |e| eprintln!("cpal error: {e}"),
            None,
        )?,
    })
}
