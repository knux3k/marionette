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
    destroy(seconds) {
      engine.destroy(this, seconds);
    }
  }
  Object.freeze(Entity.prototype);

  // Seconds to whole ticks (at least 1: "after 0 seconds" means the next tick).
  const ticksOf = (seconds, what) => {
    if (typeof seconds !== "number" || !(seconds >= 0) || seconds === Infinity)
      throw new Error(`script.bad_value: ${what} must be a finite number of seconds >= 0, got ${seconds}`);
    const t = Math.round(seconds / h.dt());
    return t < 1 ? 1 : t;
  };
  const idOf = (e) => (typeof e === "number" ? e : e && typeof e.id === "number" ? e.id : NaN);
  const json = (v) => JSON.stringify(v === undefined ? {} : v);

  // Timers of the entity whose turn it is: { next, list: [{ id, at, every, call, arg }] }.
  let turn = null;
  const timerName = (fn) => {
    if (!turn) throw new Error("script.no_turn: timers can only be set while a hook runs");
    const name = typeof fn === "function" ? fn.name : fn;
    if (typeof name !== "string" || typeof turn.mod[name] !== "function" || (typeof fn === "function" && turn.mod[name] !== fn))
      throw new Error(`script.timer_function: timers call an exported function of this script; \`${String(name)}\` is not one`);
    return name;
  };
  const schedule = (delay, every, fn, arg) => {
    const call = timerName(fn);
    const t = turn.timers;
    const id = t.next++;
    t.list.push({ id, at: h.tick() + delay, every, call, arg: arg === undefined ? null : JSON.parse(JSON.stringify(arg)) });
    turn.used = true;
    return id;
  };
  const matches = (t, which) =>
    which === undefined || (typeof which === "number" ? t.id === which : t.call === (typeof which === "function" ? which.name : which));

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
    spawn: (components, name) => new Entity(h.spawn(json(components), name === undefined ? undefined : String(name))),
    clone: (source, overrides, name) =>
      new Entity(h.clone(idOf(source), json(overrides), name === undefined ? undefined : String(name))),
    destroy: (e, seconds) => h.destroy(idOf(e), seconds === undefined || seconds === 0 ? 0 : ticksOf(seconds, "destroy delay")),
    after: (seconds, fn, arg) => schedule(ticksOf(seconds, "after"), 0, fn, arg),
    afterTicks: (ticks, fn, arg) => {
      if (!Number.isInteger(ticks) || ticks < 1) throw new Error(`script.bad_value: ticks must be a whole number >= 1, got ${ticks}`);
      return schedule(ticks, 0, fn, arg);
    },
    every: (seconds, fn, arg, firstSeconds) => {
      const every = ticksOf(seconds, "every");
      return schedule(firstSeconds === undefined ? every : ticksOf(firstSeconds, "first delay"), every, fn, arg);
    },
    cancel: (which) => {
      if (!turn) throw new Error("script.no_turn: timers can only be cancelled while a hook runs");
      const before = turn.timers.list.length;
      turn.timers.list = turn.timers.list.filter((t) => !matches(t, which));
      if (turn.timers.list.length !== before) turn.used = true;
      return before - turn.timers.list.length;
    },
    pending: (which) => (turn ? turn.timers.list.filter((t) => matches(t, which)).length : 0),
  });

  const define = (name, value) =>
    Object.defineProperty(globalThis, name, { value, writable: false, enumerable: false, configurable: false });
  define("engine", engine);
  define("console", Object.freeze({ log: engine.log, warn: engine.log, error: engine.log }));
  // One entity's turn: start (first time only), contact hooks, then update.
  const each = (fn, self, others) => {
    if (typeof fn === "function") for (const o of others) fn(self, new Entity(o));
  };
  const errorText = (e) =>
    e instanceof Error ? (e.stack ? `${e.message}\n${e.stack.trimEnd()}` : e.message) : JSON.stringify(e) ?? String(e);
  // Timers due this tick, in (tick, id) order. A one-shot timer is removed before its
  // call; a repeating one is moved to its next tick first. Cancelled ones are skipped.
  const fire = (mod, self) => {
    const now = h.tick();
    const due = turn.timers.list.filter((t) => t.at <= now).sort((a, b) => a.at - b.at || a.id - b.id);
    for (const t of due) {
      if (!turn.timers.list.includes(t)) continue;
      turn.used = true;
      if (t.every > 0) t.at = now + t.every;
      else turn.timers.list = turn.timers.list.filter((x) => x !== t);
      const f = mod[t.call];
      if (typeof f !== "function") throw new Error(`script.timer_function: \`${t.call}\` is no longer exported; its timer was dropped`);
      f(self, t.arg);
    }
  };
  // Returns [state text | null, timers text | null, error text | null].
  define("__run", (mod, id, stateText, isNew, c, timersText) => {
    turn = { mod, used: timersText !== undefined && timersText !== null, timers: timersText ? JSON.parse(timersText) : { next: 1, list: [] } };
    let state = null;
    let error = null;
    try {
      const self = new Entity(id);
      self.state = JSON.parse(stateText);
      if (isNew && typeof mod.start === "function") mod.start(self);
      each(mod.onCollisionEnter, self, c.collisionEnter);
      each(mod.onCollisionExit, self, c.collisionExit);
      each(mod.onTriggerEnter, self, c.triggerEnter);
      each(mod.onTriggerExit, self, c.triggerExit);
      fire(mod, self);
      mod.update(self);
      state = JSON.stringify(self.state ?? {});
    } catch (e) {
      error = errorText(e);
    }
    const timers = turn.used ? JSON.stringify(turn.timers) : null;
    turn = null;
    return [state, timers, error];
  });
  // `onDestroy` before the entity is removed. Timers set here are dropped with the entity.
  define("__destroy", (mod, id, stateText) => {
    turn = { mod, used: false, timers: { next: 1, list: [] } };
    let error = null;
    try {
      const self = new Entity(id);
      self.state = JSON.parse(stateText);
      if (typeof mod.onDestroy === "function") mod.onDestroy(self);
    } catch (e) {
      error = errorText(e);
    }
    turn = null;
    return [null, null, error];
  });
})();
