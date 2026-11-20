//! Shared helpers for renderer tests.
//!
//! GPU tests are skipped when no adapter/device is available so the headless
//! quality gate can run on machines without graphics hardware.

/// Request a test adapter and device, or `None` when none is available.
pub(crate) fn test_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    test_device_with_features(wgpu::Features::empty())
}

pub(crate) fn test_device_with_features(
    required_features: wgpu::Features,
) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    if !adapter.features().contains(required_features) {
        return None;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features,
        ..Default::default()
    }))
    .ok()?;
    Some((device, queue))
}

/// Log that a GPU test was skipped because the host has no usable adapter.
pub(crate) fn skip_without_adapter() {
    eprintln!("skipping GPU test: no wgpu adapter/device available");
}

/// A two-texel red/green texture for sampling and batch tests.
pub(crate) fn test_image_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    options: crate::renderer::TextureOptions,
) -> crate::renderer::Texture2d {
    crate::renderer::Texture2d::new(
        device,
        queue,
        layout,
        crate::renderer::texture::TextureUpload {
            label: "test image",
            width: 2,
            height: 1,
            rgba8: &[255, 0, 0, 255, 0, 255, 0, 255],
            options,
        },
    )
    .expect("texture")
}

/// Render into a `size` square target, read every pixel back, and return the
/// pixels plus the number of draws the closure recorded.
pub(crate) fn read_quad_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: u32,
    draw: impl FnOnce(&mut wgpu::RenderPass<'_>) -> usize,
) -> (Vec<[u8; 4]>, usize) {
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("image test target"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let bytes_per_row = size * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("image readback"),
        size: (bytes_per_row * size) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("image test encoder"),
    });
    let draws = {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("image test pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        draw(&mut pass)
    };
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(size),
            },
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("device poll failed");
    receiver
        .recv()
        .expect("map callback never ran")
        .expect("buffer mapping failed");

    let data = slice.get_mapped_range().expect("mapped range");
    let (pixels, _) = data.as_ref().as_chunks::<4>();
    let pixels = pixels.to_vec();
    drop(data);
    readback.unmap();
    (pixels, draws)
}
