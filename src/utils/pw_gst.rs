#![cfg(all(target_os = "linux", feature = "pipewire-gst"))]

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};

use gstreamer as gst;
use gstreamer::prelude::*;

pub struct PwGstHandle {
    pipeline: gst::Pipeline,
}

impl PwGstHandle {
    pub fn stop(self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

pub fn start_pipewire_capture_to_buffer(
    node_id: u32,
    buffer: Arc<Mutex<Vec<f32>>>,
    input_gain: Arc<AtomicU32>,
) -> anyhow::Result<PwGstHandle> {
    // Ensure GStreamer is inited (idempotent)
    let _ = gst::init();

    // Build pipeline: pipewiresrc target-object=<id> ! audioconvert ! audioresample ! audio/x-raw,format=F32LE,channels=1,rate=16000 ! appsink name=sink
    let pipe = gst::Pipeline::new(Some("pw-capture"));

    let src = gst::ElementFactory::make("pipewiresrc")
        .build()
        .map_err(|_| anyhow::anyhow!("pipewiresrc not available"))?;
    src.set_property_from_str("target-object", &node_id.to_string());

    let convert = gst::ElementFactory::make("audioconvert")
        .build()
        .map_err(|_| anyhow::anyhow!("audioconvert not available"))?;
    let resample = gst::ElementFactory::make("audioresample")
        .build()
        .map_err(|_| anyhow::anyhow!("audioresample not available"))?;
    let caps = gst::Caps::builder("audio/x-raw")
        .field("format", "F32LE")
        .field("channels", 1i32)
        .field("rate", 16000i32)
        .build();
    let capsfilter = gst::ElementFactory::make("capsfilter")
        .property("caps", &caps)
        .build()
        .map_err(|_| anyhow::anyhow!("capsfilter not available"))?;

    let appsink = gst::ElementFactory::make("appsink")
        .property_from_str("name", "sink")
        .property("emit-signals", true)
        .property("sync", false)
        .build()
        .map_err(|_| anyhow::anyhow!("appsink not available"))?;

    pipe.add_many(&[&src, &convert, &resample, &capsfilter, &appsink])?;
    gst::Element::link_many(&[&src, &convert, &resample, &capsfilter, &appsink])?;

    // Set callback to collect samples
    let sink = appsink
        .downcast::<gstreamer::AppSink>()
        .map_err(|_| anyhow::anyhow!("failed to downcast appsink"))?;

    let buf_for_cb = buffer.clone();
    let gain_for_cb = input_gain.clone();
    sink.set_callbacks(
        gstreamer::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                if let Ok(sample) = sink.pull_sample() {
                    if let Some(buf) = sample.buffer() {
                        if let Ok(map) = buf.map_readable() {
                            let data = map.as_slice();
                            // Expect F32LE
                            let mut tmp: Vec<f32> = Vec::with_capacity(data.len() / 4);
                            let mut chunk = [0u8; 4];
                            let g =
                                f32::from_bits(gain_for_cb.load(Ordering::Relaxed));
                            for bytes in data.chunks_exact(4) {
                                chunk.copy_from_slice(bytes);
                                let mut v = f32::from_le_bytes(chunk);
                                if (g - 1.0).abs() > f32::EPSILON { v *= g; }
                                tmp.push(v);
                            }
                            if let Ok(mut dst) = buf_for_cb.lock() {
                                dst.extend_from_slice(&tmp);
                            }
                        }
                    }
                }
                // Drop sample
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    pipe.set_state(gst::State::Playing)?;
    Ok(PwGstHandle { pipeline: pipe })
}

