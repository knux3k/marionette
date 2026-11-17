//! Game shaders (decision 0181): WGSL a game writes, spliced into the PBR shader so it still
//! gets the engine's lights, shadows, environment, fog, tone mapping and bloom.
//!
//! Composition: `pbr.wgsl` with its `// @hooks-begin` .. `// @hooks-end` block replaced by
//! hooks that call the game's `surface` / `vertex` functions, then the game bindings
//! (group 3: `game.params`, `tex0`..`tex3`, `game_samp`), then the game's source. Each shader
//! gets pipelines for the final and HDR targets, and a depth pipeline when its `vertex`
//! function moves vertices (so shadows follow). Built on first use, rebuilt when the source
//! changes. Compile errors are diagnostics; the affected models draw with the standard
//! shader meanwhile.

use super::{DEPTH_FORMAT, INSTANCE_ATTRS, InstanceData, MESH_ATTRS, MeshVertex, Renderer};
use crate::extract::FrameData;
use marionette_core::shader::{
    MAX_SHADER_PARAMS, MAX_SHADER_TEXTURES, ShaderBlend, ShaderMaterial, ShaderOptions,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

const PBR: &str = include_str!("../shaders/pbr.wgsl");
const HOOKS_BEGIN: &str = "// @hooks-begin";
const HOOKS_END: &str = "// @hooks-end";

/// Bindings every game shader can use (group 3).
const PRELUDE: &str = "
// ---- game shader bindings (decision 0181) ----
struct GameU {
    params: array<vec4<f32>, 4>,
};
@group(3) @binding(0) var<uniform> game: GameU;
@group(3) @binding(1) var tex0: texture_2d<f32>;
@group(3) @binding(2) var tex1: texture_2d<f32>;
@group(3) @binding(3) var tex2: texture_2d<f32>;
@group(3) @binding(4) var tex3: texture_2d<f32>;
@group(3) @binding(5) var game_samp: sampler;
// ---- game source ----
";

/// One compiled game shader.
pub(crate) struct GameShaderGpu {
    pub options: ShaderOptions,
    /// Lit pipelines: [final target format, HDR target].
    pub lit: [wgpu::RenderPipeline; 2],
    /// Depth pipeline that runs the shader's `vertex` function; `None` uses the standard
    /// depth pipeline (no `vertex` function, or the shader casts no shadows).
    pub shadow: Option<wgpu::RenderPipeline>,
    /// Lit pipelines for skinned meshes (decision 0185): [final target format, HDR target].
    /// `None` when the skinned variant does not compile (skinned draws then use the standard
    /// skinned shader).
    pub skinned: Option<[wgpu::RenderPipeline; 2]>,
}

/// What a draw with a game shader needs: the shader and its material's group 3.
pub(crate) type GameDraw<'a> = (&'a GameShaderGpu, &'a wgpu::BindGroup);

pub(crate) struct GameShading {
    game_layout: wgpu::BindGroupLayout,
    lit_layout: wgpu::PipelineLayout,
    /// Group 3 of skinned game shaders: the game bindings plus the skin buffers (decision 0185).
    skinned_layout: wgpu::BindGroupLayout,
    skinned_lit_layout: wgpu::PipelineLayout,
    /// The uniform buffer and images of each material group (key as in `groups`), to make the
    /// skinned groups of a frame.
    materials: BTreeMap<String, (wgpu::Buffer, Vec<wgpu::TextureView>)>,
    shadow_layout: wgpu::PipelineLayout,
    /// Bound at groups 1 and 2 in depth passes of shaders with a `vertex` function.
    pub empty_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    /// Per shader name: the source it was built from and the result.
    shaders: BTreeMap<String, (Arc<str>, Result<GameShaderGpu, String>)>,
    /// Group 3 per distinct material (key: shader, params bits, textures).
    pub(super) groups: BTreeMap<String, wgpu::BindGroup>,
    pub(super) textures: BTreeMap<String, Result<wgpu::TextureView, String>>,
    /// Per material of the current frame (`FrameData::shader_materials`, then one per bulk
    /// draw): (shader name, group key), or `None` when it cannot draw yet.
    frame: Vec<Option<(String, String)>>,
}

/// Index of a bulk draw's material in `GameShading::frame`.
pub(crate) fn bulk_slot(frame: &FrameData, bulk: usize) -> u32 {
    (frame.shader_materials.len() + bulk) as u32
}

impl GameShading {
    fn new(r: &Renderer) -> Self {
        let device = &r.device;
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let game_entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            tex(1),
            tex(2),
            tex(3),
            tex(4),
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ];
        let game_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("game shader"),
            entries: &game_entries,
        });
        let skin = |binding, min| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: std::num::NonZeroU64::new(min),
            },
            count: None,
        };
        let mut skinned_entries = game_entries.to_vec();
        skinned_entries.extend([skin(6, 64), skin(7, 4), skin(8, 16)]);
        let skinned_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skinned game shader"),
            entries: &skinned_entries,
        });
        let empty_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("empty"),
            entries: &[],
        });
        let empty_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("empty"),
            layout: &empty_layout,
            entries: &[],
        });
        let lit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("game shader"),
            bind_group_layouts: &[
                Some(&r.view_layout),
                Some(&r.lights_layout),
                Some(&r.material_layout),
                Some(&game_layout),
            ],
            immediate_size: 0,
        });
        let skinned_lit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skinned game shader"),
            bind_group_layouts: &[
                Some(&r.view_layout),
                Some(&r.lights_layout),
                Some(&r.material_layout),
                Some(&skinned_layout),
            ],
            immediate_size: 0,
        });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("game shader depth"),
            bind_group_layouts: &[
                Some(&r.view_layout),
                Some(&empty_layout),
                Some(&empty_layout),
                Some(&game_layout),
            ],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("game shader"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let white =
            super::create_texture_view(device, &r.queue, &crate::assets::Image::solid([255; 4], true));
        Self {
            game_layout,
            lit_layout,
            skinned_layout,
            skinned_lit_layout,
            materials: BTreeMap::new(),
            shadow_layout,
            empty_group,
            sampler,
            white,
            shaders: BTreeMap::new(),
            groups: BTreeMap::new(),
            textures: BTreeMap::new(),
            frame: Vec::new(),
        }
    }

    /// The skinned pipelines of material slot `slot` and its group 3 over this frame's skin
    /// buffers (palettes, morph weights, morph deltas), if it can draw skinned (decision 0185).
    pub fn skinned_draw(
        &self,
        device: &wgpu::Device,
        slot: u32,
        [palettes, weights, deltas]: [&wgpu::Buffer; 3],
    ) -> Option<(&GameShaderGpu, wgpu::BindGroup)> {
        let (shader, key) = self.frame.get(slot as usize)?.as_ref()?;
        let gpu = self.shaders.get(shader)?.1.as_ref().ok()?;
        gpu.skinned.as_ref()?;
        let (uniform, views) = self.materials.get(key)?;
        fn buffer(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            }
        }
        let mut entries = vec![buffer(0, uniform)];
        for (i, v) in views.iter().enumerate() {
            entries.push(tex(1 + i as u32, v));
        }
        entries.push(wgpu::BindGroupEntry {
            binding: 5,
            resource: wgpu::BindingResource::Sampler(&self.sampler),
        });
        entries.extend([buffer(6, palettes), buffer(7, weights), buffer(8, deltas)]);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skinned game shader material"),
            layout: &self.skinned_layout,
            entries: &entries,
        });
        Some((gpu, group))
    }

    /// The shader and group of material slot `slot` this frame, if it can draw.
    pub fn draw(&self, slot: u32) -> Option<GameDraw<'_>> {
        let (shader, key) = self.frame.get(slot as usize)?.as_ref()?;
        let gpu = self.shaders.get(shader)?.1.as_ref().ok()?;
        Some((gpu, self.groups.get(key)?))
    }
}

/// The game source with the engine's shader around it; also the line where the game's
/// source starts (for error messages).
/// Game shader state is made when the renderer first sees a game shader, before any of
/// these run; this is only the answer if that order ever breaks.
const NOT_SET_UP: &str = "game shaders are not set up";

pub(crate) fn compose(source: &str, o: &ShaderOptions) -> Result<(String, usize), String> {
    let (Some(begin), Some(end)) = (PBR.find(HOOKS_BEGIN), PBR.find(HOOKS_END)) else {
        return Err("the engine's PBR shader lost its hook markers (an engine bug)".into());
    };
    let end = end + HOOKS_END.len();
    let hooks = format!(
        "fn vertex_hook(v: ptr<function, Vertex>) {{ {} }}\nfn surface_hook(s: ptr<function, Surface>, i: SurfaceIn) {{ {} }}\nconst GAME_FOG: f32 = {};",
        if o.has_vertex { "vertex(v);" } else { "" },
        if o.has_surface { "surface(s, i);" } else { "" },
        if o.no_fog { "0.0" } else { "1.0" },
    );
    let head = format!("{}{hooks}{}{PRELUDE}", &PBR[..begin], &PBR[end..]);
    let line = head.lines().count() + 1;
    Ok((format!("{head}{source}\n"), line))
}

fn material_key(m: &ShaderMaterial) -> String {
    let mut key = m.shader.clone();
    for p in m.params.iter().take(MAX_SHADER_PARAMS) {
        key.push_str(&format!("|{:08x}", p.to_bits()));
    }
    key.push('#');
    for t in m.textures.iter().take(MAX_SHADER_TEXTURES) {
        key.push_str(t);
        key.push('|');
    }
    key
}

impl Renderer {
    /// Compiles new or changed game shaders and builds material groups for the frame.
    /// Returns (shader or material, message) for each problem.
    pub(super) fn sync_game_shaders(&mut self, frame: &FrameData) -> Vec<(String, String)> {
        let any = !frame.shader_materials.is_empty() || frame.bulk.iter().any(|b| b.shader.is_some());
        if !any {
            if let Some(g) = self.game.as_mut() {
                g.frame.clear();
            }
            return Vec::new();
        }
        if self.game.is_none() {
            self.game = Some(GameShading::new(self));
        }
        let mut problems = Vec::new();
        let materials: Vec<Option<&ShaderMaterial>> = frame
            .shader_materials
            .iter()
            .map(Some)
            .chain(frame.bulk.iter().map(|b| b.shader.as_ref()))
            .collect();
        let mut slots = Vec::with_capacity(materials.len());
        for m in materials {
            let Some(m) = m else {
                slots.push(None);
                continue;
            };
            if m.params.len() > MAX_SHADER_PARAMS || m.textures.len() > MAX_SHADER_TEXTURES {
                problems.push((
                    m.shader.clone(),
                    format!(
                        "ShaderMaterial has {} params and {} textures; at most {MAX_SHADER_PARAMS} and {MAX_SHADER_TEXTURES} (extra ones ignored)",
                        m.params.len(),
                        m.textures.len()
                    ),
                ));
            }
            let ok = match self.ensure_game_shader(&m.shader, frame) {
                Ok(()) => true,
                Err(msg) => {
                    problems.push((m.shader.clone(), msg));
                    false
                }
            };
            if !ok {
                slots.push(None);
                continue;
            }
            let key = material_key(m);
            match self.ensure_game_group(&key, m) {
                Ok(()) => slots.push(Some((m.shader.clone(), key))),
                Err(msg) => {
                    problems.push((m.shader.clone(), msg));
                    slots.push(None);
                }
            }
        }
        problems.sort();
        problems.dedup();
        if let Some(g) = self.game.as_mut() {
            g.frame = slots;
        }
        problems
    }

    fn shader_source(&self, name: &str, frame: &FrameData) -> Result<Arc<str>, String> {
        if let Some(s) = frame.game_shaders.get(name) {
            return Ok(Arc::clone(s));
        }
        if name.ends_with(".wgsl") {
            let path = self.base_dir.join(name);
            return std::fs::read_to_string(&path)
                .map(Arc::from)
                .map_err(|e| format!("cannot read shader {}: {e}", path.display()));
        }
        Err(format!(
            "no game shader named `{name}` (store one with GameShaders::set, or give a .wgsl file relative to the scene)"
        ))
    }

    fn ensure_game_shader(&mut self, name: &str, frame: &FrameData) -> Result<(), String> {
        let cached = self.game.as_ref().and_then(|g| g.shaders.get(name));
        // Stored sources are compared every frame (a pointer check, rarely a text compare);
        // a file is read once.
        let source = match (cached, frame.game_shaders.get(name)) {
            (Some((old, _)), Some(src)) if Arc::ptr_eq(old, src) || **old == **src => None,
            (Some(_), None) if name.ends_with(".wgsl") => None,
            _ => Some(self.shader_source(name, frame)?),
        };
        if let Some(source) = source {
            let built = self.build_game_shader(&source);
            if let Some(game) = self.game.as_mut() {
                game.shaders.insert(name.to_string(), (source, built));
            }
        }
        match self.game.as_ref().and_then(|g| g.shaders.get(name)) {
            Some((_, Ok(_))) => Ok(()),
            Some((_, Err(msg))) => Err(msg.clone()),
            None => Err(format!("no game shader named `{name}`")),
        }
    }

    fn build_game_shader(&self, source: &str) -> Result<GameShaderGpu, String> {
        let options = ShaderOptions::parse(source)?;
        let (wgsl, line) = compose(source, &options)?;
        let game = self.game.as_ref().ok_or(NOT_SET_UP)?;
        let device = &self.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("game shader"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<MeshVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &MESH_ATTRS,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<InstanceData>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &INSTANCE_ATTRS,
            }),
        ];
        let blend = match options.blend {
            ShaderBlend::Opaque => None,
            ShaderBlend::Alpha => Some(wgpu::BlendState::ALPHA_BLENDING),
            ShaderBlend::Additive => Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
        };
        let lit_for = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("game shader"),
                layout: Some(&game.lit_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_hooked"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: if options.double_sided {
                        None
                    } else {
                        Some(wgpu::Face::Back)
                    },
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(options.blend == ShaderBlend::Opaque),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: self.sample_count,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let lit = [lit_for(self.color_format), lit_for(crate::post::HDR_FORMAT)];
        let shadow = (options.has_vertex && options.casts_shadows()).then(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("game shader depth"),
                layout: Some(&game.shadow_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_shadow_hooked"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(super::shadow_depth_state()),
                multisample: wgpu::MultisampleState::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            })
        });
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(format!(
                "{err} (the game source starts at line {line} of the composed shader)"
            ));
        }
        let skinned = self.build_skinned_game_shader(&options, source, blend);
        Ok(GameShaderGpu {
            options,
            lit,
            shadow,
            skinned,
        })
    }

    /// The skinned variant of a game shader that compiled (decision 0185); `None` if it
    /// does not compile (the rigid variant still draws).
    fn build_skinned_game_shader(
        &self,
        options: &ShaderOptions,
        source: &str,
        blend: Option<wgpu::BlendState>,
    ) -> Option<[wgpu::RenderPipeline; 2]> {
        let (composed, _) = compose(source, options).ok()?;
        let game = self.game.as_ref()?;
        let device = &self.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skinned game shader"),
            source: wgpu::ShaderSource::Wgsl(crate::skinning::skinned_game_source(&composed).into()),
        });
        let buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<MeshVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &MESH_ATTRS,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<crate::skinning::SkinnedInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &crate::skinning::INSTANCE_ATTRS,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<crate::skinning::SkinVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &crate::skinning::SKIN_ATTRS,
            }),
        ];
        let lit_for = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("skinned game shader"),
                layout: Some(&game.skinned_lit_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_skinned_hooked"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: if options.double_sided {
                        None
                    } else {
                        Some(wgpu::Face::Back)
                    },
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(options.blend == ShaderBlend::Opaque),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: self.sample_count,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let lit = [lit_for(self.color_format), lit_for(crate::post::HDR_FORMAT)];
        match pollster::block_on(scope.pop()) {
            Some(_) => None,
            None => Some(lit),
        }
    }

    fn ensure_game_group(&mut self, key: &str, m: &ShaderMaterial) -> Result<(), String> {
        if self.game.as_ref().is_some_and(|g| g.groups.contains_key(key)) {
            return Ok(());
        }
        for t in m.textures.iter().take(MAX_SHADER_TEXTURES) {
            if !self.game.as_ref().is_some_and(|g| g.textures.contains_key(t)) {
                let view = match self.game_image_view(t) {
                    Some(found) => found.map(|(view, ..)| view),
                    None => crate::assets::load_image(t, &self.base_dir)
                        .map(|img| super::create_texture_view(&self.device, &self.queue, &img)),
                };
                if let Some(g) = self.game.as_mut() {
                    g.textures.insert(t.clone(), view);
                }
            }
        }
        let game = self.game.as_mut().ok_or(NOT_SET_UP)?;
        let mut views = Vec::with_capacity(MAX_SHADER_TEXTURES);
        for i in 0..MAX_SHADER_TEXTURES {
            match m.textures.get(i) {
                Some(t) => match &game.textures[t] {
                    Ok(v) => views.push(v),
                    Err(msg) => return Err(format!("texture {t}: {msg}")),
                },
                None => views.push(&game.white),
            }
        }
        let mut params = [0.0f32; MAX_SHADER_PARAMS];
        for (p, v) in params.iter_mut().zip(&m.params) {
            *p = *v;
        }
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("game shader params"),
            contents: bytemuck::cast_slice(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("game shader material"),
            layout: &game.game_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                tex(1, views[0]),
                tex(2, views[1]),
                tex(3, views[2]),
                tex(4, views[3]),
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&game.sampler),
                },
            ],
        });
        game.groups.insert(key.to_string(), group);
        // Kept to make the skinned groups of a frame (decision 0185); views are reference
        // counted handles.
        let views = views.into_iter().cloned().collect();
        game.materials.insert(key.to_string(), (uniform, views));
        Ok(())
    }
}

fn tex(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}
