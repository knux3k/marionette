//! GPU time per render pass from timestamp queries (decision 0103), for benchmarks only.
//!
//! Off unless asked for, and only where the device has `TIMESTAMP_QUERY`. The renderer asks
//! for a pair of timestamps per pass while recording; after the GPU finishes, the caller reads
//! the times back. Nothing here feeds the simulation or the picture.

use std::cell::RefCell;

/// Passes timed per frame; later passes go untimed.
const MAX_PASSES: u32 = 32;
/// Bytes per timestamp.
const STAMP: u64 = 8;

/// Owns the query set and two buffers: one the queries resolve into (GPU only) and one the CPU
/// maps to read them. `labels` names the passes recorded this frame, in query order; it is a
/// `RefCell` because passes are begun from closures that hold the renderer by shared
/// reference.
pub(crate) struct PassTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    labels: RefCell<Vec<&'static str>>,
    /// Nanoseconds per timestamp tick.
    period_ns: f64,
}

impl PassTimer {
    /// A timer, or `None` when the device cannot write timestamps.
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let bytes = u64::from(MAX_PASSES) * 2 * STAMP;
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage,
                mapped_at_creation: false,
            })
        };
        Some(Self {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("pass timer"),
                ty: wgpu::QueryType::Timestamp,
                count: MAX_PASSES * 2,
            }),
            resolve: buffer(
                "pass timer resolve",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            ),
            readback: buffer(
                "pass timer readback",
                wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            ),
            labels: RefCell::new(Vec::new()),
            period_ns: f64::from(queue.get_timestamp_period()),
        })
    }

    /// Timestamp writes for the next pass, named `label`; `None` once the set is full.
    pub(crate) fn writes(&self, label: &'static str) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let mut labels = self.labels.borrow_mut();
        let i = labels.len() as u32;
        if i >= MAX_PASSES {
            return None;
        }
        labels.push(label);
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: Some(2 * i),
            end_of_pass_write_index: Some(2 * i + 1),
        })
    }

    /// Copies this frame's timestamps to the readable buffer; record last, before submit.
    pub(crate) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let n = self.labels.borrow().len() as u32 * 2;
        if n == 0 {
            return;
        }
        encoder.resolve_query_set(&self.set, 0..n, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, u64::from(n) * STAMP);
    }

    /// Milliseconds per pass of the last frame, in recording order. Call after the GPU has
    /// finished that frame. Empties the frame's list either way.
    pub(crate) fn read(&self, device: &wgpu::Device) -> Result<Vec<(&'static str, f64)>, String> {
        let labels = std::mem::take(&mut *self.labels.borrow_mut());
        if labels.is_empty() {
            return Ok(Vec::new());
        }
        let bytes = labels.len() as u64 * 2 * STAMP;
        let slice = self.readback.slice(..bytes);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("GPU wait failed: {e}"))?;
        let times = {
            let view = slice
                .get_mapped_range()
                .map_err(|e| format!("cannot read pass timestamps: {e}"))?;
            labels
                .iter()
                .enumerate()
                .map(|(i, &label)| {
                    let at = i * 2 * STAMP as usize;
                    let stamp = |o: usize| {
                        let mut b = [0u8; 8];
                        b.copy_from_slice(&view[o..o + 8]);
                        u64::from_le_bytes(b)
                    };
                    let ticks = stamp(at + 8).saturating_sub(stamp(at));
                    (label, ticks as f64 * self.period_ns / 1e6)
                })
                .collect()
        };
        self.readback.unmap();
        Ok(times)
    }
}
