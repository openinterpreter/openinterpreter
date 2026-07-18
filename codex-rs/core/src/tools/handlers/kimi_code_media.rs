use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ImageDetail;
use codex_utils_image::data_url_from_bytes;
use image::DynamicImage;
use image::GenericImageView;
use image::ImageEncoder;
use image::ImageFormat;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use serde::Deserialize;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::parse_arguments;

const MAX_MEDIA_BYTES: usize = 100 * 1024 * 1024;
const MAX_IMAGE_DECODE_BYTES: usize = 64 * 1024 * 1024;
const FULL_IMAGE_BYTE_BUDGET: usize = 15 * 1024 * 1024 / 4;
const READ_IMAGE_BYTE_BUDGET: usize = 256 * 1024;
const MAX_IMAGE_EDGE: u32 = 2_000;
const FALLBACK_EDGES: [u32; 5] = [1_000, 768, 512, 384, 256];
const JPEG_QUALITY_STEPS: [u8; 4] = [80, 60, 40, 20];

#[derive(Deserialize)]
struct ReadMediaArgs {
    path: String,
    #[serde(default)]
    region: Option<ImageRegion>,
    #[serde(default)]
    full_resolution: bool,
}

#[derive(Clone, Copy, Deserialize)]
struct ImageRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

struct PreparedImage {
    bytes: Vec<u8>,
    mime_type: &'static str,
    width: u32,
    height: u32,
    delivery: ImageDelivery,
}

#[derive(Clone, Copy)]
enum ImageDelivery {
    Untouched,
    Downsampled,
    Full,
    Crop { region: ImageRegion, resized: bool },
}

pub(super) async fn handle(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let ToolPayload::Function { arguments } = &invocation.payload else {
        return Err(FunctionCallError::RespondToModel(
            "ReadMediaFile received unsupported tool payload".to_string(),
        ));
    };
    let args: ReadMediaArgs = parse_arguments(arguments)?;
    if args.path.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "File path cannot be empty.".to_string(),
        ));
    }
    let path = super::harness_fs::checked_read_path(&invocation, &args.path, "ReadMediaFile")?;
    let data = std::fs::read(&path).map_err(|error| {
        FunctionCallError::RespondToModel(format!("Failed to read {}: {error}", args.path))
    })?;
    let original_size = data.len();
    if data.is_empty() {
        return Err(FunctionCallError::RespondToModel(format!(
            "\"{}\" is empty.",
            args.path
        )));
    }
    if data.len() > MAX_MEDIA_BYTES {
        return Err(FunctionCallError::RespondToModel(format!(
            "\"{}\" is {} bytes, which exceeds the maximum 100MB for media files.",
            args.path,
            data.len()
        )));
    }

    if let Some(mime_type) = super::kimi_code_video::mime_type(&path, &data) {
        let read_mode = if args.region.is_some() || args.full_resolution {
            super::kimi_code_video::ReadMode::ImageOptionsRequested
        } else {
            super::kimi_code_video::ReadMode::Default
        };
        return super::kimi_code_video::handle(&invocation, &path, &data, mime_type, read_mode)
            .await;
    }

    let format = image::guess_format(&data).map_err(|_| {
        FunctionCallError::RespondToModel(format!(
            "\"{}\" is not a supported image or video file. Use Read for text files, or Bash or an MCP tool for other binary formats.",
            args.path
        ))
    })?;
    let mime_type = supported_mime_type(format).ok_or_else(|| {
        FunctionCallError::RespondToModel(format!(
            "\"{}\" is not a provider-supported PNG, JPEG, GIF, or WebP image. Convert it with Bash, then call ReadMediaFile on the converted file.",
            args.path
        ))
    })?;
    let (original_width, original_height, prepared) = if matches!(format, ImageFormat::Gif) {
        let (width, height) = gif_dimensions(&data).ok_or_else(|| {
            FunctionCallError::RespondToModel(format!(
                "Failed to read {}: failed to decode GIF dimensions",
                args.path
            ))
        })?;
        let prepared = prepare_gif(&args, data, width, height)?;
        (width, height, prepared)
    } else {
        let decoded = image::load_from_memory_with_format(&data, format).map_err(|error| {
            let message = if args.region.is_some()
                && matches!(format, ImageFormat::Png)
                && error.to_string().contains("CRC error")
            {
                format!(
                    "Cannot read region from \"{}\": Failed to decode the image for cropping: unrecognised content at end of stream",
                    args.path
                )
            } else {
                format!("Failed to read {}: failed to decode image: {error}", args.path)
            };
            FunctionCallError::RespondToModel(message)
        })?;
        let (width, height) = decoded.dimensions();
        let prepared = prepare_image(&args, data, decoded, format, mime_type)?;
        (width, height, prepared)
    };
    let absolute_path = path.to_string_lossy();
    let note = media_note(
        mime_type,
        original_size,
        original_width,
        original_height,
        &prepared,
    );
    let image_url = data_url_from_bytes(prepared.mime_type, &prepared.bytes);

    Ok(boxed_tool_output(FunctionToolOutput::from_content(
        vec![
            FunctionCallOutputContentItem::InputText {
                text: format!("<image path=\"{absolute_path}\">"),
            },
            FunctionCallOutputContentItem::InputImage {
                image_url,
                detail: Some(ImageDetail::Original),
            },
            FunctionCallOutputContentItem::InputText {
                text: "</image>".to_string(),
            },
            FunctionCallOutputContentItem::InputText { text: note },
        ],
        /*success*/ Some(true),
    )))
}

fn prepare_gif(
    args: &ReadMediaArgs,
    data: Vec<u8>,
    width: u32,
    height: u32,
) -> Result<PreparedImage, FunctionCallError> {
    if args.region.is_some() {
        return Err(FunctionCallError::RespondToModel(format!(
            "Cannot read region from \"{}\": Cropping is only supported for PNG, JPEG, and WebP images; got image/gif.",
            args.path
        )));
    }
    if args.full_resolution {
        if data.len() > FULL_IMAGE_BYTE_BUDGET {
            return Err(FunctionCallError::RespondToModel(format!(
                "\"{}\" is {} bytes, over the {}-byte per-image limit, so full_resolution cannot be honored. Use region to view a crop at full fidelity instead.",
                args.path,
                data.len(),
                FULL_IMAGE_BYTE_BUDGET
            )));
        }
        return Ok(PreparedImage {
            bytes: data,
            mime_type: "image/gif",
            width,
            height,
            delivery: ImageDelivery::Full,
        });
    }
    if data.len() > READ_IMAGE_BYTE_BUDGET || width > MAX_IMAGE_EDGE || height > MAX_IMAGE_EDGE {
        return Err(FunctionCallError::RespondToModel(format!(
            "Image is too large to send safely after compression ({} bytes; limit {READ_IMAGE_BYTE_BUDGET} bytes and {MAX_IMAGE_EDGE}px on the longest edge). The original image was not sent to the model. Do not retry the same file unchanged.",
            data.len()
        )));
    }
    Ok(PreparedImage {
        bytes: data,
        mime_type: "image/gif",
        width,
        height,
        delivery: ImageDelivery::Untouched,
    })
}

fn gif_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let header = data.get(..10)?;
    let signature = &header[..6];
    if signature != b"GIF87a" && signature != b"GIF89a" {
        return None;
    }
    let width = u16::from_le_bytes([header[6], header[7]]).into();
    let height = u16::from_le_bytes([header[8], header[9]]).into();
    Some((width, height))
}

fn prepare_image(
    args: &ReadMediaArgs,
    data: Vec<u8>,
    decoded: DynamicImage,
    format: ImageFormat,
    mime_type: &'static str,
) -> Result<PreparedImage, FunctionCallError> {
    let (original_width, original_height) = decoded.dimensions();
    if let Some(requested) = args.region {
        if data.len() > MAX_IMAGE_DECODE_BYTES {
            return Err(FunctionCallError::RespondToModel(format!(
                "Image is too large to process safely for region or full_resolution ({} bytes; safe decode limit {} bytes). The original image was not sent to the model.",
                data.len(),
                MAX_IMAGE_DECODE_BYTES
            )));
        }
        if requested.width == 0
            || requested.height == 0
            || requested.x >= original_width
            || requested.y >= original_height
        {
            return Err(FunctionCallError::RespondToModel(format!(
                "Cannot read region from \"{}\": Region (x={}, y={}, width={}, height={}) lies outside the {}x{} image.",
                args.path,
                requested.x,
                requested.y,
                requested.width,
                requested.height,
                original_width,
                original_height
            )));
        }
        let region = ImageRegion {
            x: requested.x,
            y: requested.y,
            width: requested.width.min(original_width - requested.x),
            height: requested.height.min(original_height - requested.y),
        };
        let crop = decoded.crop_imm(region.x, region.y, region.width, region.height);
        let (crop, resized) = if args.full_resolution
            || crop.width() <= MAX_IMAGE_EDGE && crop.height() <= MAX_IMAGE_EDGE
        {
            (crop, false)
        } else {
            (
                crop.resize(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE, FilterType::Triangle),
                true,
            )
        };
        let (bytes, output_mime) = encode_native(&crop, format)?;
        if bytes.len() > FULL_IMAGE_BYTE_BUDGET {
            return Err(FunctionCallError::RespondToModel(format!(
                "Cannot read region from \"{}\": The cropped region encodes to {} bytes, over the {}-byte per-image limit. Choose a smaller region, or allow downscaling.",
                args.path,
                bytes.len(),
                FULL_IMAGE_BYTE_BUDGET
            )));
        }
        return Ok(PreparedImage {
            width: crop.width(),
            height: crop.height(),
            bytes,
            mime_type: output_mime,
            delivery: ImageDelivery::Crop { region, resized },
        });
    }

    if args.full_resolution {
        if data.len() > FULL_IMAGE_BYTE_BUDGET {
            return Err(FunctionCallError::RespondToModel(format!(
                "\"{}\" is {} bytes, over the {}-byte per-image limit, so full_resolution cannot be honored. Use region to view a crop at full fidelity instead.",
                args.path,
                data.len(),
                FULL_IMAGE_BYTE_BUDGET
            )));
        }
        return Ok(PreparedImage {
            bytes: data,
            mime_type,
            width: original_width,
            height: original_height,
            delivery: ImageDelivery::Full,
        });
    }

    if data.len() <= READ_IMAGE_BYTE_BUDGET
        && original_width <= MAX_IMAGE_EDGE
        && original_height <= MAX_IMAGE_EDGE
    {
        return Ok(PreparedImage {
            bytes: data,
            mime_type,
            width: original_width,
            height: original_height,
            delivery: ImageDelivery::Untouched,
        });
    }

    let resized = if original_width > MAX_IMAGE_EDGE || original_height > MAX_IMAGE_EDGE {
        decoded.resize(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE, FilterType::Triangle)
    } else {
        decoded
    };
    let (bytes, output_mime, width, height) = encode_with_read_budget(resized, format)?;
    Ok(PreparedImage {
        bytes,
        mime_type: output_mime,
        width,
        height,
        delivery: ImageDelivery::Downsampled,
    })
}

fn encode_with_read_budget(
    image: DynamicImage,
    source_format: ImageFormat,
) -> Result<(Vec<u8>, &'static str, u32, u32), FunctionCallError> {
    let mut candidate = image;
    let mut fallback_edges = FALLBACK_EDGES.into_iter();
    loop {
        if !matches!(source_format, ImageFormat::Jpeg)
            && (candidate.width() >= 1_000 || candidate.height() >= 1_000)
        {
            let png = encode_png(&candidate)?;
            if png.len() <= READ_IMAGE_BYTE_BUDGET {
                return Ok((png, "image/png", candidate.width(), candidate.height()));
            }
        }
        for quality in JPEG_QUALITY_STEPS {
            let jpeg = encode_jpeg(&candidate, quality)?;
            if jpeg.len() <= READ_IMAGE_BYTE_BUDGET {
                return Ok((jpeg, "image/jpeg", candidate.width(), candidate.height()));
            }
        }
        let Some(edge) = fallback_edges.next() else {
            break;
        };
        if candidate.width() > edge || candidate.height() > edge {
            candidate = candidate.resize(edge, edge, FilterType::Triangle);
        }
    }
    Err(FunctionCallError::RespondToModel(format!(
        "Image is too large to send safely after compression (limit {READ_IMAGE_BYTE_BUDGET} bytes and {MAX_IMAGE_EDGE}px on the longest edge). The original image was not sent to the model. Do not retry the same file unchanged."
    )))
}

fn encode_native(
    image: &DynamicImage,
    source_format: ImageFormat,
) -> Result<(Vec<u8>, &'static str), FunctionCallError> {
    if matches!(source_format, ImageFormat::Jpeg) {
        Ok((encode_jpeg(image, /*quality*/ 90)?, "image/jpeg"))
    } else {
        Ok((encode_png(image)?, "image/png"))
    }
}

fn encode_png(image: &DynamicImage) -> Result<Vec<u8>, FunctionCallError> {
    let rgba = image.to_rgba8();
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(
            rgba.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!("Failed to encode image: {error}"))
        })?;
    Ok(bytes)
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Result<Vec<u8>, FunctionCallError> {
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, quality)
        .encode_image(image)
        .map_err(|error| {
            FunctionCallError::RespondToModel(format!("Failed to encode image: {error}"))
        })?;
    Ok(bytes)
}

fn supported_mime_type(format: ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Png => Some("image/png"),
        ImageFormat::Jpeg => Some("image/jpeg"),
        ImageFormat::Gif => Some("image/gif"),
        ImageFormat::WebP => Some("image/webp"),
        _ => None,
    }
}

fn media_note(
    original_mime: &str,
    original_size: usize,
    original_width: u32,
    original_height: u32,
    prepared: &PreparedImage,
) -> String {
    let mut parts = vec![
        "Read image file.".to_string(),
        format!("Mime type: {original_mime}."),
        format!("Size: {original_size} bytes."),
        format!("Original dimensions: {original_width}x{original_height} pixels."),
    ];
    match prepared.delivery {
        ImageDelivery::Untouched => {}
        ImageDelivery::Downsampled => parts.push(format!(
            "The attached image was downsampled to {}x{} pixels ({}, {} bytes) to fit model limits; fine detail may be lost. To inspect fine detail, call ReadMediaFile again with the region parameter (original-image pixel coordinates) to view a crop at full fidelity.",
            prepared.width, prepared.height, prepared.mime_type, prepared.bytes.len()
        )),
        ImageDelivery::Full => {
            parts.push("Shown at native resolution; no downscaling applied.".to_string());
        }
        ImageDelivery::Crop { region, resized } => {
            let resolution = if resized {
                format!(
                    ", downsampled to {}x{} pixels",
                    prepared.width, prepared.height
                )
            } else {
                " at native resolution".to_string()
            };
            parts.push(format!(
                "Showing region (x={}, y={}, width={}, height={}) of the original image{resolution}. To output coordinates in original-image pixels, locate them within this crop and add the region offset (x={}, y={}).",
                region.x, region.y, region.width, region.height, region.x, region.y
            ));
        }
    }
    if !matches!(prepared.delivery, ImageDelivery::Crop { .. }) {
        parts.push(
            "If you need to output coordinates, output relative coordinates first and compute absolute coordinates using the original image size.".to_string(),
        );
    }
    parts.push(
        "If you generate or edit images or videos via commands or scripts, read the result back immediately before continuing.".to_string(),
    );
    format!("<system>{}</system>", parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageBuffer;
    use image::Rgb;

    #[test]
    fn byte_budget_compression_never_upscales_an_image() {
        let width = 800;
        let height = 400;
        let image = DynamicImage::ImageRgb8(ImageBuffer::from_fn(width, height, |x, y| {
            Rgb([
                x.wrapping_mul(73).wrapping_add(y.wrapping_mul(151)) as u8,
                x.wrapping_mul(197).wrapping_add(y.wrapping_mul(31)) as u8,
                x.wrapping_mul(17).wrapping_add(y.wrapping_mul(229)) as u8,
            ])
        }));
        let data = encode_png(&image).expect("encode oversized PNG fixture");
        assert!(data.len() > READ_IMAGE_BYTE_BUDGET);

        let prepared = prepare_image(
            &ReadMediaArgs {
                path: "fixture.png".to_string(),
                region: None,
                full_resolution: false,
            },
            data,
            image,
            ImageFormat::Png,
            "image/png",
        )
        .expect("compress image within read budget");

        assert_eq!((prepared.width, prepared.height), (width, height));
        assert!(prepared.bytes.len() <= READ_IMAGE_BYTE_BUDGET);
    }
}
