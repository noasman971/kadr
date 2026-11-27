//! The GPU renderer, built on `wgpu` (Vulkan, DirectX 12, Metal, OpenGL ES).
//!
//! For each frame:
//!
//! 1. every layer's picture is uploaded as is — YUV pictures as three planes, so
//!    the colour conversion happens on the GPU, in the shader;
//! 2. layers are drawn onto an RGBA canvas, each as a quad placed by the same
//!    geometry as the CPU renderer, blended with straight alpha;
//! 3. two passes convert the canvas into the Y, U and V planes the encoder takes
//!    (BT.709, limited range);
//! 4. the three planes are copied back to the CPU.
//!
//! Colours use the very same coefficients as the CPU renderer
//! (`YuvColorSpace::yuv_to_rgb` / `rgb_to_yuv`), so both give the same picture
//! within rounding.

mod device;
mod pipelines;
mod resources;

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

use kadr_media::{VideoFrame, YuvColorSpace, YuvPicture};

pub use device::ADAPTER_ENVIRONMENT_VARIABLE;
use device::open_device;
use pipelines::Pipelines;
use resources::{
    LayerSlot, OutputPlane, OutputTargets, PictureTextures, TextureWithView,
    create_color_lut_texture, create_texture, floats_as_bytes,
};

use crate::color_adjustment::{MAXIMUM_COLOR_ADJUSTMENTS_PER_LAYER, shader_values};
use crate::color_lookup_table::{LayerColorLut, NO_LUT_SHADER_ROWS};
use crate::layers::{CanvasDescription, LayerGeometry, VisualLayer};
use crate::renderer::{FrameRenderer, RenderError, pass_through_frame};
use kadr_core::effect::ColorAdjustment;

/// Renders frames on the GPU.
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_description: String,
    pipelines: Pipelines,
    picture_sampler: wgpu::Sampler,
    /// Bound in place of the U and V planes when drawing an RGBA picture.
    placeholder_plane: TextureWithView,
    /// Bound in place of a colour LUT when the layer has none.
    placeholder_color_lut: TextureWithView,
    /// The colour LUTs uploaded, by table fingerprint: uploaded once, kept while
    /// frames use them.
    color_lut_textures: HashMap<u64, TextureWithView>,
    layer_slots: Vec<LayerSlot>,
    output_targets: Option<OutputTargets>,
}

/// A layer ready to draw: which program, with which bindings.
struct PreparedLayer {
    is_yuv: bool,
    bind_group: wgpu::BindGroup,
}

impl GpuRenderer {
    /// Opens the most powerful GPU and prepares the programs.
    ///
    /// # Errors
    ///
    /// Fails when no graphics adapter can be opened.
    pub fn new() -> Result<Self, RenderError> {
        let open_device = open_device()?;
        let device = open_device.device;
        let pipelines = Pipelines::create(&device);
        let picture_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Kadr picture sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let placeholder_plane = create_texture(
            &device,
            "Kadr placeholder plane",
            (1, 1),
            pipelines::PLANE_FORMAT,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let placeholder_color_lut =
            create_color_lut_texture(&device, &open_device.queue, [1, 1, 1], &[0.0; 4]);
        Ok(Self {
            device,
            queue: open_device.queue,
            adapter_description: open_device.adapter_description,
            pipelines,
            picture_sampler,
            placeholder_plane,
            placeholder_color_lut,
            color_lut_textures: HashMap::new(),
            layer_slots: Vec::new(),
            output_targets: None,
        })
    }

    /// The canvas, output planes and conversion bindings for `canvas_size`.
    fn output_targets_for(&mut self, canvas_size: (u32, u32)) -> &OutputTargets {
        let still_fits = self
            .output_targets
            .as_ref()
            .is_some_and(|targets| targets.canvas_size == canvas_size);
        if !still_fits {
            self.output_targets = None;
        }
        let (device, queue, layout) = (
            &self.device,
            &self.queue,
            &self.pipelines.output_bind_group_layout,
        );
        self.output_targets.get_or_insert_with(|| {
            OutputTargets::new(
                device,
                queue,
                layout,
                canvas_size,
                &YuvColorSpace::BT709_LIMITED.rgb_to_yuv().rows,
            )
        })
    }

    /// Uploads one layer's picture and uniforms; `None` if the layer shows nothing.
    fn prepare_layer(
        &mut self,
        slot_index: usize,
        canvas: &CanvasDescription,
        layer: &VisualLayer<'_>,
    ) -> Option<PreparedLayer> {
        let opacity = layer.placement.clamped_opacity();
        let picture = layer.picture;
        let geometry = LayerGeometry::new(
            canvas.size,
            picture.width(),
            picture.height(),
            &layer.placement,
        )?;
        if opacity <= 0.0 {
            return None;
        }
        while self.layer_slots.len() <= slot_index {
            self.layer_slots.push(LayerSlot::new(&self.device));
        }
        let color_lut = layer.color_lut.filter(LayerColorLut::is_visible);
        if let Some(color_lut) = color_lut {
            self.upload_color_lut(color_lut);
        }
        let uniforms = layer_uniforms(
            canvas,
            &geometry,
            picture,
            opacity,
            layer.color_adjustments,
            color_lut,
        );
        self.layer_slots[slot_index].upload_picture(&self.device, &self.queue, picture);
        let slot = &self.layer_slots[slot_index];
        self.queue
            .write_buffer(&slot.uniform_buffer, 0, &floats_as_bytes(&uniforms));
        let (first, second, third, is_yuv) = match slot.textures()? {
            PictureTextures::Rgba { rgba, .. } => (
                &rgba.view,
                &self.placeholder_plane.view,
                &self.placeholder_plane.view,
                false,
            ),
            PictureTextures::Yuv(textures) => (
                &textures.luma.view,
                &textures.chroma_u.view,
                &textures.chroma_v.view,
                true,
            ),
        };
        let color_lut_view = color_lut
            .and_then(|color_lut| self.color_lut_textures.get(&color_lut.table.fingerprint()))
            .map_or(&self.placeholder_color_lut.view, |texture| &texture.view);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Kadr layer bindings"),
            layout: &self.pipelines.layer_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: slot.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.picture_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(first),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(second),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(third),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(color_lut_view),
                },
            ],
        });
        Some(PreparedLayer { is_yuv, bind_group })
    }

    /// Uploads the LUT's table, unless a layer already did.
    fn upload_color_lut(&mut self, color_lut: LayerColorLut<'_>) {
        let table = color_lut.table;
        let (device, queue) = (&self.device, &self.queue);
        self.color_lut_textures
            .entry(table.fingerprint())
            .or_insert_with(|| {
                create_color_lut_texture(
                    device,
                    queue,
                    table.texture_extent(),
                    &table.texture_texels(),
                )
            });
    }

    /// Forgets the LUTs no layer of the current frame uses.
    fn keep_only_color_luts_of(&mut self, layers: &[VisualLayer<'_>]) {
        let in_use: HashSet<u64> = layers
            .iter()
            .filter_map(|layer| layer.color_lut)
            .map(|color_lut| color_lut.table.fingerprint())
            .collect();
        self.color_lut_textures
            .retain(|fingerprint, _| in_use.contains(fingerprint));
    }

    /// Records the drawing of every layer and the conversion to YUV planes.
    fn record_frame(
        &self,
        canvas: &CanvasDescription,
        prepared_layers: &[PreparedLayer],
        targets: &OutputTargets,
    ) -> wgpu::CommandBuffer {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Kadr frame"),
            });
        let background = canvas.background;
        let clear_color = wgpu::Color {
            r: f64::from(background.red) / 255.0,
            g: f64::from(background.green) / 255.0,
            b: f64::from(background.blue) / 255.0,
            a: 1.0,
        };
        {
            let mut canvas_pass = begin_pass(
                &mut encoder,
                "Kadr layers",
                &[&targets.canvas.view],
                wgpu::LoadOp::Clear(clear_color),
            );
            for prepared_layer in prepared_layers {
                let pipeline = if prepared_layer.is_yuv {
                    &self.pipelines.yuv_layer
                } else {
                    &self.pipelines.rgba_layer
                };
                canvas_pass.set_pipeline(pipeline);
                canvas_pass.set_bind_group(0, &prepared_layer.bind_group, &[]);
                canvas_pass.draw(0..4, 0..1);
            }
        }
        {
            let mut luma_pass = begin_pass(
                &mut encoder,
                "Kadr Y plane",
                &[&targets.luma.target.view],
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            );
            luma_pass.set_pipeline(&self.pipelines.luma_output);
            luma_pass.set_bind_group(0, &targets.output_bind_group, &[]);
            luma_pass.draw(0..3, 0..1);
        }
        {
            let chroma_views = [&targets.chroma_u.target.view, &targets.chroma_v.target.view];
            let mut chroma_pass = begin_pass(
                &mut encoder,
                "Kadr U and V planes",
                &chroma_views,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            );
            chroma_pass.set_pipeline(&self.pipelines.chroma_output);
            chroma_pass.set_bind_group(0, &targets.output_bind_group, &[]);
            chroma_pass.draw(0..3, 0..1);
        }
        for plane in [&targets.luma, &targets.chroma_u, &targets.chroma_v] {
            plane.record_copy_to_readback(&mut encoder);
        }
        encoder.finish()
    }
}

/// Starts a render pass drawing into `views`.
fn begin_pass<'encoder>(
    encoder: &'encoder mut wgpu::CommandEncoder,
    label: &str,
    views: &[&wgpu::TextureView],
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPass<'encoder> {
    let color_attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = views
        .iter()
        .map(|view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })
        })
        .collect();
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &color_attachments,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// The uniforms of one layer, in the order of `LayerUniforms` in `layers.wgsl`.
fn layer_uniforms(
    canvas: &CanvasDescription,
    geometry: &LayerGeometry,
    picture: &VideoFrame,
    opacity: f32,
    color_adjustments: &[ColorAdjustment],
    color_lut: Option<LayerColorLut<'_>>,
) -> Vec<f32> {
    let (canvas_width, canvas_height) = (
        canvas.size.width_in_pixels as f32,
        canvas.size.height_in_pixels as f32,
    );
    let mut uniforms = Vec::with_capacity(64);
    let texture_corners = geometry.texture_corners(picture.width(), picture.height());
    for ((canvas_x, canvas_y), (texture_x, texture_y)) in
        geometry.canvas_corners().into_iter().zip(texture_corners)
    {
        // Canvas pixels to clip space: x from −1 (left) to 1, y from 1 (top) to −1.
        uniforms.extend([
            canvas_x / canvas_width * 2.0 - 1.0,
            1.0 - canvas_y / canvas_height * 2.0,
            texture_x,
            texture_y,
        ]);
    }
    let yuv_to_rgb_rows = match picture {
        VideoFrame::Yuv420(yuv_picture) => yuv_picture.color_space().yuv_to_rgb().rows,
        VideoFrame::Rgba(_) => [[0.0; 4]; 3],
    };
    uniforms.extend(yuv_to_rgb_rows.as_flattened());
    uniforms.extend([opacity, 0.0, 0.0, 0.0]);
    let adjustment_count = color_adjustments
        .len()
        .min(MAXIMUM_COLOR_ADJUSTMENTS_PER_LAYER);
    for slot in 0..MAXIMUM_COLOR_ADJUSTMENTS_PER_LAYER {
        let values = color_adjustments
            .get(slot)
            .filter(|_| slot < adjustment_count)
            .map_or([0.0; 4], shader_values);
        uniforms.extend(values);
    }
    uniforms.extend([adjustment_count as f32, 0.0, 0.0, 0.0]);
    let color_lut_rows = color_lut.map_or(NO_LUT_SHADER_ROWS, |color_lut| {
        color_lut.table.shader_rows(color_lut.strength_fraction)
    });
    uniforms.extend(color_lut_rows.as_flattened());
    uniforms
}

/// Waits until the three output planes are readable and copies them out.
fn read_output_planes(
    device: &wgpu::Device,
    targets: &OutputTargets,
) -> Result<[Vec<u8>; 3], RenderError> {
    let planes = [&targets.luma, &targets.chroma_u, &targets.chroma_v];
    let (sender, receiver) = mpsc::channel();
    for plane in planes {
        plane.request_mapping(sender.clone());
    }
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| RenderError::GpuFailure {
            reason: format!("waiting for the frame: {error}"),
        })?;
    for _ in planes {
        receiver
            .recv()
            .map_err(|error| RenderError::GpuFailure {
                reason: format!("reading the frame back: {error}"),
            })?
            .map_err(|error| RenderError::GpuFailure {
                reason: format!("reading the frame back: {error}"),
            })?;
    }
    let [luma, chroma_u, chroma_v] = planes.map(OutputPlane::take_mapped_rows);
    let reading_failure = |error: wgpu::MapRangeError| RenderError::GpuFailure {
        reason: format!("reading the frame back: {error}"),
    };
    Ok([
        luma.map_err(reading_failure)?,
        chroma_u.map_err(reading_failure)?,
        chroma_v.map_err(reading_failure)?,
    ])
}

impl FrameRenderer for GpuRenderer {
    fn description(&self) -> String {
        self.adapter_description.clone()
    }

    fn render_frame(
        &mut self,
        canvas: &CanvasDescription,
        layers: &[VisualLayer<'_>],
    ) -> Result<VideoFrame, RenderError> {
        if let Some(frame) = pass_through_frame(canvas, layers) {
            return Ok(frame);
        }
        let error_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let canvas_size = (canvas.size.width_in_pixels, canvas.size.height_in_pixels);
        self.output_targets_for(canvas_size);

        self.keep_only_color_luts_of(layers);
        let prepared_layers: Vec<PreparedLayer> = layers
            .iter()
            .enumerate()
            .filter_map(|(slot_index, layer)| self.prepare_layer(slot_index, canvas, layer))
            .collect();
        let targets = self
            .output_targets
            .as_ref()
            .ok_or_else(|| RenderError::GpuFailure {
                reason: "the output targets were not created".to_string(),
            })?;
        let commands = self.record_frame(canvas, &prepared_layers, targets);
        self.queue.submit([commands]);
        let [y_plane, u_plane, v_plane] = read_output_planes(&self.device, targets)?;

        if let Some(error) = pollster::block_on(error_scope.pop()) {
            return Err(RenderError::GpuFailure {
                reason: error.to_string(),
            });
        }
        YuvPicture::from_planes(
            canvas_size.0,
            canvas_size.1,
            y_plane,
            u_plane,
            v_plane,
            YuvColorSpace::BT709_LIMITED,
        )
        .map(VideoFrame::Yuv420)
        .map_err(|error| RenderError::GpuFailure {
            reason: error.to_string(),
        })
    }
}
