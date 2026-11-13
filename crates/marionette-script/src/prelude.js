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
    playClip(clip, o = {}) {
      h.anim(this.id, JSON.stringify({ op: "play", clip: String(clip), fade: o.fade, speed: o.speed, looping: o.looping }));
    }
    blendClip(clip, weight, o = {}) {
      h.anim(this.id, JSON.stringify({ op: "blend", clip: String(clip), weight, fade: o.fade, speed: o.speed, looping: o.looping }));
    }
    stopClip(clip, o = {}) {
      h.anim(this.id, JSON.stringify({ op: "stop", clip: clip === undefined || clip === null ? undefined : String(clip), fade: o.fade }));
    }
    setBlend(space, x, y, o = {}) {
      h.anim(this.id, JSON.stringify({ op: "param", space: String(space), x, y: y ?? 0, rate: o.rate }));
    }
    playOverlay(name, clip, o = {}) {
      h.anim(this.id, JSON.stringify({ op: "overlay", name: String(name), clip: String(clip), weight: o.weight, mask: o.mask, additive: o.additive, fade: o.fade, speed: o.speed, looping: o.looping }));
    }
    stopOverlay(name, o = {}) {
      h.anim(this.id, JSON.stringify({ op: "overlay_stop", name: String(name), fade: o.fade }));
    }
    get name() {
      return h.name(this.id) ?? null;
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

  // Job arguments as the native side reads them (see `parse_job_args`).
  const jobRef = (mode) => (x, entity) => {
    const e = entity === undefined || entity === null ? -1 : idOf(entity);
    return Object.freeze(typeof x === "string" ? { mode, data: x, entity: e } : { mode, array: x });
  };
  const jobArg = (a) => (typeof a === "string" ? { mode: "write", data: a, entity: -1 } : ArrayBuffer.isView(a) ? { mode: "write", array: a } : a);
  const jobRun = (path, fn, args, options) => {
    if (args !== undefined && !Array.isArray(args)) throw new Error("jobts.args: job arguments are an array, in parameter order");
    const chunk = options && options.chunk !== undefined ? options.chunk : 0;
    return h.jobs_run(String(path), String(fn), (args ?? []).map(jobArg), chunk);
  };
  const jobs = Object.freeze({
    run: jobRun,
    sum: (path, fn, args, options) => jobRun(path, fn, args, options).reduce((a, b) => a + b, 0),
    get: (name, entity) => h.jobs_get(entity === undefined || entity === null ? -1 : idOf(entity), String(name)) ?? null,
    set: (name, values, entity) => h.jobs_set(entity === undefined || entity === null ? -1 : idOf(entity), String(name), values),
    read: jobRef("read"),
    whole: jobRef("whole"),
    write: jobRef("write"),
    chunkStart: Object.freeze({ mode: "chunk_start" }),
  });

  const engine = Object.freeze({
    jobs,
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
    // A missing detail must not be passed as `undefined` (the native side takes an optional
    // string and refuses an explicit undefined).
    emit: (kind, detail) => (detail === undefined ? h.emit(String(kind)) : h.emit(String(kind), JSON.stringify(detail))),
    log: (...parts) => h.log(parts.map((p) => (typeof p === "string" ? p : JSON.stringify(p))).join(" ")),
    entity: (id) => (h.exists(id) ? new Entity(id) : null),
    find: (name) => {
      const id = h.find(String(name));
      return id === undefined || id === null ? null : new Entity(id);
    },
    query: (name) => h.query(name).map((id) => new Entity(id)),
    resource: (name) => parse(h.resource(name)),
    raycast: (origin, direction, maxDistance, exclude) => {
      const v = (a) => [a.x, a.y, a.z ?? 0].map((c) => (typeof c === "number" ? c : 0));
      const hit = parse(h.raycast(v(origin), v(direction), maxDistance === undefined ? 1000 : maxDistance, exclude === undefined || exclude === null ? undefined : idOf(exclude)));
      if (hit) hit.entity = new Entity(hit.entity);
      return hit ?? null;
    },
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
    send: (target, kind, detail) => {
      if (!turn) throw new Error("script.no_turn: messages can only be sent while a hook runs");
      const id = idOf(target);
      if (!h.exists(id)) throw new Error(`script.no_entity: entity ${id} does not exist`);
      if (!h.has(id, "Script")) throw new Error(`script.no_script: entity ${id} has no script to receive messages`);
      const inbox = parse(h.get(id, "ScriptInbox"));
      const list = inbox ? JSON.parse(inbox.data) : [];
      list.push({ from: turn.id, kind: String(kind), detail: detail === undefined ? null : JSON.parse(JSON.stringify(detail)) });
      h.set(id, "ScriptInbox", JSON.stringify({ data: JSON.stringify(list) }));
    },
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
    turn = { mod, id, used: timersText !== undefined && timersText !== null, timers: timersText ? JSON.parse(timersText) : { next: 1, list: [] } };
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
      // Messages: taken out of the inbox first, so a throwing handler does not see them again.
      const inbox = parse(h.get(id, "ScriptInbox"));
      if (inbox && inbox.data !== "[]") {
        h.set(id, "ScriptInbox", JSON.stringify({ data: "[]" }));
        if (typeof mod.onMessage === "function") for (const m of JSON.parse(inbox.data)) mod.onMessage(self, m);
      }
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
    turn = { mod, id, used: false, timers: { next: 1, list: [] } };
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
