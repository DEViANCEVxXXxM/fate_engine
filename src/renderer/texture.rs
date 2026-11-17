//! GPU textures created from caller-owned RGBA8 pixels.
//!
//! This is deliberately an upload primitive, not an asset pipeline: consumers
//! decide where pixels come from, how they are decoded, and when they are
//! released. Textures are sampled by [`crate::renderer::UiImage`] draws.

use crate::math::Vec2;

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
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(TextureCreateError::InvalidPixelDataLength {
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
/// Pixels are interpreted as sRGB with straight alpha. Sampling uses linear
/// filtering and clamp-to-edge addressing at the image's natural size. Keep the
/// handle alive while any [`UiImage`](crate::renderer::UiImage) references it.
pub struct Texture2d {
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
    pub(crate) bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

/// A normalized sub-rectangle of a texture, used to sample an atlas tile.
///
/// `min` is the top-left corner and `max` the bottom-right corner in `0..=1`
/// texture coordinates, matching wgpu's top-left texture origin. Reversed
/// corners flip the image. Values outside `0..=1` are allowed; clamp-to-edge
/// sampling repeats the border texel. Leave padding between atlas tiles when
/// using linear filtering so neighbouring tiles do not bleed into the sampled
/// region.
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
    /// The rectangle is not clamped to the texture; callers that pass an
    /// out-of-range tile get clamp-to-edge sampling at the border.
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
                (x + width) as f32 / texture_width,
                (y + height) as f32 / texture_height,
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

impl Texture2d {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        label: &str,
        width: u32,
        height: u32,
        rgba8: &[u8],
    ) -> Result<Self, TextureCreateError> {
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
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
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
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui image sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui image bind group"),
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
            _texture: texture,
            _view: view,
            bind_group,
            width,
            height,
        })
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

#[cfg(test)]
mod tests {
    use super::*;

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
