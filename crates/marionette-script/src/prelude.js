// Runs once per script host, before any game script. Wraps the native `__host` functions
// into the `engine` object and the `Entity` class described by `marionette.d.ts`.
"use strict";
(() => {
  const h = globalThis.__host;
  delete globalThis.__host;
  const parse = (t) => (t === undefined || t === null ? undefined : JSON.parse(t));

  class Entity {
    constructor(id) {
      this.id = id;
      this.state = {};
    }
    get(name) {
      return parse(h.get(this.id, name));
    }
    set(name, value) {
      h.set(this.id, name, JSON.stringify(value));
    }
    has(name) {
      return h.has(this.id, name);
    }
  }
  Object.freeze(Entity.prototype);

  const engine = Object.freeze({
    get tick() {
      return h.tick();
    },
    get dt() {
      return h.dt();
    },
    rand: () => h.rand(),
    action: (name) => h.action(name),
    pressed: (name) => h.pressed(name),
    justPressed: (name) => h.just_pressed(name),
    emit: (kind, detail) => h.emit(String(kind), detail === undefined ? undefined : JSON.stringify(detail)),
    log: (...parts) => h.log(parts.map((p) => (typeof p === "string" ? p : JSON.stringify(p))).join(" ")),
    entity: (id) => (h.exists(id) ? new Entity(id) : null),
    find: (name) => {
      const id = h.find(String(name));
      return id === undefined || id === null ? null : new Entity(id);
    },
    query: (name) => h.query(name).map((id) => new Entity(id)),
    resource: (name) => parse(h.resource(name)),
  });

  const define = (name, value) =>
    Object.defineProperty(globalThis, name, { value, writable: false, enumerable: false, configurable: false });
  define("engine", engine);
  define("console", Object.freeze({ log: engine.log, warn: engine.log, error: engine.log }));
  // One entity's turn: start (first time only), contact hooks, then update.
  const each = (fn, self, others) => {
    if (typeof fn === "function") for (const o of others) fn(self, new Entity(o));
  };
  define("__run", (mod, id, stateText, isNew, c) => {
    const self = new Entity(id);
    self.state = JSON.parse(stateText);
    if (isNew && typeof mod.start === "function") mod.start(self);
    each(mod.onCollisionEnter, self, c.collisionEnter);
    each(mod.onCollisionExit, self, c.collisionExit);
    each(mod.onTriggerEnter, self, c.triggerEnter);
    each(mod.onTriggerExit, self, c.triggerExit);
    mod.update(self);
    return JSON.stringify(self.state ?? {});
  });
})();
