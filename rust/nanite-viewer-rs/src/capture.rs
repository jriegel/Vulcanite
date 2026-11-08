//! Frame capture: render offscreen, read the colour attachment back, write a PNG.
//!
//! `--capture <png> --captureframes <N>` in the C++ viewer saves the **last** frame
//! (`base/vulkanexamplebase.cpp::render` sets `capture.pending = (framesRendered + 1 >= frames)`, and
//! `submitFrame` then does `vkQueueWaitIdle` + `saveSwapchainImage` *before* presenting). The same
//! ordering is implemented in [`crate::app`]: the capture is taken from an offscreen target that is
//! never presented, so the readback cannot race a present.
//!
//! Why offscreen at all: `Instance::get_default_config` reports the surface's usages, and on Windows
//! a swapchain surface normally does **not** include `COPY_SRC`. The C++ code gets away with it
//! because it re-transitions the swapchain image into `TRANSFER_SRC_OPTIMAL` by hand. Here the frame
//! is rendered into a `COPY_SRC | RENDER_ATTACHMENT` texture instead.
//!
//! The capture texture's **format is not free**: wgpu checks a render pass' colour attachment format
//! against the pipeline's, and every pipeline in [`crate::renderer`] is built against the surface
//! format chosen by [`crate::app::pick_surface_config`]. So [`CaptureTarget::new`] takes that format
//! rather than a private constant - an offscreen target with a different format fails with
//! "Render pipeline targets are incompatible with render pass".

use anyhow::{Context, Result};
use image::RgbaImage;

/// The format a capture target uses when the caller has no pipeline format to match, e.g. a test.
///
/// The C++ swapchain prefers `B8G8R8A8_UNORM` and `saveSwapchainImage` swaps B->R on the way out, so
/// the golden PNGs are plain RGBA. Writing `Rgba8Unorm` directly produces the same bytes without the
/// swap, and it is a format wgpu guarantees is `COPY_SRC`-able.
pub const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`. `wgpu-core` rejects a `bytes_per_row` that is not a multiple
/// of this (`command/transfer.rs`: `if aligned && bytes_per_row % COPY_BYTES_PER_ROW_ALIGNMENT != 0`),
/// so a readback row is padded and then unpadded again.
pub const BYTES_PER_ROW_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

/// `saveSwapchainImage`'s `bufferSize = width * height * 4`: the C++ copy is **tightly packed**
/// because Vulkan's default `imageRowLength` is 0 ("same as the extent") and 4-byte rows already
/// satisfy its 4-byte requirement. wgpu's 256-byte rule is the only reason padding exists here.
pub fn padded_bytes_per_row(width: u32) -> u32 {
    let raw = width * 4;
    (raw + BYTES_PER_ROW_ALIGNMENT - 1) / BYTES_PER_ROW_ALIGNMENT * BYTES_PER_ROW_ALIGNMENT
}

/// Drop the per-row padding written by a `copy_texture_to_buffer`.
pub fn unpad_rows(padded: &[u8], width: u32, height: u32) -> Vec<u8> {
    let stride = padded_bytes_per_row(width) as usize;
    let row = width as usize * 4;
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        out.extend_from_slice(&padded[y * stride..y * stride + row]);
    }
    out
}

/// `true` when a format's byte order is B,G,R,A, so a readback has to swap channels 0 and 2 to get
/// the RGBA the PNG writer (and the C++ goldens) expect.
pub fn is_bgra(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    )
}

/// Swap the B and R bytes of every pixel, in place. The C++ `saveSwapchainImage` does exactly this
/// (`base/vulkanexamplebase.cpp`: it copies from a `B8G8R8A8_UNORM` image and writes `dst[0]=src[2]`,
/// `dst[2]=src[0]`), which is why a `Bgra8Unorm` capture must be swapped before it becomes a PNG.
pub fn swap_bgra_to_rgba(bytes: &mut [u8]) {
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

/// An offscreen colour + depth pair to render a captured frame into.
pub struct CaptureTarget {
    pub width: u32,
    pub height: u32,
    /// The colour attachment's format. Must be the format every render pipeline was created with.
    pub format: wgpu::TextureFormat,
    color: wgpu::Texture,
    /// The main pass' depth attachment. `Depth32Float` is what the viewer's swapchain depth buffer
    /// is, so the capture sees the same depth format (and therefore the same precision) as a
    /// presented frame.
    depth: wgpu::Texture,
}

impl CaptureTarget {
    pub fn new(device: &wgpu::Device, width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture.color"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture.depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::renderer::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Self {
            width,
            height,
            format,
            color,
            depth,
        }
    }

    pub fn color_view(&self) -> wgpu::TextureView {
        self.color.create_view(&wgpu::TextureViewDescriptor::default())
    }

    pub fn depth_view(&self) -> wgpu::TextureView {
        self.depth.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Copy the colour attachment into a mapped buffer and return it as tightly packed RGBA.
    ///
    /// `encoder` is the caller's: the frame's own submit has to happen before the copy is submitted,
    /// and wgpu requires both to come from the same queue order. Passing the encoder through keeps
    /// that ordering explicit instead of hiding a second submit inside this function.
    pub fn read_back(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
    ) -> Result<RgbaImage> {
        let bytes_per_row = padded_bytes_per_row(self.width);
        let size = (bytes_per_row * self.height) as u64;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture.readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));

        let slice = staging.slice(0..size);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .context("device lost while waiting for a capture readback")?;
        receiver
            .recv()
            .context("the capture readback map callback never ran")?
            .context("cannot map the capture readback buffer")?;

        let view = slice
            .get_mapped_range()
            .map_err(|e| anyhow::anyhow!("cannot read the capture readback buffer: {e}"))?;
        let mut rgba = unpad_rows(&view[..size as usize], self.width, self.height);
        drop(view);
        staging.unmap();
        if is_bgra(self.format) {
            swap_bgra_to_rgba(&mut rgba);
        }

        RgbaImage::from_raw(self.width, self.height, rgba)
            .with_context(|| format!("bad readback size for {}x{}", self.width, self.height))
    }
}

/// `saveSwapchainImage`'s last step: "PNG is RGBA and must be opaque so that the alpha channel never
/// influences comparisons". The nanite passes write `alpha = 1.0` anyway, but the C++ forces it and
/// the goldens were produced that way, so the Rust capture does the same.
pub fn write_png(path: &std::path::Path, image: &RgbaImage) -> Result<()> {
    let mut pixels = image.clone();
    for pixel in pixels.pixels_mut() {
        pixel[3] = 255;
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
    }
    pixels
        .save(path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    tracing::info!("[capture] wrote {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readback_rows_are_padded_to_the_wgpu_alignment() {
        assert_eq!(BYTES_PER_ROW_ALIGNMENT, 256);
        assert_eq!(padded_bytes_per_row(64), 256);
        assert_eq!(padded_bytes_per_row(63), 256);
        assert_eq!(padded_bytes_per_row(1), 256);
        assert_eq!(padded_bytes_per_row(1280), 5120);
        assert_eq!(padded_bytes_per_row(1281), 5376);
        assert_eq!(padded_bytes_per_row(1280) % BYTES_PER_ROW_ALIGNMENT, 0);
    }

    #[test]
    fn unpadding_recovers_a_tightly_packed_image() {
        let (w, h) = (5u32, 3u32);
        let stride = padded_bytes_per_row(w) as usize;
        let mut padded = vec![0u8; stride * h as usize];
        // Every pixel gets a unique value so a wrong offset is visible.
        for y in 0..h as usize {
            for x in 0..w as usize {
                let base = y * stride + x * 4;
                padded[base] = (x * 10 + y) as u8;
                padded[base + 1] = (y * 7) as u8;
                padded[base + 2] = (x * 3) as u8;
                padded[base + 3] = 0; // padding must never be read
            }
        }
        let tight = unpad_rows(&padded, w, h);
        assert_eq!(tight.len(), w as usize * h as usize * 4);
        for y in 0..h as usize {
            for x in 0..w as usize {
                let base = (y * w as usize + x) * 4;
                assert_eq!(tight[base], (x * 10 + y) as u8, "pixel {x},{y}");
                assert_eq!(tight[base + 1], (y * 7) as u8, "pixel {x},{y}");
                assert_eq!(tight[base + 2], (x * 3) as u8, "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn only_bgra_formats_need_a_channel_swap() {
        assert!(!is_bgra(wgpu::TextureFormat::Rgba8Unorm));
        assert!(!is_bgra(wgpu::TextureFormat::Rgba8UnormSrgb));
        assert!(is_bgra(wgpu::TextureFormat::Bgra8Unorm));
        assert!(is_bgra(wgpu::TextureFormat::Bgra8UnormSrgb));
    }

    #[test]
    fn the_channel_swap_is_its_own_inverse_and_leaves_alpha_alone() {
        let mut bytes = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        swap_bgra_to_rgba(&mut bytes);
        assert_eq!(bytes, vec![3, 2, 1, 4, 7, 6, 5, 8]);
        swap_bgra_to_rgba(&mut bytes);
        assert_eq!(bytes, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn a_capture_target_format_is_copy_src_able() {
        // `guaranteed_format_features` gives Rgba8Unorm COPY_SRC + RENDER_ATTACHMENT on every
        // backend; anything else would need a format feature check.
        let features = wgpu::Features::empty();
        let info = CAPTURE_FORMAT.guaranteed_format_features(features);
        assert!(
            info.allowed_usages.contains(wgpu::TextureUsages::COPY_SRC)
                && info
                    .allowed_usages
                    .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
            "{CAPTURE_FORMAT:?} cannot be read back and rendered to: {info:?}"
        );
        // And the depth attachment must be attachable, since the main pass needs one.
        let depth = crate::renderer::DEPTH_FORMAT.guaranteed_format_features(features);
        assert!(
            depth
                .allowed_usages
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
            "{:?} cannot be a depth attachment: {depth:?}",
            crate::renderer::DEPTH_FORMAT
        );
    }
}
