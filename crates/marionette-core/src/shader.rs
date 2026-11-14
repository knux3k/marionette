//! Materials with a game's own shader code (decision 0181).
//!
//! A game shader is a short piece of WGSL with a `surface` function (and optionally a
//! `vertex` function) that the engine splices into its PBR shader, so the result still goes
//! through the engine's lights, shadows, environment, fog, tone mapping and bloom. The
//! contract (types, bindings, options) is in `docs/ai/SHADERS.md`.
//!
//! Where the code comes from: [`ShaderMaterial::shader`] is either a name a module stored in
//! [`GameShaders`] (code shipped inside the game binary) or a `.wgsl` file relative to the
//! scene file.

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Most numbers a [`ShaderMaterial`] passes to its shader (`game.params`, 4 vec4s).
pub const MAX_SHADER_PARAMS: usize = 16;
/// Most images a [`ShaderMaterial`] binds (`tex0` to `tex3` in the shader).
pub const MAX_SHADER_TEXTURES: usize = 4;

/// Draws the entity's `Model` with a game shader. `Material` (if present) still sets the
/// base color, metallic and roughness the shader starts from.
#[derive(Component, Reflect, Debug, Clone, PartialEq, Default)]
pub struct ShaderMaterial {
    /// A name stored in `GameShaders`, or a `.wgsl` file relative to the scene file.
    pub shader: String,
    /// Up to 16 numbers, read in the shader as `game.params[i / 4][i % 4]`; missing
    /// ones are zero.
    pub params: Vec<f32>,
    /// Up to 4 images (PNG paths relative to the scene file, or `builtin:` images), bound as
    /// `tex0` to `tex3`; missing ones are white.
    pub textures: Vec<String>,
}

/// Shader sources stored by game code under names (decision 0181). Render data: not in
/// scenes or the state hash. Changing a source rebuilds its pipelines on the next frame.
#[derive(Resource, Debug, Clone, Default)]
pub struct GameShaders {
    map: Arc<BTreeMap<String, Arc<str>>>,
}

impl GameShaders {
    /// Stores WGSL `source` under `name` (replacing an earlier one).
    pub fn set(&mut self, name: &str, source: &str) {
        Arc::make_mut(&mut self.map).insert(name.to_string(), Arc::from(source));
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.map.contains_key(name) && Arc::make_mut(&mut self.map).remove(name).is_some()
    }

    pub fn get(&self, name: &str) -> Option<&Arc<str>> {
        self.map.get(name)
    }

    /// A cheap copy of every source for one frame.
    pub fn snapshot(&self) -> Arc<BTreeMap<String, Arc<str>>> {
        self.map.clone()
    }

    /// Convenience for modules: stores into the world's resource, creating it.
    pub fn set_in(world: &mut World, name: &str, source: &str) {
        world
            .get_resource_or_insert_with(GameShaders::default)
            .set(name, source);
    }
}

/// How a shader's result is combined with what is behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ShaderBlend {
    /// Writes depth and replaces the color (the default).
    #[default]
    Opaque,
    /// Mixed by `base_color.a`, drawn after opaque models without writing depth.
    Alpha,
    /// Added to what is behind (glows, holograms), drawn after opaque models.
    Additive,
}

/// Options a shader source declares in comment lines starting with `// @`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShaderOptions {
    /// `// @blend alpha` or `// @blend additive`.
    pub blend: ShaderBlend,
    /// `// @double_sided`: draw back faces too (leaves, water seen from below).
    pub double_sided: bool,
    /// `// @no_shadows`: does not cast shadows. Blended shaders never cast.
    pub no_shadows: bool,
    /// The source has a `vertex` function (found by name).
    pub has_vertex: bool,
    /// The source has a `surface` function.
    pub has_surface: bool,
}

impl ShaderOptions {
    /// Reads the options from `source`. Unknown `// @` words are errors, so a typo does not
    /// silently change nothing.
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut o = ShaderOptions::default();
        for (n, line) in source.lines().enumerate() {
            let Some(rest) = line.trim().strip_prefix("// @") else {
                continue;
            };
            let mut words = rest.split_whitespace();
            match (words.next(), words.next()) {
                (Some("blend"), Some("opaque")) => o.blend = ShaderBlend::Opaque,
                (Some("blend"), Some("alpha")) => o.blend = ShaderBlend::Alpha,
                (Some("blend"), Some("additive")) => o.blend = ShaderBlend::Additive,
                (Some("double_sided"), None) => o.double_sided = true,
                (Some("no_shadows"), None) => o.no_shadows = true,
                _ => {
                    return Err(format!(
                        "line {}: unknown option `// @{rest}` (use blend opaque|alpha|additive, double_sided, no_shadows)",
                        n + 1
                    ));
                }
            }
        }
        o.has_vertex = has_fn(source, "vertex");
        o.has_surface = has_fn(source, "surface");
        if !o.has_surface && !o.has_vertex {
            return Err("a game shader needs `fn surface(s: ptr<function, Surface>, i: SurfaceIn)` or `fn vertex(v: ptr<function, Vertex>)`".into());
        }
        Ok(o)
    }

    /// Whether the shader draws into shadow maps.
    pub fn casts_shadows(&self) -> bool {
        self.blend == ShaderBlend::Opaque && !self.no_shadows
    }
}

/// `fn <name>(` appears in `source` outside comments.
pub fn has_fn(source: &str, name: &str) -> bool {
    source.lines().any(|line| {
        let code = line.split("//").next().unwrap_or("");
        let mut rest = code;
        while let Some(at) = rest.find("fn ") {
            let after = rest[at + 3..].trim_start();
            if let Some(tail) = after.strip_prefix(name)
                && tail.trim_start().starts_with('(')
                && (at == 0 || !rest.as_bytes()[at - 1].is_ascii_alphanumeric())
            {
                return true;
            }
            rest = &rest[at + 3..];
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_come_from_comment_lines() {
        let src =
            "// @blend additive\n// @double_sided\nfn surface(s: ptr<function, Surface>, i: SurfaceIn) {}\n";
        let o = ShaderOptions::parse(src).unwrap();
        assert_eq!(o.blend, ShaderBlend::Additive);
        assert!(o.double_sided && !o.has_vertex && !o.casts_shadows());
        assert!(ShaderOptions::parse("// @blend glass\nfn surface() {}").is_err());
        assert!(ShaderOptions::parse("fn other() {}").is_err());
        let v = ShaderOptions::parse("fn vertex (v: ptr<function, Vertex>) {}").unwrap();
        assert!(v.has_vertex && v.casts_shadows());
        assert!(!has_fn("// fn surface(\nfn my_surface() {}", "surface"));
    }

    #[test]
    fn sources_are_stored_by_name() {
        let mut s = GameShaders::default();
        s.set("water", "fn surface() {}");
        let snap = s.snapshot();
        s.set("water", "fn surface() { }");
        assert_eq!(&*snap["water"], "fn surface() {}");
        assert!(s.remove("water") && !s.remove("water"));
    }
}
