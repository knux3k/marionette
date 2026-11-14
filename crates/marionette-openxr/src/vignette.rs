//! Comfort vignette: darkens the edges of each eye's image while the player moves by stick
//! or turns smoothly (`marionette_xr::XrComfort`). Drawn over the finished eye image, so the
//! renderer is untouched; costs one full-screen triangle per eye, and nothing at strength 0.

const SHADER: &str = r#"
struct Params { strength: f32, aspect: f32, _a: f32, _b: f32 };
@group(0) @binding(0) var<uniform> params: Params;

struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    o.uv = uv;
    return o;
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    let c = (in.uv - 0.5) * vec2<f32>(params.aspect, 1.0) * 2.0;
    let r = length(c);
    // Clear in the middle; fades to black from 55% to 95% of the half-height.
    let inner = mix(0.95, 0.55, params.strength);
    let a = smoothstep(inner, inner + 0.4, r) * params.strength;
    return vec4<f32>(0.0, 0.0, 0.0, a);
}
"#;

pub struct Vignette {
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

impl Vignette {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xr vignette"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xr vignette params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("xr vignette"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xr vignette"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xr vignette"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xr vignette"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            buffer,
            bind,
        }
    }

    /// Records the vignette over `target` (single-sampled, already rendered).
    pub fn draw(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        strength: f32,
        aspect: f32,
    ) {
        if strength <= 0.0 {
            return;
        }
        queue.write_buffer(&self.buffer, 0, &f32_bytes([strength.min(1.0), aspect, 0.0, 0.0]));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("xr vignette"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The four floats as the GPU reads them: native byte order, the same bytes as the array in
/// memory, copied into a plain 16-byte array instead of reinterpreting the floats in place.
fn f32_bytes(v: [f32; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (chunk, x) in out.chunks_exact_mut(4).zip(v) {
        chunk.copy_from_slice(&x.to_ne_bytes());
    }
    out
}
