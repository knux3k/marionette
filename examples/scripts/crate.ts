// A dropped crate: reports its first landing and removes itself three seconds after it
// starts (engine.destroy with a delay, then onDestroy).

export function start(self: Entity): void {
  self.state.landed = false;
  self.destroy(3);
}

export function onCollisionEnter(self: Entity, other: Entity): void {
  if (!self.state.landed) {
    self.state.landed = true;
    engine.emit("landed", { on: other.id, tick: engine.tick });
  }
}

export function update(self: Entity): void {}

export function onDestroy(self: Entity): void {
  engine.emit("crate_gone", { landed: self.state.landed });
}
