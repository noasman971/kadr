//! GPU memory: pictures uploaded as textures, the canvas, the YUV output planes
//! and the buffers they are read back through.
//!
//! Textures are kept from one frame to the next and only re-created when a
//! picture changes size or kind, so a steady export allocates nothing per frame.

use kadr_media::{VideoFrame, YuvPicture};

use super::pipelines::{CANVAS_FORMAT, OUTPUT_UNIFORM_SIZE_IN_BYTES, PLANE_FORMAT};

/// Converts floats to the bytes the GPU expects (native endianness).
pub(crate) fn floats_as_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

/// A texture and the view shaders read it through.
pub(crate) struct TextureWithView {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
}

pub(crate) fn create_texture(
    device: &wgpu::Device,
    label: &str,
    (width, height): (u32, u32),
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> TextureWithView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    TextureWithView { texture, view }
}

/// Copies tightly packed rows into a texture.
fn write_texture_rows(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    bytes: &[u8],
    (width, height): (u32, u32),
    bytes_per_pixel: u32,
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * bytes_per_pixel),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

/// The three planes of a YUV picture on the GPU: Y at full size, U and V at half size.
pub(crate) struct YuvTextures {
    pub(crate) size: (u32, u32),
    pub(crate) luma: TextureWithView,
    pub(crate) chroma_u: TextureWithView,
    pub(crate) chroma_v: TextureWithView,
}

/// A picture uploaded to the GPU.
pub(crate) enum PictureTextures {
    /// One RGBA texture.
    Rgba {
        size: (u32, u32),
        rgba: TextureWithView,
    },
    /// Three single-channel textures (boxed: three times larger than the RGBA case).
    Yuv(Box<YuvTextures>),
}

const SAMPLED_TEXTURE_USAGE: wgpu::TextureUsages =
    wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::COPY_DST);

impl PictureTextures {
    fn for_picture(device: &wgpu::Device, picture: &VideoFrame) -> Self {
        let size = (picture.width(), picture.height());
        match picture {
            VideoFrame::Rgba(_) => Self::Rgba {
                size,
                rgba: create_texture(
                    device,
                    "Kadr RGBA picture",
                    size,
                    wgpu::TextureFormat::Rgba8Unorm,
                    SAMPLED_TEXTURE_USAGE,
                ),
            },
            VideoFrame::Yuv420(_) => {
                let chroma_size = (
                    YuvPicture::chroma_width(size.0),
                    YuvPicture::chroma_height(size.1),
                );
                Self::Yuv(Box::new(YuvTextures {
                    size,
                    luma: create_texture(
                        device,
                        "Kadr Y plane",
                        size,
                        PLANE_FORMAT,
                        SAMPLED_TEXTURE_USAGE,
                    ),
                    chroma_u: create_texture(
                        device,
                        "Kadr U plane",
                        chroma_size,
                        PLANE_FORMAT,
                        SAMPLED_TEXTURE_USAGE,
                    ),
                    chroma_v: create_texture(
                        device,
                        "Kadr V plane",
                        chroma_size,
                        PLANE_FORMAT,
                        SAMPLED_TEXTURE_USAGE,
                    ),
                }))
            }
        }
    }

    /// Whether these textures can hold `picture` (same kind and size).
    fn fits(&self, picture: &VideoFrame) -> bool {
        let picture_size = (picture.width(), picture.height());
        match (self, picture) {
            (Self::Rgba { size, .. }, VideoFrame::Rgba(_)) => *size == picture_size,
            (Self::Yuv(textures), VideoFrame::Yuv420(_)) => textures.size == picture_size,
            _ => false,
        }
    }

    fn upload(&self, queue: &wgpu::Queue, picture: &VideoFrame) {
        match (self, picture) {
            (Self::Rgba { size, rgba }, VideoFrame::Rgba(image)) => {
                write_texture_rows(queue, &rgba.texture, image.as_raw(), *size, 4)
            }
            (Self::Yuv(textures), VideoFrame::Yuv420(yuv_picture)) => {
                let YuvTextures {
                    size,
                    luma,
                    chroma_u,
                    chroma_v,
                } = textures.as_ref();
                let chroma_size = (
                    YuvPicture::chroma_width(size.0),
                    YuvPicture::chroma_height(size.1),
                );
                write_texture_rows(queue, &luma.texture, yuv_picture.y_plane(), *size, 1);
                write_texture_rows(
                    queue,
                    &chroma_u.texture,
                    yuv_picture.u_plane(),
                    chroma_size,
                    1,
                );
                write_texture_rows(
                    queue,
                    &chroma_v.texture,
                    yuv_picture.v_plane(),
                    chroma_size,
                    1,
                );
            }
            // `fits` is always checked first, so the kinds always match here.
            _ => {}
        }
    }
}

/// What the renderer keeps for one layer position: its uniforms and textures.
pub(crate) struct LayerSlot {
    pub(crate) uniform_buffer: wgpu::Buffer,
    textures: Option<PictureTextures>,
}

impl LayerSlot {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        Self {
            uniform_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Kadr layer uniforms"),
                size: super::pipelines::LAYER_UNIFORM_SIZE_IN_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            textures: None,
        }
    }

    /// The textures of the last uploaded picture.
    pub(crate) fn textures(&self) -> Option<&PictureTextures> {
        self.textures.as_ref()
    }

    /// Uploads `picture`, re-creating the textures only if needed.
    pub(crate) fn upload_picture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        picture: &VideoFrame,
    ) {
        let textures_still_fit = self
            .textures
            .as_ref()
            .is_some_and(|textures| textures.fits(picture));
        if !textures_still_fit {
            self.textures = None;
        }
        self.textures
            .get_or_insert_with(|| PictureTextures::for_picture(device, picture))
            .upload(queue, picture);
    }
}

/// One output plane: the texture rendered into, and the buffer it is copied to
/// for reading on the CPU.
pub(crate) struct OutputPlane {
    pub(crate) target: TextureWithView,
    pub(crate) size: (u32, u32),
    /// Row length in the readback buffer, padded to the GPU copy alignment (256 bytes).
    padded_bytes_per_row: u32,
    readback_buffer: wgpu::Buffer,
}

impl OutputPlane {
    fn new(device: &wgpu::Device, label: &str, size: (u32, u32)) -> Self {
        let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = size.0.div_ceil(alignment) * alignment;
        Self {
            target: create_texture(
                device,
                label,
                size,
                PLANE_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            ),
            size,
            padded_bytes_per_row,
            readback_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: u64::from(padded_bytes_per_row) * u64::from(size.1),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        }
    }

    /// Records the copy of the rendered plane into the readback buffer.
    pub(crate) fn record_copy_to_readback(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.size.1),
                },
            },
            wgpu::Extent3d {
                width: self.size.0,
                height: self.size.1,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Asks for the readback buffer to be mapped; `on_mapped` reports the result.
    pub(crate) fn request_mapping(
        &self,
        on_mapped: std::sync::mpsc::Sender<Result<(), wgpu::BufferAsyncError>>,
    ) {
        self.readback_buffer
            .map_async(wgpu::MapMode::Read, .., move |result| {
                // The receiver only disappears if rendering was abandoned; nothing to report then.
                let _ = on_mapped.send(result);
            });
    }

    /// Copies the mapped plane out, without the row padding, then unmaps it.
    pub(crate) fn take_mapped_rows(&self) -> Result<Vec<u8>, wgpu::MapRangeError> {
        let (width, height) = (self.size.0 as usize, self.size.1 as usize);
        let mut packed = Vec::with_capacity(width * height);
        {
            let mapped = self.readback_buffer.slice(..).get_mapped_range()?;
            for row in mapped
                .chunks(self.padded_bytes_per_row as usize)
                .take(height)
            {
                packed.extend_from_slice(&row[..width]);
            }
        }
        self.readback_buffer.unmap();
        Ok(packed)
    }
}

/// Everything sized after the canvas: the canvas itself, the three output planes,
/// and the bindings of the YUV conversion passes.
pub(crate) struct OutputTargets {
    pub(crate) canvas_size: (u32, u32),
    pub(crate) canvas: TextureWithView,
    pub(crate) luma: OutputPlane,
    pub(crate) chroma_u: OutputPlane,
    pub(crate) chroma_v: OutputPlane,
    pub(crate) output_bind_group: wgpu::BindGroup,
}

impl OutputTargets {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_bind_group_layout: &wgpu::BindGroupLayout,
        canvas_size: (u32, u32),
        rgb_to_yuv_rows: &[[f32; 4]; 3],
    ) -> Self {
        let canvas = create_texture(
            device,
            "Kadr canvas",
            canvas_size,
            CANVAS_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let chroma_size = (
            YuvPicture::chroma_width(canvas_size.0),
            YuvPicture::chroma_height(canvas_size.1),
        );
        let output_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Kadr output conversion"),
            size: OUTPUT_UNIFORM_SIZE_IN_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &output_uniform_buffer,
            0,
            &floats_as_bytes(rgb_to_yuv_rows.as_flattened()),
        );
        let output_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Kadr output bindings"),
            layout: output_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: output_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&canvas.view),
                },
            ],
        });
        Self {
            canvas_size,
            canvas,
            luma: OutputPlane::new(device, "Kadr output Y", canvas_size),
            chroma_u: OutputPlane::new(device, "Kadr output U", chroma_size),
            chroma_v: OutputPlane::new(device, "Kadr output V", chroma_size),
            output_bind_group,
        }
    }
}
