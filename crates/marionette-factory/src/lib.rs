//! Factory simulation: machines that turn items into items, and belts that carry items
//! between them, for games with very large numbers of both (factory builders, automation).
//!
//! Authoring is ordinary scene data: [`FactoryMachine`] and [`FactoryBelt`] components and
//! a [`FactoryRecipes`] resource, edited through operations like anything else. At load
//! the module compiles them into flat arrays ([`FactoryState`]) and each tick runs three
//! phases over those arrays, never one script call per machine or item:
//!
//! 1. machines craft (data-parallel over machines);
//! 2. items advance along belts, downstream slots first (data-parallel over belts);
//! 3. belt ends hand items to machines and machines put items on belts (sequential, in
//!    belt order, because two belts can share a machine).
//!
//! Every number is an integer (time in thousandths of a tick), so results are exact on
//! every platform, and the parallel phases only write their own slice
//! (`marionette_jobs`, decision 0024), so the thread count never changes the state hash.
//!
//! [`FactoryState`] is a registered resource: it is saved in the scene, restored by
//! snapshots and part of the state hash. Items on belts are drawn in bulk by a render hook
//! (no entity per item).

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::builtin::SceneRef;
use marionette_core::module::Stage;
use marionette_core::scene::SceneId;
use marionette_core::spatial::{BulkDraw, BulkDrawHooks, Color, Material, Transform};
use marionette_core::{Diagnostic, Module, Registry, Result};
use marionette_jobs::Jobs;
use std::collections::BTreeMap;

/// Inputs a recipe can have.
pub const MAX_INPUTS: usize = 4;
/// Machines per job chunk in the crafting phase.
const MACHINE_CHUNK: usize = 1024;
/// Belts per job segment in the belt phase (segments are cut at belt boundaries).
const BELTS_PER_SEGMENT: usize = 64;

/// Stable error codes.
pub mod codes {
    pub const UNKNOWN_RECIPE: &str = "factory.unknown_recipe";
    pub const BAD_RECIPE: &str = "factory.bad_recipe";
    pub const BAD_BELT: &str = "factory.bad_belt";
}

/// A count of one item.
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Default)]
pub struct ItemAmount {
    pub item: String,
    pub count: u32,
}

/// What a machine makes. A recipe with no inputs is a source (a miner, a pump).
#[derive(Reflect, Debug, Clone, PartialEq, Eq, Default)]
pub struct Recipe {
    pub id: String,
    /// Up to four inputs, consumed when a cycle starts.
    pub inputs: Vec<ItemAmount>,
    /// Produced when a cycle ends. A count of 0 makes a consumer (a lab, a sink): it uses
    /// its inputs and produces nothing, so its line never backs up.
    pub output: ItemAmount,
    /// Length of one cycle in ticks (at least 1).
    pub ticks: u32,
}

/// How an item looks on a belt.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct ItemLook {
    pub item: String,
    /// Model asset (default `primitive:cube`).
    pub model: String,
    pub color: Color,
    /// Uniform scale of the model.
    pub scale: f32,
}

/// Every recipe and item look of the scene.
#[derive(Resource, Reflect, Debug, Clone, PartialEq, Default)]
pub struct FactoryRecipes {
    pub recipes: Vec<Recipe>,
    /// Optional looks; items without one are small grey cubes.
    pub looks: Vec<ItemLook>,
}

/// A machine running one recipe. Its position (for belts and drawing) is its `Transform`.
#[derive(Component, Reflect, Debug, Clone, PartialEq, Eq, Default)]
pub struct FactoryMachine {
    /// Recipe id from `FactoryRecipes`.
    pub recipe: String,
    /// Most items held per input, and in the output.
    pub buffer: u32,
}

/// A belt from one machine's output to another machine's input.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FactoryBelt {
    pub from: SceneRef,
    pub to: SceneRef,
    /// Number of item slots (one item each); at least 1.
    pub slots: u32,
    /// Thousandths of a tick an item waits in a slot before it may hop to the next
    /// (1000 = one slot per tick, the fastest a belt can run).
    pub step_milliticks: u32,
}

/// One machine's live state.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MachineRow {
    /// Stable scene id of the machine entity.
    pub entity: u64,
    /// Index into the recipe table.
    pub recipe: u32,
    /// Items held per recipe input.
    pub have: [u32; MAX_INPUTS],
    /// Finished items waiting to leave.
    pub output: u32,
    /// Ticks into the current cycle.
    pub progress: u32,
    /// 1 while a cycle runs.
    pub running: u32,
    pub buffer: u32,
}

/// One belt's place in the slot array.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BeltRow {
    pub entity: u64,
    /// Machine indices (into `FactoryState::machines`).
    pub from: u32,
    pub to: u32,
    /// First slot in `FactoryState::slots`.
    pub first: u32,
    pub len: u32,
    pub step: u32,
}

/// One belt slot.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    /// Item index, -1 when empty.
    pub item: i32,
    /// When the item arrived, in thousandths of a tick.
    pub arrived: i64,
}

impl Default for Slot {
    fn default() -> Self {
        Self { item: -1, arrived: 0 }
    }
}

/// The compiled factory: flat arrays stepped every tick. Saved in the scene and hashed.
#[derive(Resource, Reflect, Debug, Clone, PartialEq, Default)]
pub struct FactoryState {
    /// Fingerprint of the authoring data these arrays were built from. A scene whose
    /// machines, belts or recipes changed gets fresh arrays at load.
    pub layout: u64,
    /// Ticks the factory has run.
    pub now: i64,
    /// Machines in scene-id order.
    pub machines: Vec<MachineRow>,
    /// Belts in scene-id order; their slots are consecutive in `slots`.
    pub belts: Vec<BeltRow>,
    pub slots: Vec<Slot>,
    /// Items finished by machines, ever.
    pub made: u64,
    /// Items delivered from belts into machines, ever.
    pub delivered: u64,
}

/// A recipe compiled to item indices.
#[derive(Debug, Clone, Copy, Default)]
struct CRecipe {
    inputs: [(u32, u32); MAX_INPUTS],
    n_inputs: usize,
    output: u32,
    out_count: u32,
    ticks: u32,
}

/// Read-only tables built at load (not state: rebuilt from authoring data every load).
#[derive(Resource, Debug, Clone, Default)]
pub struct FactoryTables {
    recipes: Vec<CRecipe>,
    /// Item names, sorted (index = item id).
    pub items: Vec<String>,
    looks: Vec<(String, Material, f32)>,
    /// Belt end points in world space, per belt.
    ends: Vec<([f32; 3], [f32; 3])>,
    /// Slot index where each belt segment ends (for the parallel belt phase).
    segment_ends: Vec<usize>,
    /// Belt index range per segment.
    segment_belts: Vec<(usize, usize)>,
}

pub struct FactoryModule;

impl Module for FactoryModule {
    fn name(&self) -> &'static str {
        "factory"
    }

    fn register(&self, r: &mut Registry) {
        r.register_component::<FactoryMachine>();
        r.register_component::<FactoryBelt>();
        r.register_resource::<FactoryRecipes>();
        r.register_resource::<FactoryState>();
    }

    fn add_systems(&self, schedule: &mut Schedule) {
        schedule.add_systems(factory_tick.in_set(Stage::Logic));
    }

    fn init(&self, world: &mut World) -> Result<()> {
        let has_machines = world
            .try_query::<&FactoryMachine>()
            .is_some_and(|mut q| q.iter(world).next().is_some());
        if !has_machines && !world.contains_resource::<FactoryRecipes>() {
            return Ok(()); // not a factory scene
        }
        if !world.contains_resource::<Jobs>() {
            world.insert_resource(Jobs::shared());
        }
        let (tables, fresh) = compile(world)?;
        let keep = world
            .get_resource::<FactoryState>()
            .is_some_and(|s| s.layout == fresh.layout);
        if !keep {
            world.insert_resource(fresh);
        }
        world.insert_resource(tables);
        BulkDrawHooks::add(world, draw_items);
        Ok(())
    }
}

/// FNV-1a, for the layout fingerprint.
struct Fnv(u64);
impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 ^= *x as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        self.bytes(s.as_bytes());
    }
}

/// Builds the tables and a fresh state from the authoring components.
fn compile(world: &mut World) -> Result<(FactoryTables, FactoryState)> {
    let recipes = world
        .get_resource::<FactoryRecipes>()
        .cloned()
        .unwrap_or_default();
    let mut fp = Fnv::new();

    // Items: every name used by any recipe, sorted.
    let mut names: Vec<String> = recipes
        .recipes
        .iter()
        .flat_map(|r| {
            r.inputs
                .iter()
                .map(|a| a.item.clone())
                .chain([r.output.item.clone()])
        })
        .collect();
    names.sort();
    names.dedup();
    let item_id = |name: &str| {
        names
            .binary_search_by(|n| n.as_str().cmp(name))
            .ok()
            .map(|i| i as u32)
    };

    let mut by_id = BTreeMap::new();
    let mut compiled = Vec::with_capacity(recipes.recipes.len());
    for (i, r) in recipes.recipes.iter().enumerate() {
        if r.inputs.len() > MAX_INPUTS || r.ticks == 0 || (r.inputs.is_empty() && r.output.count == 0) {
            return Err(Diagnostic::error(
                codes::BAD_RECIPE,
                format!(
                    "recipe `{}` needs 0..={MAX_INPUTS} inputs, ticks >= 1, and an output unless it has inputs",
                    r.id
                ),
            )
            .component("FactoryRecipes")
            .into());
        }
        let mut c = CRecipe {
            n_inputs: r.inputs.len(),
            output: item_id(&r.output.item).expect("collected above"),
            out_count: r.output.count,
            ticks: r.ticks,
            ..Default::default()
        };
        for (k, a) in r.inputs.iter().enumerate() {
            c.inputs[k] = (item_id(&a.item).expect("collected above"), a.count.max(1));
        }
        fp.str(&r.id);
        fp.u64(r.ticks as u64);
        fp.str(&r.output.item);
        fp.u64(r.output.count as u64);
        for a in &r.inputs {
            fp.str(&a.item);
            fp.u64(a.count as u64);
        }
        by_id.insert(r.id.clone(), i as u32);
        compiled.push(c);
    }

    // Machines in scene-id order.
    let mut machines: Vec<(u64, FactoryMachine, [f32; 3])> = world
        .query::<(&SceneId, &FactoryMachine, Option<&Transform>)>()
        .iter(world)
        .map(|(id, m, t)| {
            let p = t.map_or([0.0; 3], |t| [t.translation.x, t.translation.y, t.translation.z]);
            (id.0, m.clone(), p)
        })
        .collect();
    machines.sort_by_key(|(id, _, _)| *id);
    let mut index = BTreeMap::new();
    let mut rows = Vec::with_capacity(machines.len());
    let mut positions = Vec::with_capacity(machines.len());
    for (i, (id, m, p)) in machines.iter().enumerate() {
        let Some(&recipe) = by_id.get(&m.recipe) else {
            return Err(
                Diagnostic::error(codes::UNKNOWN_RECIPE, format!("unknown recipe `{}`", m.recipe))
                    .entity(*id)
                    .component("FactoryMachine")
                    .fix("add the recipe to the FactoryRecipes resource or use one of its ids")
                    .into(),
            );
        };
        fp.u64(*id);
        fp.str(&m.recipe);
        fp.u64(m.buffer as u64);
        index.insert(*id, i as u32);
        positions.push(*p);
        rows.push(MachineRow {
            entity: *id,
            recipe,
            buffer: m.buffer.max(1),
            ..Default::default()
        });
    }

    // Belts in scene-id order, slots laid out consecutively.
    let mut belts: Vec<(u64, FactoryBelt)> = world
        .query::<(&SceneId, &FactoryBelt)>()
        .iter(world)
        .map(|(id, b)| (id.0, *b))
        .collect();
    belts.sort_by_key(|(id, _)| *id);
    let mut belt_rows = Vec::with_capacity(belts.len());
    let mut ends = Vec::with_capacity(belts.len());
    let mut first = 0u32;
    for (id, b) in &belts {
        let (Some(&from), Some(&to)) = (index.get(&b.from.0), index.get(&b.to.0)) else {
            return Err(
                Diagnostic::error(codes::BAD_BELT, "belt ends must be FactoryMachine entities")
                    .entity(*id)
                    .component("FactoryBelt")
                    .fix("set `from` and `to` to ids of entities with a FactoryMachine")
                    .into(),
            );
        };
        let len = b.slots.max(1);
        fp.u64(*id);
        fp.u64(b.from.0);
        fp.u64(b.to.0);
        fp.u64(len as u64);
        fp.u64(b.step_milliticks as u64);
        belt_rows.push(BeltRow {
            entity: *id,
            from,
            to,
            first,
            len,
            step: b.step_milliticks.max(1000),
        });
        ends.push((positions[from as usize], positions[to as usize]));
        first += len;
    }

    let mut segment_ends = Vec::new();
    let mut segment_belts = Vec::new();
    for (k, group) in belt_rows.chunks(BELTS_PER_SEGMENT).enumerate() {
        let last = group.last().expect("chunks are non-empty");
        segment_ends.push((last.first + last.len) as usize);
        segment_belts.push((k * BELTS_PER_SEGMENT, k * BELTS_PER_SEGMENT + group.len()));
    }

    let looks = names
        .iter()
        .map(|n| {
            let look = recipes.looks.iter().find(|l| &l.item == n);
            let model = look.map_or("primitive:cube".to_string(), |l| l.model.clone());
            let color = look.map_or(
                Color {
                    r: 0.6,
                    g: 0.6,
                    b: 0.6,
                    a: 1.0,
                },
                |l| l.color,
            );
            let scale = look.map_or(0.2, |l| l.scale);
            (
                model,
                Material {
                    base_color: color,
                    metallic: 0.0,
                    roughness: 0.6,
                },
                scale,
            )
        })
        .collect();

    let state = FactoryState {
        layout: fp.0,
        machines: rows,
        slots: vec![Slot::default(); first as usize],
        belts: belt_rows,
        ..Default::default()
    };
    let tables = FactoryTables {
        recipes: compiled,
        items: names,
        looks,
        ends,
        segment_ends,
        segment_belts,
    };
    Ok((tables, state))
}

/// One tick of the factory (see the module docs for the three phases).
fn factory_tick(
    jobs: Option<Res<Jobs>>,
    tables: Option<Res<FactoryTables>>,
    state: Option<ResMut<FactoryState>>,
) {
    let (Some(tables), Some(mut state)) = (tables, state) else {
        return;
    };
    let inline = Jobs::inline();
    let jobs = jobs.as_deref().unwrap_or(&inline);
    step(jobs, &tables, &mut state);
}

/// Advances `state` by one tick. Public for benchmarks and tests.
pub fn step(jobs: &Jobs, tables: &FactoryTables, state: &mut FactoryState) {
    state.now += 1;
    let now_m = state.now * 1000;
    let FactoryState {
        machines,
        belts,
        slots,
        made,
        delivered,
        ..
    } = state;

    // 1. Machines craft. Each chunk counts what it finished; counts are added in order.
    let recipes = &tables.recipes;
    let finished = std::sync::Mutex::new(vec![0u64; machines.len().div_ceil(MACHINE_CHUNK)]);
    jobs.for_each_chunk_mut(machines, MACHINE_CHUNK, |c, chunk| {
        let mut n = 0u64;
        for m in chunk {
            n += craft(m, &recipes[m.recipe as usize]);
        }
        finished.lock().expect("no panics while held")[c] = n;
    });
    *made += finished
        .into_inner()
        .expect("no panics while held")
        .iter()
        .sum::<u64>();

    // 2. Items advance inside each belt, downstream first, so a full line moves together.
    let belt_rows: &[BeltRow] = belts;
    jobs.for_each_segment_mut(slots, &tables.segment_ends, |seg, seg_slots| {
        let (b0, b1) = tables.segment_belts[seg];
        let base = belt_rows[b0].first as usize;
        for b in &belt_rows[b0..b1] {
            let s = &mut seg_slots[b.first as usize - base..(b.first + b.len) as usize - base];
            for k in (0..s.len().saturating_sub(1)).rev() {
                if s[k].item >= 0 && s[k + 1].item < 0 && now_m - s[k].arrived >= b.step as i64 {
                    s[k + 1] = Slot {
                        item: s[k].item,
                        arrived: now_m,
                    };
                    s[k] = Slot::default();
                }
            }
        }
    });

    // 3. Hand-offs between belts and machines, in belt order.
    for b in belt_rows {
        let last = (b.first + b.len - 1) as usize;
        let s = slots[last];
        if s.item >= 0 && now_m - s.arrived >= b.step as i64 {
            let m = &mut machines[b.to as usize];
            let r = &recipes[m.recipe as usize];
            if let Some(k) = (0..r.n_inputs).find(|&k| r.inputs[k].0 == s.item as u32 && m.have[k] < m.buffer)
            {
                m.have[k] += 1;
                slots[last] = Slot::default();
                *delivered += 1;
            }
        }
        let first = b.first as usize;
        let src = &mut machines[b.from as usize];
        if src.output > 0 && slots[first].item < 0 {
            src.output -= 1;
            slots[first] = Slot {
                item: recipes[src.recipe as usize].output as i32,
                arrived: now_m,
            };
        }
    }
}

/// One machine's tick; returns how many items it finished.
#[inline]
fn craft(m: &mut MachineRow, r: &CRecipe) -> u64 {
    let mut made = 0;
    if r.n_inputs == 0 {
        // A source: works while there is room for what it makes.
        if m.output + r.out_count <= m.buffer {
            m.progress += 1;
            if m.progress >= r.ticks {
                m.progress = 0;
                m.output += r.out_count;
                made += r.out_count as u64;
            }
        }
        return made;
    }
    if m.running != 0 {
        m.progress += 1;
        if m.progress >= r.ticks {
            m.output += r.out_count;
            made += r.out_count as u64;
            m.running = 0;
            m.progress = 0;
        }
    }
    if m.running == 0
        && m.output + r.out_count <= m.buffer
        && (0..r.n_inputs).all(|k| m.have[k] >= r.inputs[k].1)
    {
        for k in 0..r.n_inputs {
            m.have[k] -= r.inputs[k].1;
        }
        m.running = 1;
    }
    made
}

/// Render hook: every item on a belt, one bulk draw per item kind, placed between its slot
/// and the previous one by how far it has travelled (smooth at any display rate).
fn draw_items(world: &World, alpha: f32, out: &mut Vec<BulkDraw>) {
    let (Some(tables), Some(state)) = (
        world.get_resource::<FactoryTables>(),
        world.get_resource::<FactoryState>(),
    ) else {
        return;
    };
    let mut per_item: Vec<Vec<[f32; 12]>> = vec![Vec::new(); tables.items.len()];
    let now = (state.now as f32 + alpha) * 1000.0;
    for (b, (a, z)) in state.belts.iter().zip(&tables.ends) {
        let len = b.len as f32;
        for k in 0..b.len as usize {
            let s = state.slots[b.first as usize + k];
            if s.item < 0 {
                continue;
            }
            let t = ((now - s.arrived as f32) / b.step as f32).clamp(0.0, 1.0);
            let f = (k as f32 + t) / len;
            let p = [
                a[0] + (z[0] - a[0]) * f,
                a[1] + (z[1] - a[1]) * f + 0.5,
                a[2] + (z[2] - a[2]) * f,
            ];
            let sc = tables.looks[s.item as usize].2;
            per_item[s.item as usize].push([sc, 0.0, 0.0, 0.0, sc, 0.0, 0.0, 0.0, sc, p[0], p[1], p[2]]);
        }
    }
    for (i, transforms) in per_item.into_iter().enumerate() {
        if transforms.is_empty() {
            continue;
        }
        let (model, material, _) = &tables.looks[i];
        out.push(BulkDraw {
            asset: model.clone(),
            material: Some(*material),
            transforms,
        });
    }
}
