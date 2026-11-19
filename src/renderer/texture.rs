//! GPU textures created from caller-owned RGBA8 pixels.
//!
//! This is deliberately an upload primitive, not an asset pipeline: consumers
//! decide where pixels come from, how they are decoded, and when they are
//! released. The same handle can be sampled by sprites, HUD images, and meshes.

use crate::math::Vec2;

/// How a texture is filtered when sampled.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextureFilter {
    /// Blend neighbouring texels. Good for photographic or smooth artwork, but
    /// atlas tiles need padding to avoid bleeding into each other.
    #[default]
    Linear,
    /// Pick the nearest texel. Crisp for pixel art and safer for tight atlas
    /// packs, but magnified textures become blocky.
    Nearest,
}

/// What happens when a sample falls outside `0..=1` on an axis.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextureAddress {
    /// Repeat the edge texel.
    #[default]
    ClampToEdge,
    /// Wrap to the opposite edge. Pair this with a [`TextureRegion`] wider than
    /// `1.0` to tile an image.
    Repeat,
    /// Wrap and mirror on each repetition.
    MirrorRepeat,
}

/// Sampling behavior for one texture upload.
///
/// Defaults match the original texture path: linear filtering with
/// clamp-to-edge addressing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextureOptions {
    pub filter: TextureFilter,
    pub address: TextureAddress,
}

impl TextureOptions {
    /// Crisp, non-repeating sampling for pixel art.
    pub const NEAREST: Self = Self {
        filter: TextureFilter::Nearest,
        address: TextureAddress::ClampToEdge,
    };

    /// Tiling with crisp texels, for repeating patterns.
    pub const NEAREST_REPEAT: Self = Self {
        filter: TextureFilter::Nearest,
        address: TextureAddress::Repeat,
    };
}

fn to_address_mode(address: TextureAddress) -> wgpu::AddressMode {
    match address {
        TextureAddress::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        TextureAddress::Repeat => wgpu::AddressMode::Repeat,
        TextureAddress::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
    }
}

fn to_filter_mode(filter: TextureFilter) -> wgpu::FilterMode {
    match filter {
        TextureFilter::Linear => wgpu::FilterMode::Linear,
        TextureFilter::Nearest => wgpu::FilterMode::Nearest,
    }
}

/// Why a texture upload was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextureCreateError {
    /// At least one dimension was zero.
    ZeroSize { width: u32, height: u32 },
    /// A dimension exceeded the device's maximum 2D texture size.
    ExceedsMaxDimension { width: u32, height: u32, max: u32 },
    /// The pixel buffer did not contain exactly `width * height * 4` bytes.
    InvalidPixelDataLength { expected: usize, actual: usize },
}

impl std::fmt::Display for TextureCreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroSize { width, height } => {
                write!(f, "texture size {width}x{height} must be non-zero")
            }
            Self::ExceedsMaxDimension { width, height, max } => write!(
                f,
                "texture size {width}x{height} exceeds the device maximum {max}"
            ),
            Self::InvalidPixelDataLength { expected, actual } => write!(
                f,
                "texture needs {expected} RGBA bytes but received {actual}"
            ),
        }
    }
}

impl std::error::Error for TextureCreateError {}

/// Why an in-place RGBA8 texture update was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextureUpdateError {
    EmptyRegion,
    OutOfBounds {
        origin: [u32; 2],
        size: [u32; 2],
        texture_size: [u32; 2],
    },
    InvalidPixelDataLength {
        expected: usize,
        actual: usize,
    },
}

impl std::fmt::Display for TextureUpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyRegion => write!(f, "texture update must have nonzero width and height"),
            Self::OutOfBounds {
                origin,
                size,
                texture_size,
            } => write!(
                f,
                "texture update at {origin:?} of size {size:?} exceeds texture size {texture_size:?}"
            ),
            Self::InvalidPixelDataLength { expected, actual } => {
                write!(
                    f,
                    "texture update needs {expected} RGBA bytes but received {actual}"
                )
            }
        }
    }
}

impl std::error::Error for TextureUpdateError {}

fn rgba8_len(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
}

fn validate_update(
    texture_size: [u32; 2],
    origin: [u32; 2],
    size: [u32; 2],
    pixel_len: usize,
) -> Result<(), TextureUpdateError> {
    if size.contains(&0) {
        return Err(TextureUpdateError::EmptyRegion);
    }
    if (0..2).any(|axis| size[axis] > texture_size[axis].saturating_sub(origin[axis])) {
        return Err(TextureUpdateError::OutOfBounds {
            origin,
            size,
            texture_size,
        });
    }
    let expected = rgba8_len(size[0], size[1]);
    if expected != Some(pixel_len) {
        return Err(TextureUpdateError::InvalidPixelDataLength {
            expected: expected.unwrap_or(usize::MAX),
            actual: pixel_len,
        });
    }
    Ok(())
}

/// Check dimensions and pixel length before allocating GPU resources.
pub(crate) fn validate_texture(
    width: u32,
    height: u32,
    pixel_len: usize,
    max_dimension: u32,
) -> Result<(), TextureCreateError> {
    if width == 0 || height == 0 {
        return Err(TextureCreateError::ZeroSize { width, height });
    }
    if width > max_dimension || height > max_dimension {
        return Err(TextureCreateError::ExceedsMaxDimension {
            width,
            height,
            max: max_dimension,
        });
    }
    let expected = rgba8_len(width, height).ok_or(TextureCreateError::InvalidPixelDataLength {
        expected: usize::MAX,
        actual: pixel_len,
    })?;
    if pixel_len != expected {
        return Err(TextureCreateError::InvalidPixelDataLength {
            expected,
            actual: pixel_len,
        });
    }
    Ok(())
}

/// An RGBA8 sRGB texture uploaded to the renderer's device.
///
/// Pixels are interpreted as sRGB with straight alpha. Sampling behavior comes
/// from the [`TextureOptions`] used at upload and is fixed for the texture's
/// lifetime. Keep the handle alive while any draw references it.
pub struct Texture2d {
    texture: wgpu::Texture,
    queue: wgpu::Queue,
    _view: wgpu::TextureView,
    pub(crate) bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

/// A normalized sub-rectangle of a texture, used to sample an atlas tile.
///
/// `min` is the top-left corner and `max` the bottom-right corner in `0..=1`
/// texture coordinates, matching wgpu's top-left texture origin. Reversed
/// corners flip the image. Values outside `0..=1` are allowed: clamp-to-edge
/// sampling repeats the border texel, while repeat addressing tiles the
/// texture. Leave padding between atlas tiles when using linear filtering so
/// neighbouring tiles do not bleed into the sampled region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureRegion {
    pub min: Vec2,
    pub max: Vec2,
}

impl TextureRegion {
    /// The whole texture.
    pub const FULL: Self = Self {
        min: Vec2::new(0.0, 0.0),
        max: Vec2::new(1.0, 1.0),
    };

    /// A region from normalized top-left and bottom-right coordinates.
    pub const fn new(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }

    /// Convert a pixel rectangle inside a texture of the given size into
    /// normalized coordinates.
    ///
    /// The rectangle is not clamped to the texture; the sampler's addressing
    /// mode decides what happens outside `0..=1`.
    pub fn from_pixels(
        texture_width: u32,
        texture_height: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Self {
        if texture_width == 0 || texture_height == 0 {
            return Self::FULL;
        }
        let texture_width = texture_width as f32;
        let texture_height = texture_height as f32;
        Self {
            min: Vec2::new(x as f32 / texture_width, y as f32 / texture_height),
            max: Vec2::new(
                (u64::from(x) + u64::from(width)) as f32 / texture_width,
                (u64::from(y) + u64::from(height)) as f32 / texture_height,
            ),
        }
    }

    pub(crate) fn is_finite(&self) -> bool {
        self.min.x.is_finite()
            && self.min.y.is_finite()
            && self.max.x.is_finite()
            && self.max.y.is_finite()
    }
}

impl Default for TextureRegion {
    fn default() -> Self {
        Self::FULL
    }
}

impl std::fmt::Debug for Texture2d {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Texture2d")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Arguments for one texture upload, kept together to avoid a long parameter
/// list.
pub(crate) struct TextureUpload<'a> {
    pub(crate) label: &'a str,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba8: &'a [u8],
    pub(crate) options: TextureOptions,
}

/// Bind-group layout shared by every pipeline that samples a `Texture2d`.
pub(crate) fn texture_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("2d texture layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

impl Texture2d {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        upload: TextureUpload<'_>,
    ) -> Result<Self, TextureCreateError> {
        let TextureUpload {
            label,
            width,
            height,
            rgba8,
            options,
        } = upload;
        validate_texture(
            width,
            height,
            rgba8.len(),
            device.limits().max_texture_dimension_2d,
        )?;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        write_rgba8(queue, &texture, [0, 0], [width, height], rgba8);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("2d texture sampler"),
            address_mode_u: to_address_mode(options.address),
            address_mode_v: to_address_mode(options.address),
            address_mode_w: to_address_mode(options.address),
            mag_filter: to_filter_mode(options.filter),
            min_filter: to_filter_mode(options.filter),
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("2d texture bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        Ok(Self {
            texture,
            queue: queue.clone(),
            _view: view,
            bind_group,
            width,
            height,
        })
    }

    /// Replace all texels using tightly packed RGBA8 sRGB rows.
    ///
    /// The queued write becomes visible to subsequent submissions on the
    /// texture's original queue. Dimensions, sampler, and atlas regions stay
    /// unchanged. Invalid input leaves the texture untouched.
    pub fn update_rgba8(&self, rgba8: &[u8]) -> Result<(), TextureUpdateError> {
        self.update_region_rgba8([0, 0], [self.width, self.height], rgba8)
    }

    /// Replace a rectangle of texels. The origin is measured from the top
    /// left. Input rows are tightly packed (no stride or alignment padding),
    /// with exactly `size[0] * size[1] * 4` bytes in RGBA order.
    ///
    /// Like [`Self::update_rgba8`], this queues a write rather than waiting
    /// for GPU completion. It does not recreate the view or bind group.
    pub fn update_region_rgba8(
        &self,
        origin: [u32; 2],
        size: [u32; 2],
        rgba8: &[u8],
    ) -> Result<(), TextureUpdateError> {
        validate_update([self.width, self.height], origin, size, rgba8.len())?;
        write_rgba8(&self.queue, &self.texture, origin, size, rgba8);
        Ok(())
    }

    /// Texture width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Texture height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Convert a pixel rectangle in this texture into a normalized region.
    ///
    /// This is the usual way to select an atlas tile for
    /// [`UiImage::with_region`](crate::renderer::UiImage::with_region).
    pub fn region(&self, x: u32, y: u32, width: u32, height: u32) -> TextureRegion {
        TextureRegion::from_pixels(self.width, self.height, x, y, width, height)
    }
}

fn write_rgba8(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    [x, y]: [u32; 2],
    [width, height]: [u32; 2],
    rgba8: &[u8],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x, y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        rgba8,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_validation_checks_geometry_and_exact_unpadded_byte_count() {
        use TextureUpdateError::*;

        assert!(validate_update([8, 8], [5, 6], [3, 2], 24).is_ok());
        assert_eq!(validate_update([8, 8], [0, 0], [3, 0], 0), Err(EmptyRegion));
        for (origin, size) in [
            ([8, 0], [1, 1]),
            ([0, 7], [1, 2]),
            ([u32::MAX, 2], [8, 1]),
            ([1, 1], [u32::MAX, u32::MAX]),
        ] {
            assert_eq!(
                validate_update([8, 8], origin, size, 0),
                Err(OutOfBounds {
                    origin,
                    size,
                    texture_size: [8, 8]
                })
            );
        }
        for len in [23, 25] {
            assert_eq!(
                validate_update([8, 8], [1, 1], [3, 2], len),
                Err(InvalidPixelDataLength {
                    expected: 24,
                    actual: len
                })
            );
        }
        assert_eq!(
            validate_update([u32::MAX; 2], [0, 0], [u32::MAX; 2], usize::MAX),
            Err(InvalidPixelDataLength {
                expected: usize::MAX,
                actual: usize::MAX
            })
        );
    }

    #[test]
    fn pixel_region_endpoint_does_not_wrap_at_u32_limit() {
        let region = TextureRegion::from_pixels(u32::MAX, 16, u32::MAX, 0, u32::MAX, 16);
        assert_eq!(region.min, Vec2::new(1.0, 0.0));
        assert_eq!(region.max, Vec2::new(2.0, 1.0));
    }

    #[test]
    fn validation_rejects_zero_dimensions() {
        assert_eq!(
            validate_texture(0, 4, 0, 8192),
            Err(TextureCreateError::ZeroSize {
                width: 0,
                height: 4
            })
        );
        assert_eq!(
            validate_texture(4, 0, 0, 8192),
            Err(TextureCreateError::ZeroSize {
                width: 4,
                height: 0
            })
        );
    }

    #[test]
    fn validation_rejects_dimensions_beyond_the_device_limit() {
        assert_eq!(
            validate_texture(9000, 1, 9000 * 4, 8192),
            Err(TextureCreateError::ExceedsMaxDimension {
                width: 9000,
                height: 1,
                max: 8192
            })
        );
    }

    #[test]
    fn validation_requires_exactly_four_bytes_per_pixel() {
        assert_eq!(
            validate_texture(2, 2, 15, 8192),
            Err(TextureCreateError::InvalidPixelDataLength {
                expected: 16,
                actual: 15
            })
        );
        assert!(validate_texture(2, 2, 16, 8192).is_ok());
    }

    #[test]
    fn region_converts_pixels_to_normalized_coordinates() {
        let region = TextureRegion::from_pixels(32, 16, 16, 0, 16, 16);
        assert_eq!(region.min, Vec2::new(0.5, 0.0));
        assert_eq!(region.max, Vec2::new(1.0, 1.0));
    }

    #[test]
    fn region_defaults_to_the_full_texture() {
        assert_eq!(TextureRegion::default(), TextureRegion::FULL);
        assert_eq!(
            TextureRegion::from_pixels(0, 16, 0, 0, 0, 0),
            TextureRegion::FULL
        );
    }

    #[test]
    fn region_reports_non_finite_coordinates() {
        assert!(TextureRegion::FULL.is_finite());
        assert!(!TextureRegion::new(Vec2::new(f32::NAN, 0.0), Vec2::new(1.0, 1.0)).is_finite());
        assert!(!TextureRegion::new(Vec2::ZERO, Vec2::new(f32::INFINITY, 1.0)).is_finite());
    }

    #[test]
    fn sampling_options_default_to_linear_clamp() {
        let options = TextureOptions::default();
        assert_eq!(options.filter, TextureFilter::Linear);
        assert_eq!(options.address, TextureAddress::ClampToEdge);
        assert_eq!(TextureOptions::NEAREST.filter, TextureFilter::Nearest);
        assert_eq!(
            TextureOptions::NEAREST_REPEAT.address,
            TextureAddress::Repeat
        );
    }

    #[test]
    fn validation_reports_overflow_as_an_invalid_length() {
        assert_eq!(
            validate_texture(u32::MAX, u32::MAX, 0, u32::MAX),
            Err(TextureCreateError::InvalidPixelDataLength {
                expected: usize::MAX,
                actual: 0
            })
        );
    }
}
