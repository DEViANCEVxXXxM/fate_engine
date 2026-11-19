use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

const SLOT_COUNT: usize = 3;
const QUERY_COUNT: u32 = 4;
const TICKS_PER_PASS: u32 = 2;
const QUERY_SIZE: u64 = 8;
const BUFFER_SIZE: u64 = QUERY_COUNT as u64 * QUERY_SIZE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuFrameKind {
    Triangles2d,
    Sprites2d,
    TileMap2d,
    Meshes3d,
}

/// Asynchronously measured GPU pass durations. Surface acquisition,
/// presentation, and time between passes are not measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuFrameTiming {
    /// ID of the sampled submission. Reports can arrive on later frames.
    pub sample_id: u64,
    pub kind: GpuFrameKind,
    pub scene: Duration,
    pub overlay: Option<Duration>,
}

struct Pending {
    id: u64,
    kind: GpuFrameKind,
    overlay: bool,
    status: Receiver<Result<(), wgpu::BufferAsyncError>>,
}

struct Slot {
    buffer: wgpu::Buffer,
    pending: Option<Pending>,
}

pub(crate) struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: [Slot; SLOT_COUNT],
    period_ns: f64,
    next_id: u64,
}

impl GpuTimer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let period_ns = f64::from(queue.get_timestamp_period());
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY)
            || !period_ns.is_finite()
            || period_ns <= 0.0
        {
            return None;
        }
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("frame timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: QUERY_COUNT,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("frame timestamp resolve"),
                size: BUFFER_SIZE,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            slots: std::array::from_fn(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("frame timestamp readback"),
                    size: BUFFER_SIZE,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                pending: None,
            }),
            period_ns,
            next_id: 0,
        })
    }

    /// Skip instrumentation if the bounded readback pool is full.
    pub(crate) fn free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|slot| slot.pending.is_none())
    }

    pub(crate) fn writes(&self, overlay: bool) -> wgpu::RenderPassTimestampWrites<'_> {
        let start = if overlay { TICKS_PER_PASS } else { 0 };
        wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(start),
            end_of_pass_write_index: Some(start + 1),
        }
    }

    pub(crate) fn resolve(&self, encoder: &mut wgpu::CommandEncoder, slot: usize, overlay: bool) {
        let count = query_count(overlay);
        // All resolves and copies use the same queue. The next frame can
        // reuse the query set and resolve buffer without touching a mapped slot.
        encoder.resolve_query_set(&self.queries, 0..count, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(
            &self.resolve,
            0,
            &self.slots[slot].buffer,
            0,
            u64::from(count) * QUERY_SIZE,
        );
    }

    pub(crate) fn submitted(&mut self, slot: usize, kind: GpuFrameKind, overlay: bool) {
        let (sender, status) = mpsc::channel();
        let slot = &mut self.slots[slot];
        let size = u64::from(query_count(overlay)) * QUERY_SIZE;
        slot.buffer
            .slice(..size)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        slot.pending = Some(Pending {
            id: self.next_id,
            kind,
            overlay,
            status,
        });
        self.next_id = self.next_id.wrapping_add(1);
    }

    pub(crate) fn poll(&mut self, device: &wgpu::Device) -> Vec<GpuFrameTiming> {
        if self.slots.iter().all(|slot| slot.pending.is_none()) {
            return Vec::new();
        }
        let _ = device.poll(wgpu::PollType::Poll);
        let mut ready = Vec::new();
        for slot in &mut self.slots {
            let Some(pending) = &slot.pending else {
                continue;
            };
            match pending.status.try_recv() {
                Ok(Ok(())) => {
                    let size = u64::from(query_count(pending.overlay)) * QUERY_SIZE;
                    if let Ok(data) = slot.buffer.slice(..size).get_mapped_range() {
                        let bytes: &[u8] = data.as_ref();
                        if let Some((scene, overlay)) =
                            decode(bytes, self.period_ns, pending.overlay)
                        {
                            ready.push(GpuFrameTiming {
                                sample_id: pending.id,
                                kind: pending.kind,
                                scene,
                                overlay,
                            });
                        }
                    }
                    slot.buffer.unmap();
                }
                Err(TryRecvError::Empty) => continue,
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => {}
            }
            slot.pending = None;
        }
        ready.sort_unstable_by_key(|timing| timing.sample_id);
        ready
    }
}

fn query_count(overlay: bool) -> u32 {
    if overlay { QUERY_COUNT } else { TICKS_PER_PASS }
}

fn decode(bytes: &[u8], period_ns: f64, overlay: bool) -> Option<(Duration, Option<Duration>)> {
    if bytes.len() as u64 != u64::from(query_count(overlay)) * QUERY_SIZE {
        return None;
    }
    let (ticks, _) = bytes.as_chunks::<8>();
    let pass_time = |index: usize| {
        let start = u64::from_ne_bytes(ticks[index]);
        let end = u64::from_ne_bytes(ticks[index + 1]);
        Duration::try_from_secs_f64(end.checked_sub(start)? as f64 * period_ns * 1e-9).ok()
    };
    Some((
        pass_time(0)?,
        if overlay { Some(pass_time(2)?) } else { None },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_pass_ticks_without_charging_the_gap() {
        let bytes: Vec<_> = [10_u64, 14, 30, 32]
            .into_iter()
            .flat_map(u64::to_ne_bytes)
            .collect();
        let (scene, overlay) = decode(&bytes, 2.5, true).unwrap();
        assert_eq!(scene, Duration::from_nanos(10));
        assert_eq!(overlay, Some(Duration::from_nanos(5)));
        assert_eq!(decode(&bytes[..16], 2.5, false), Some((scene, None)));
        assert!(decode(&bytes[..16], 2.5, true).is_none());
        assert!(decode(&bytes, f64::INFINITY, true).is_none());
    }

    #[test]
    fn rejects_reversed_pass_timestamps() {
        let bytes: Vec<_> = [14_u64, 10]
            .into_iter()
            .flat_map(u64::to_ne_bytes)
            .collect();
        assert!(decode(&bytes, 1.0, false).is_none());
    }

    #[test]
    fn timestamp_slots_are_bounded_and_reusable_after_readback() {
        use super::super::test_support::test_device_with_features;

        let Some((device, queue)) = test_device_with_features(wgpu::Features::TIMESTAMP_QUERY)
        else {
            eprintln!("skipping GPU timing test: timestamp queries unavailable");
            return;
        };
        let Some(mut timer) = GpuTimer::new(&device, &queue) else {
            eprintln!("skipping GPU timing test: no valid timestamp period");
            return;
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("timestamp test target"),
            size: wgpu::Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let submit = |timer: &mut GpuTimer, with_overlay: bool| {
            let slot = timer.free_slot().expect("free readback slot");
            let mut encoder = device.create_command_encoder(&Default::default());
            for overlay in [false, true]
                .into_iter()
                .take(1 + usize::from(with_overlay))
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    timestamp_writes: Some(timer.writes(overlay)),
                    ..Default::default()
                });
            }
            timer.resolve(&mut encoder, slot, with_overlay);
            queue.submit(Some(encoder.finish()));
            timer.submitted(slot, GpuFrameKind::Meshes3d, with_overlay);
        };
        for index in 0..SLOT_COUNT {
            submit(&mut timer, index == 1);
        }
        assert!(timer.free_slot().is_none());
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("readback completes");
        let ready = timer.poll(&device);
        assert_eq!(ready.len(), SLOT_COUNT);
        for (index, sample) in ready.into_iter().enumerate() {
            assert_eq!(sample.sample_id, index as u64);
            assert_eq!(sample.overlay.is_some(), index == 1);
        }
        assert_eq!(timer.free_slot(), Some(0));
        assert!(timer.poll(&device).is_empty());
        submit(&mut timer, false);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("reused slot completes");
        assert_eq!(timer.poll(&device)[0].sample_id, SLOT_COUNT as u64);
    }
}
