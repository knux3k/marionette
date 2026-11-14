//! Lit pipeline variants per material (decision 0280): single- or double-sided, opaque or
//! blended. Cut-outs need no variant (the fragment shader discards below the material's
//! cutoff). Variants other than the plain one are built the first time a frame draws them,
//! so scenes without such materials pay nothing.

use crate::assets::{AlphaMode, MaterialData};
use std::sync::OnceLock;

/// Number of variants: bit 0 = both faces drawn, bit 1 = blended.
pub(crate) const VARIANTS: usize = 4;
pub(crate) const DOUBLE_SIDED: usize = 1;
pub(crate) const BLENDED: usize = 2;

/// How one drawn part looks to the pipelines: its variant and whether it is cut out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) struct Look {
    pub variant: u8,
    pub masked: bool,
}

impl Look {
    pub fn blended(self) -> bool {
        self.variant as usize & BLENDED != 0
    }

    /// The look of a part drawn with its own material `m`, or with a `Material` override:
    /// an override keeps the part's sidedness, has no cut-out and blends when its base color
    /// is see-through.
    pub fn of(m: &MaterialData, over: Option<&marionette_core::spatial::Material>) -> Self {
        let sides = if m.double_sided { DOUBLE_SIDED } else { 0 };
        match over {
            Some(o) => Self {
                variant: (sides | if o.base_color.a < 1.0 { BLENDED } else { 0 }) as u8,
                masked: false,
            },
            None => Self {
                variant: (sides
                    | if m.alpha_mode == AlphaMode::Blend {
                        BLENDED
                    } else {
                        0
                    }) as u8,
                masked: matches!(m.alpha_mode, AlphaMode::Mask { .. }),
            },
        }
    }
}

/// Culling, blending and depth writes of a variant.
pub(crate) fn state(
    variant: usize,
    depth: &wgpu::DepthStencilState,
) -> (
    Option<wgpu::Face>,
    Option<wgpu::BlendState>,
    wgpu::DepthStencilState,
) {
    let cull = if variant & DOUBLE_SIDED != 0 {
        None
    } else {
        Some(wgpu::Face::Back)
    };
    if variant & BLENDED != 0 {
        let depth = wgpu::DepthStencilState {
            depth_write_enabled: Some(false),
            ..depth.clone()
        };
        (cull, Some(wgpu::BlendState::ALPHA_BLENDING), depth)
    } else {
        (cull, None, depth.clone())
    }
}

type Make = Box<dyn Fn(usize, usize) -> wgpu::RenderPipeline + Send + Sync>;

/// Targets a pipeline draws into: the final format, the floating-point bloom target, and the
/// final format with both eyes in one pass (decision 0285).
pub(crate) const TARGETS: usize = 3;
pub(crate) const FINAL: usize = 0;
pub(crate) const HDR: usize = 1;
pub(crate) const STEREO: usize = 2;

/// One pipeline per variant and target, made on first use.
pub(crate) struct VariantPipelines {
    built: [[OnceLock<wgpu::RenderPipeline>; TARGETS]; VARIANTS],
    make: Make,
}

impl VariantPipelines {
    /// `make(variant, target)` builds one; the plain variant's final and floating-point
    /// pipelines are built right away.
    pub fn new(make: Make) -> Self {
        let built: [[OnceLock<wgpu::RenderPipeline>; TARGETS]; VARIANTS] = Default::default();
        for t in [FINAL, HDR] {
            let _ = built[0][t].set(make(0, t));
        }
        Self { built, make }
    }

    /// The pipeline of `variant` into `target` ([`FINAL`], [`HDR`] or [`STEREO`]).
    pub fn get(&self, variant: usize, target: usize) -> &wgpu::RenderPipeline {
        let t = target.min(TARGETS - 1);
        let v = variant.min(VARIANTS - 1);
        self.built[v][t].get_or_init(|| (self.make)(v, t))
    }
}
