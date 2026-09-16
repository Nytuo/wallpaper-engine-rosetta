use std::io::{Cursor, Read};

const FREE_IMAGE_FORMAT_PNG: i32 = 13;
const FREE_IMAGE_FORMAT_RAW: i32 = -1;

mod tex_format {
    pub const RGBA8888: i32 = 0;
    pub const DXT5: i32 = 4;
    pub const DXT3: i32 = 6;
    pub const DXT1: i32 = 7;
    pub const RG88: i32 = 8;
    pub const R8: i32 = 9;
}

#[derive(Debug, thiserror::Error)]
pub enum TexError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a Wallpaper Engine texture (bad magic: {0:?})")]
    BadMagic(String),
    #[error("unsupported .tex container ({0}) — only TEXB0003/PNG/R8/RG88/RGBA8888 is decoded")]
    Unsupported(String),
    #[error("LZ4 decompression failed: {0}")]
    Lz4(#[from] lz4_flex::block::DecompressError),
    #[error("raw pixel data size mismatch: expected {expected} bytes for {width}x{height} at this format, got {got}")]
    SizeMismatch {
        expected: usize,
        got: usize,
        width: u32,
        height: u32,
    },
    #[error("PNG re-encoding failed: {0}")]
    PngEncode(#[from] image::ImageError),
}

pub fn extract_primary_png(data: &[u8]) -> Result<Vec<u8>, TexError> {
    let mut cursor = Cursor::new(data);

    let magic1 = read_cstring(&mut cursor)?;
    if magic1 != "TEXV0005" {
        return Err(TexError::BadMagic(magic1));
    }
    let magic2 = read_cstring(&mut cursor)?;
    if magic2 != "TEXI0001" {
        return Err(TexError::BadMagic(magic2));
    }

    let format = read_i32(&mut cursor)?;
    skip(&mut cursor, 24)?;

    let container_magic = read_cstring(&mut cursor)?;
    if container_magic != "TEXB0003" {
        return Err(TexError::Unsupported(container_magic));
    }

    let image_count = read_i32(&mut cursor)?;
    if image_count < 1 {
        return Err(TexError::Unsupported("no images in container".into()));
    }
    let image_format = read_i32(&mut cursor)?;

    let _unk1 = read_i32(&mut cursor)?;
    let width = read_i32(&mut cursor)?.max(0) as u32;
    let height = read_i32(&mut cursor)?.max(0) as u32;
    let is_lz4 = read_i32(&mut cursor)? == 1;
    let decompressed_len = read_i32(&mut cursor)?.max(0) as usize;
    let byte_count = read_i32(&mut cursor)?.max(0) as usize;
    let raw = read_bytes(&mut cursor, byte_count)?;

    let decompressed = if is_lz4 {
        lz4_flex::block::decompress(&raw, decompressed_len)?
    } else {
        raw
    };

    if image_format == FREE_IMAGE_FORMAT_PNG {
        return Ok(decompressed);
    }
    if image_format != FREE_IMAGE_FORMAT_RAW {
        return match image::load_from_memory(&decompressed) {
            Ok(decoded) => {
                let mut png = Vec::new();
                decoded.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)?;
                Ok(png)
            }
            Err(error) => Err(TexError::Unsupported(format!(
                "FreeImage format {image_format}, which is not PNG, not raw, and did not decode: {error}"
            ))),
        };
    }
    encode_raw_pixels_as_png(&decompressed, width, height, format)
}

fn encode_raw_pixels_as_png(
    pixels: &[u8],
    width: u32,
    height: u32,
    format: i32,
) -> Result<Vec<u8>, TexError> {
    let pixel_count = width as usize * height as usize;
    let rgba: Vec<u8> = match format {
        tex_format::R8 => {
            if pixels.len() != pixel_count {
                return Err(TexError::SizeMismatch { expected: pixel_count, got: pixels.len(), width, height });
            }




            pixels.iter().flat_map(|&v| [v, v, v, 255]).collect()
        }
        tex_format::RG88 => {
            if pixels.len() != pixel_count * 2 {
                return Err(TexError::SizeMismatch { expected: pixel_count * 2, got: pixels.len(), width, height });
            }
            pixels.chunks_exact(2).flat_map(|c| [c[0], c[1], 0, 255]).collect()
        }
        tex_format::RGBA8888 => {
            if pixels.len() != pixel_count * 4 {
                return Err(TexError::SizeMismatch { expected: pixel_count * 4, got: pixels.len(), width, height });
            }
            pixels.to_vec()
        }
        tex_format::DXT1 | tex_format::DXT3 | tex_format::DXT5 => {
            return Err(TexError::Unsupported(format!(
                "raw DXT-compressed texture (TexFormat {format}) — block decompression isn't implemented; no real \
                 content using it has been seen yet (every real mask found so far is R8 or RG88)"
            )))
        }
        other => return Err(TexError::Unsupported(format!("raw pixel TexFormat {other}"))),
    };

    let image_buffer =
        image::RgbaImage::from_raw(width, height, rgba).ok_or_else(|| TexError::SizeMismatch {
            expected: pixel_count * 4,
            got: 0,
            width,
            height,
        })?;
    let mut png_bytes = Vec::new();
    image_buffer.write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)?;
    Ok(png_bytes)
}

fn skip(cursor: &mut Cursor<&[u8]>, n: u64) -> Result<(), std::io::Error> {
    cursor.set_position(cursor.position() + n);
    Ok(())
}

fn read_i32(cursor: &mut Cursor<&[u8]>) -> Result<i32, std::io::Error> {
    let mut buf = [0u8; 4];
    cursor.read_exact(&mut buf)?;
    Ok(i32::from_le_bytes(buf))
}

fn read_bytes(cursor: &mut Cursor<&[u8]>, len: usize) -> Result<Vec<u8>, std::io::Error> {
    let mut buf = vec![0u8; len];
    cursor.read_exact(&mut buf)?;
    Ok(buf)
}

fn read_cstring(cursor: &mut Cursor<&[u8]>) -> Result<String, std::io::Error> {
    let mut bytes = Vec::new();
    loop {
        let mut b = [0u8; 1];
        cursor.read_exact(&mut b)?;
        if b[0] == 0 {
            break;
        }
        bytes.push(b[0]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_magic() {
        let err = extract_primary_png(b"NOTATEX\0").unwrap_err();
        assert!(matches!(err, TexError::BadMagic(_)));
    }

    #[test]
    fn decodes_a_synthetic_r8_mask_to_a_real_rgba_png() {
        let pixels: [u8; 4] = [0, 64, 128, 255];
        let png = encode_raw_pixels_as_png(&pixels, 2, 2, tex_format::R8).unwrap();
        let decoded = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(decoded.get_pixel(1, 1).0, [255, 255, 255, 255]);
    }

    #[test]
    fn decodes_a_synthetic_rg88_mask_to_a_real_rgba_png() {
        let pixels: [u8; 8] = [10, 20, 30, 40, 50, 60, 70, 80];
        let png = encode_raw_pixels_as_png(&pixels, 2, 2, tex_format::RG88).unwrap();
        let decoded = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [10, 20, 0, 255]);
        assert_eq!(decoded.get_pixel(1, 0).0, [30, 40, 0, 255]);
    }

    fn container(image_format: i32, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"TEXV0005\0");
        out.extend_from_slice(b"TEXI0001\0");
        out.extend_from_slice(&(-1i32).to_le_bytes());
        out.extend_from_slice(&[0u8; 24]);
        out.extend_from_slice(b"TEXB0003\0");
        out.extend_from_slice(&1i32.to_le_bytes());
        out.extend_from_slice(&image_format.to_le_bytes());
        out.extend_from_slice(&7i32.to_le_bytes());
        out.extend_from_slice(&2i32.to_le_bytes());
        out.extend_from_slice(&2i32.to_le_bytes());
        out.extend_from_slice(&0i32.to_le_bytes());
        out.extend_from_slice(&0i32.to_le_bytes());
        out.extend_from_slice(&(payload.len() as i32).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn decodes_an_image_carried_in_a_format_other_than_png() {
        let mut jpeg = Vec::new();
        image::RgbImage::from_pixel(2, 2, image::Rgb([200, 30, 40]))
            .write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .unwrap();
        let png = extract_primary_png(&container(2, &jpeg)).unwrap();
        assert_eq!(&png[..4], b"\x89PNG", "everything downstream is handed PNG");
        assert_eq!(
            image::load_from_memory(&png)
                .unwrap()
                .into_rgba8()
                .dimensions(),
            (2, 2)
        );
    }

    #[test]
    fn a_png_container_is_still_passed_through_untouched() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(extract_primary_png(&container(13, &png)).unwrap(), png);
    }

    #[test]
    fn a_payload_that_is_not_an_image_is_reported_rather_than_guessed_at() {
        let err = extract_primary_png(&container(5, b"not an image at all")).unwrap_err();
        assert!(matches!(err, TexError::Unsupported(_)));
    }

    #[test]
    fn rejects_a_raw_size_mismatch_rather_than_reading_out_of_bounds() {
        let err = encode_raw_pixels_as_png(&[1, 2, 3], 2, 2, tex_format::R8).unwrap_err();
        assert!(matches!(err, TexError::SizeMismatch { .. }));
    }
}
