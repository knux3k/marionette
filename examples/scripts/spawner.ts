// Drops a crate every quarter second until it has dropped 40, then stops its timer.
// Shows engine.every / engine.cancel (timers), engine.spawn and engine.clone (spawning).
// Hooks and engine API: docs/ai/reference/marionette.d.ts.

const LIMIT = 40;

export function start(self: Entity): void {
  self.state.dropped = 0;
  self.state.gone = 0;
  engine.every(0.25, drop);
}

export function drop(self: Entity): void {
  const x = (engine.rand() - 0.5) * 6;
  const z = -3 + (engine.rand() - 0.5) * 4;
  const crate = engine.spawn(
    {
      Transform: {
        translation: { x, y: 6, z },
        rotation: { x: 0, y: 0, z: 0, w: 1 },
        scale: { x: 0.5, y: 0.5, z: 0.5 },
      },
      RigidBody: { kind: "Dynamic", linear_damping: 0, angular_damping: 0.1, gravity_scale: 1, lock_rotations: false },
      Collider: {
        shape: { Cuboid: { half_extents: { x: 0.5, y: 0.5, z: 0.5 } } },
        friction: 0.7,
        restitution: 0.2,
        density: 1,
        sensor: false,
      },
      Model: { asset: "primitive:cube" },
      Material: { base_color: { r: 0.9, g: 0.5 + engine.rand() * 0.4, b: 0.2, a: 1 }, metallic: 0, roughness: 0.8 },
      Script: { path: "scripts/crate.ts" },
    },
    "crate",
  );
  self.state.dropped += 1;
  // Every tenth drop is a copy of the previous crate, higher up.
  if (self.state.dropped % 10 === 0) {
    const t = crate.get("Transform")!;
    t.translation.y = 9;
    engine.clone(crate, { Transform: t });
  }
  if (self.state.dropped >= LIMIT) engine.cancel(drop);
}

export function update(self: Entity): void {}
