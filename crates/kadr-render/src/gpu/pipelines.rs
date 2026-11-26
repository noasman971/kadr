//! The GPU programs (pipelines) and the layout of what they read.

/// Size of the layer uniforms, 4 floats each: 4 corners, 3 conversion rows,
/// opacity, 4 colour adjustments and their count.
pub(crate) const LAYER_UNIFORM_SIZE_IN_BYTES: u64 = 13 * 16;
/// Size of the output conversion uniforms: 3 rows of 4 floats.
pub(crate) const OUTPUT_UNIFORM_SIZE_IN_BYTES: u64 = 3 * 16;

/// Format of the canvas while layers are drawn. Not sRGB: blending happens on the
/// stored values, exactly like the CPU renderer.
pub(crate) const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Format of each YUV output plane.
pub(crate) const PLANE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Every pipeline the renderer uses.
pub(crate) struct Pipelines {
    pub(crate) layer_bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) rgba_layer: wgpu::RenderPipeline,
    pub(crate) yuv_layer: wgpu::RenderPipeline,
    pub(crate) output_bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) luma_output: wgpu::RenderPipeline,
    pub(crate) chroma_output: wgpu::RenderPipeline,
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Straight-alpha "over" blending for colour; the canvas alpha stays opaque.
fn layer_blend_state() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

/// One render pipeline from a shader module, its entry points and targets.
fn create_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    (vertex_entry, fragment_entry): (&str, &str),
    topology: wgpu::PrimitiveTopology,
    targets: &[Option<wgpu::ColorTargetState>],
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vertex_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl Pipelines {
    pub(crate) fn create(device: &wgpu::Device) -> Self {
        let layer_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Kadr layers"),
            source: wgpu::ShaderSource::Wgsl(include_str!("layers.wgsl").into()),
        });
        let output_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Kadr YUV output"),
            source: wgpu::ShaderSource::Wgsl(include_str!("yuv_output.wgsl").into()),
        });

        let layer_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Kadr layer bindings"),
                entries: &[
                    uniform_entry(0),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    texture_entry(2, true),
                    texture_entry(3, true),
                    texture_entry(4, true),
                ],
            });
        let layer_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Kadr layer pipeline layout"),
                bind_group_layouts: &[Some(&layer_bind_group_layout)],
                immediate_size: 0,
            });
        let canvas_target = [Some(wgpu::ColorTargetState {
            format: CANVAS_FORMAT,
            blend: Some(layer_blend_state()),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let strip = wgpu::PrimitiveTopology::TriangleStrip;
        let rgba_layer = create_pipeline(
            device,
            "Kadr RGBA layer",
            &layer_pipeline_layout,
            &layer_module,
            ("layer_vertex", "rgba_layer_fragment"),
            strip,
            &canvas_target,
        );
        let yuv_layer = create_pipeline(
            device,
            "Kadr YUV layer",
            &layer_pipeline_layout,
            &layer_module,
            ("layer_vertex", "yuv_layer_fragment"),
            strip,
            &canvas_target,
        );

        let output_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Kadr output bindings"),
                entries: &[uniform_entry(0), texture_entry(1, false)],
            });
        let output_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Kadr output pipeline layout"),
                bind_group_layouts: &[Some(&output_bind_group_layout)],
                immediate_size: 0,
            });
        let plane_target = Some(wgpu::ColorTargetState {
            format: PLANE_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        });
        let triangles = wgpu::PrimitiveTopology::TriangleList;
        let luma_output = create_pipeline(
            device,
            "Kadr luma output",
            &output_pipeline_layout,
            &output_module,
            ("full_target_vertex", "luma_fragment"),
            triangles,
            std::slice::from_ref(&plane_target),
        );
        let chroma_output = create_pipeline(
            device,
            "Kadr chroma output",
            &output_pipeline_layout,
            &output_module,
            ("full_target_vertex", "chroma_fragment"),
            triangles,
            &[plane_target.clone(), plane_target],
        );

        Self {
            layer_bind_group_layout,
            rgba_layer,
            yuv_layer,
            output_bind_group_layout,
            luma_output,
            chroma_output,
        }
    }
}
