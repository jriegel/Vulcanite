//! Minimal KTX 1 reader for the environment cube map.
//!
//! The only environment asset in this repository is `assets/textures/hdr/gcanyon_cube.ktx`, and the
//! C++ viewer loads it through `vks::TextureCubeMap::loadFromFile` as
//! `VK_FORMAT_R16G16B16A16_SFLOAT` (`pbrtexture.cpp:1346`). This reader covers exactly what that
//! file needs - KTX 1, uncompressed, cube map, little endian - and rejects everything else rather
//! than guessing.
//!
//! KTX 1 layout, verified against the file itself (67,108,956 bytes):
//!
//! ```text
//! 0   identifier  «KTX 11»\r\n\x1a\n        (12 B)
//! 12  endianness  0x04030201                (LE; 0x01020304 would mean byte-swapped)
//! 16  glTypeSize 2, glFormat 6408 (GL_RGBA), glInternalFormat 34842 (GL_RGBA16F),
//!     glBaseInternalFormat 6408, pixelWidth 1024, pixelHeight 1024, pixelDepth 0,
//!     numberOfArrayElements 0, numberOfFaces 6, numberOfMipmapLevels 11, kvBytes 0
//! 64  per mip level: u32 imageSize, then `imageSize * numberOfFaces` bytes,
//!     then padding so the *level block* is a multiple of 4
//! ```
//!
//! The trap is `imageSize`: for a cube map it is the size of **one face**, not of the whole level.
//! Reading it as a whole level walks off the end of the file after mip 0 (measured: mip 1's header
//! lands on `0x22222222`-ish garbage and the computed offset overruns the file). The arithmetic
//! below is checked against the real file by `decodes_the_environment_cube`.

use anyhow::{bail, ensure, Result};
use std::path::Path;

/// `«KTX 11»\r\n\x1a\n`
const IDENTIFIER: [u8; 12] = [0xAB, 0x4B, 0x54, 0x58, 0x20, 0x31, 0x31, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A];
const ENDIAN_LITTLE: u32 = 0x0403_0201;
const ENDIAN_BIG: u32 = 0x0102_0304;

/// GL `GLenum`s needed to recognise the formats in this repository. Only the RGBA family is
/// supported: the environment cube is `RGBA16F`, and the skybox generators write `RGBA16F` /
/// `RGBA32F` targets. A single-channel or RGB-packed KTX would need its own row math, so it is
/// rejected instead of silently mis-uploaded.
mod gl {
    pub const UNSIGNED_BYTE: u32 = 0x1401;
    pub const HALF_FLOAT: u32 = 0x140B;
    pub const FLOAT: u32 = 0x1406;
    pub const RGBA: u32 = 0x1908;
    pub const RGBA8: u32 = 0x8058;
    pub const RGBA16F: u32 = 0x881A;
    pub const RGBA32F: u32 = 0x8814;
}

/// One decoded mip level: six faces, tightly packed, no row padding.
#[derive(Clone, Debug, PartialEq)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    /// `imageSize` from the file - one face, in bytes.
    pub face_bytes: usize,
    /// `face_bytes * 6` bytes, faces in GL/Vulkan cube order
    /// `+X, -X, +Y, -Y, +Z, -Z`.
    pub faces: Vec<u8>,
}

/// A decoded cube map, ready to be uploaded with `Queue::write_texture`.
#[derive(Clone, Debug, PartialEq)]
pub struct CubeMap {
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
    pub levels: Vec<MipLevel>,
}

impl CubeMap {
    pub fn mip_levels(&self) -> u32 {
        self.levels.len() as u32
    }
}

/// Read a KTX 1 cube map from disk.
pub fn read_cube_map(path: &Path) -> Result<CubeMap> {
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("ktx: cannot open {}: {e}", path.display()))?;
    read_cube_map_bytes(&bytes).map_err(|e| anyhow::anyhow!("{}: {e:#}", path.display()))
}

/// The decode itself, split out so the real asset can be checked in a unit test.
pub fn read_cube_map_bytes(b: &[u8]) -> Result<CubeMap> {
    ensure!(b.len() >= 64, "ktx: file too small ({} bytes)", b.len());
    ensure!(
        b[0..12] == IDENTIFIER,
        "ktx: not a KTX 1 file (identifier {:02x?})",
        &b[0..12]
    );

    // Only little endian is supported. A big-endian file is rare enough that swapping the whole
    // decoder is not worth it; failing loudly beats reading the header mirrored.
    match u32::from_le_bytes(b[12..16].try_into().unwrap()) {
        ENDIAN_LITTLE => {}
        ENDIAN_BIG => bail!("ktx: big-endian files are not supported"),
        other => bail!("ktx: bad endianness marker {other:#010x}"),
    }

    let gl_type = u32::from_le_bytes(b[16..20].try_into().unwrap());
    let gl_type_size = u32::from_le_bytes(b[20..24].try_into().unwrap());
    let gl_format = u32::from_le_bytes(b[24..28].try_into().unwrap());
    let gl_internal_format = u32::from_le_bytes(b[28..32].try_into().unwrap());
    let _gl_base_internal_format = u32::from_le_bytes(b[32..36].try_into().unwrap());
    let width = u32::from_le_bytes(b[36..40].try_into().unwrap());
    let height = u32::from_le_bytes(b[40..44].try_into().unwrap());
    let depth = u32::from_le_bytes(b[44..48].try_into().unwrap());
    let array_elements = u32::from_le_bytes(b[48..52].try_into().unwrap());
    let faces = u32::from_le_bytes(b[52..56].try_into().unwrap());
    let mip_levels = u32::from_le_bytes(b[56..60].try_into().unwrap());
    let kv_bytes = u32::from_le_bytes(b[60..64].try_into().unwrap());

    ensure!(depth == 0, "ktx: 3D textures are not supported (depth {depth})");
    ensure!(
        array_elements == 0,
        "ktx: array textures are not supported (numberOfArrayElements {array_elements})"
    );
    ensure!(
        faces == 6,
        "ktx: not a cube map (numberOfFaces {faces}) - this viewer only takes cube maps"
    );
    ensure!(
        gl_type != 0,
        "ktx: compressed (glType=0) KTX is not supported"
    );
    ensure!(width > 0 && height > 0, "ktx: zero-sized image {width}x{height}");
    ensure!(mip_levels > 0, "ktx: no mip levels");

    let (format, bytes_per_texel) = map_format(gl_internal_format, gl_type, gl_format, gl_type_size)?;

    let mut offset = 64usize + kv_bytes as usize;
    let mut levels = Vec::with_capacity(mip_levels as usize);
    for level in 0..mip_levels {
        ensure!(
            offset + 4 <= b.len(),
            "ktx: mip {level} header runs past the end of the file (offset {offset})"
        );
        let image_size = u32::from_le_bytes(b[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;

        // `imageSize` is per face for a cube map, so the level block is six times it.
        let block_bytes = image_size
            .checked_mul(6)
            .ok_or_else(|| anyhow::anyhow!("ktx: mip {level} imageSize {image_size} overflows"))?;
        ensure!(
            offset + block_bytes <= b.len(),
            "ktx: mip {level} block ({block_bytes} B at {offset}) runs past the end of the \
             file ({} B)",
            b.len()
        );

        let expect_w = (width >> level).max(1);
        let expect_h = (height >> level).max(1);
        ensure!(
            image_size == expect_w as usize * expect_h as usize * bytes_per_texel,
            "ktx: mip {level} imageSize {image_size} does not match {expect_w}x{expect_h} x \
             {bytes_per_texel} B/texel"
        );

        levels.push(MipLevel {
            width: expect_w,
            height: expect_h,
            face_bytes: image_size,
            faces: b[offset..offset + block_bytes].to_vec(),
        });

        offset += block_bytes;
        // Padding aligns the *level block*, not each face.
        offset += (4 - (block_bytes % 4)) % 4;
    }

    ensure!(
        offset == b.len(),
        "ktx: {} trailing bytes after the last mip level (consumed {offset} of {})",
        b.len() - offset,
        b.len()
    );

    Ok(CubeMap {
        width,
        height,
        format,
        levels,
    })
}

/// `glInternalFormat` -> wgpu format plus the expected bytes per texel.
///
/// The bytes-per-texel value is cross-checked against `glTypeSize * channels` by the caller's
/// `imageSize` check, so a mismatched header is caught per mip rather than as a short read.
fn map_format(
    internal: u32,
    gl_type: u32,
    gl_format: u32,
    gl_type_size: u32,
) -> Result<(wgpu::TextureFormat, usize)> {
    // The header's own type/format pair is validated too: `glFormat` says how many channels the
    // data really has, and reading an RGB file as RGBA would silently produce shifted pixels.
    let channels = match gl_format {
        gl::RGBA => 4,
        other => bail!(
            "ktx: glFormat {other:#x} is not GL_RGBA - only RGBA-packed cube maps are supported"
        ),
    };
    let declared = gl_type_size as usize * channels;
    let (format, size) = match (internal, gl_type) {
        (gl::RGBA16F, gl::HALF_FLOAT) => (wgpu::TextureFormat::Rgba16Float, 8),
        (gl::RGBA32F, gl::FLOAT) => (wgpu::TextureFormat::Rgba32Float, 16),
        (gl::RGBA8, gl::UNSIGNED_BYTE) => (wgpu::TextureFormat::Rgba8UnormSrgb, 4),
        (internal, gl_type) => bail!(
            "ktx: unsupported format glInternalFormat={internal:#x} glType={gl_type:#x} \
             glFormat={gl_format:#x}"
        ),
    };
    ensure!(
        declared == size,
        "ktx: header says {declared} B/texel (glTypeSize {gl_type_size} x {channels}) but \
         {internal:#x} needs {size}"
    );
    Ok((format, size))
}

/// `wgpu` requires `bytes_per_row` to be a multiple of `COPY_BYTES_PER_ROW_ALIGNMENT` (256).
/// KTX rows are tightly packed, so small mip levels need padding - at 16x16 x 8 B the row is 128 B,
/// below the alignment, which is exactly where an unpadded upload fails.
pub fn padded_bytes_per_row(width: u32, bytes_per_texel: usize) -> usize {
    const ALIGN: usize = 256;
    let raw = width as usize * bytes_per_texel;
    raw.div_ceil(ALIGN) * ALIGN
}

/// Copy one [`MipLevel`] into a 256-byte-row-aligned buffer holding all six faces, ready for a
/// single `write_texture` with `origin.z = 0` and `depth_or_array_layers = 6`.
pub fn pad_level_for_upload(level: &MipLevel, bytes_per_texel: usize) -> Vec<u8> {
    let src_stride = level.width as usize * bytes_per_texel;
    let dst_stride = padded_bytes_per_row(level.width, bytes_per_texel);
    let mut out = vec![0u8; dst_stride * level.height as usize * 6];
    for face in 0..6usize {
        for row in 0..level.height as usize {
            let src = face * level.face_bytes + row * src_stride;
            let dst = (face * level.height as usize + row) * dst_stride;
            out[dst..dst + src_stride].copy_from_slice(&level.faces[src..src + src_stride]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real asset from `assets/textures/hdr/`. Its exact header values are pinned here because
    /// every one of them (faces, mips, glType, internalFormat) decides the decode.
    fn asset_path() -> std::path::PathBuf {
        // CARGO_MANIFEST_DIR is `rust/nanite-viewer-rs`; the asset lives in the repository root.
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/textures/hdr/gcanyon_cube.ktx")
    }

    #[test]
    fn decodes_the_environment_cube() {
        let path = asset_path();
        if !path.exists() {
            eprintln!("skipping: {} missing", path.display());
            return;
        }
        let bytes = std::fs::read(&path).expect("read");
        assert_eq!(bytes.len(), 67_108_956, "the asset changed");
        let cube = read_cube_map_bytes(&bytes).expect("decode");
        assert_eq!(cube.width, 1024);
        assert_eq!(cube.height, 1024);
        assert_eq!(cube.format, wgpu::TextureFormat::Rgba16Float);
        assert_eq!(cube.mip_levels(), 11);
        assert_eq!(cube.levels[0].face_bytes, 1024 * 1024 * 8);
        assert_eq!(cube.levels[10].width, 1);
        assert_eq!(cube.levels[10].height, 1);
        assert_eq!(cube.levels[10].face_bytes, 8);
        for (i, level) in cube.levels.iter().enumerate() {
            assert_eq!(level.width, (1024 >> i).max(1), "mip {i} width");
            assert_eq!(level.faces.len(), level.face_bytes * 6, "mip {i} block");
        }
    }

    #[test]
    fn image_size_is_per_face_not_per_level() {
        // The failure mode this guards: treating mip 0's imageSize as the whole level makes mip 1's
        // length prefix land inside the pixel data. Asserting the second level decodes at all is
        // what catches it.
        let path = asset_path();
        if !path.exists() {
            return;
        }
        let bytes = std::fs::read(&path).expect("read");
        let image_size = u32::from_le_bytes(bytes[64..68].try_into().unwrap()) as usize;
        assert_eq!(image_size, 8_388_608);
        // Read the wrong way, the next "header" is pixel data, not a plausible length.
        let wrong = u32::from_le_bytes(bytes[64 + 4 + image_size..64 + 8 + image_size].try_into().unwrap());
        assert_ne!(wrong, 2_097_152, "the wrong layout would have looked right here");
        let right = u32::from_le_bytes(
            bytes[64 + 4 + image_size * 6..64 + 8 + image_size * 6]
                .try_into()
                .unwrap(),
        );
        assert_eq!(right, 2_097_152);
    }

    #[test]
    fn row_padding_matches_the_wgpu_alignment() {
        assert_eq!(padded_bytes_per_row(1024, 8), 8192);
        assert_eq!(padded_bytes_per_row(32, 8), 256);
        assert_eq!(padded_bytes_per_row(16, 8), 256);
        assert_eq!(padded_bytes_per_row(1, 8), 256);
        assert_eq!(padded_bytes_per_row(1024, 4), 4096);
    }

    #[test]
    fn padding_keeps_every_texel() {
        let level = MipLevel {
            width: 2,
            height: 2,
            face_bytes: 2 * 2 * 4,
            faces: (0..4 * 4 * 6u8).collect(),
        };
        let padded = pad_level_for_upload(&level, 4);
        assert_eq!(padded.len(), 256 * 2 * 6);
        // Face 3, row 1, texel 1 -> byte 4 of that row.
        let src = 3 * level.face_bytes + 1 * 8 + 4;
        let dst = (3 * 2 + 1) * 256 + 4;
        assert_eq!(padded[dst], level.faces[src]);
        // Everything past the row is zero fill.
        assert!(padded[(3 * 2 + 1) * 256 + 8..(3 * 2 + 2) * 256]
            .iter()
            .all(|&b| b == 0));
    }

    #[test]
    fn rejects_what_it_cannot_upload() {
        // Not a KTX file at all.
        assert!(read_cube_map_bytes(&[0u8; 128]).is_err());

        let mut header = Vec::new();
        header.extend_from_slice(&IDENTIFIER);
        header.extend_from_slice(&ENDIAN_LITTLE.to_le_bytes());
        // glType, glTypeSize, glFormat, glInternalFormat, glBaseInternalFormat,
        // width, height, depth, arrayElements, faces, mips, kvBytes.
        for v in [
            gl::HALF_FLOAT,
            2,
            gl::RGBA,
            gl::RGBA16F,
            gl::RGBA,
            4,
            4,
            0,
            0,
            6,
            1,
            0,
        ] {
            header.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(header.len(), 64);
        // One mip: 4x4 RGBA16F = 128 B per face, 768 B for the level.
        header.extend_from_slice(&128u32.to_le_bytes());
        header.extend_from_slice(&vec![7u8; 768]);
        let cube = read_cube_map_bytes(&header).expect("valid cube");
        assert_eq!(cube.mip_levels(), 1);
        assert_eq!(cube.levels[0].faces.len(), 768);

        // A cube map claiming 24 faces.
        let mut bad = header.clone();
        bad[52..56].copy_from_slice(&24u32.to_le_bytes());
        assert!(read_cube_map_bytes(&bad).is_err());

        // Truncated pixel data.
        assert!(read_cube_map_bytes(&header[..header.len() - 10]).is_err());

        // An unsupported internal format.
        let mut bad = header.clone();
        bad[28..32].copy_from_slice(&0x8C01u32.to_le_bytes()); // COMPRESSED_RGBA8_ETC2_EAC
        assert!(read_cube_map_bytes(&bad).is_err());

        // glTypeSize contradicting the internal format.
        let mut bad = header.clone();
        bad[20..24].copy_from_slice(&4u32.to_le_bytes());
        assert!(read_cube_map_bytes(&bad).is_err());
    }
}
