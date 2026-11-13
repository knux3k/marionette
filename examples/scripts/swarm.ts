// Fills the swarm's job arrays on the first tick (engine.jobs.set, then the job function
// `seed`) and reports the swarm's kinetic energy once a second (engine.jobs.sum). The
// per-tick motion is the `step` job in the entity's JobSchedule, not script code.
// Engine API: docs/ai/reference/marionette.d.ts; job files: docs/ai/JOBS-TS.md.

const COUNT = 4096;
const JOB = "jobs/swarm.job.ts";
const ARRAYS = ["px", "py", "pz", "vx", "vy", "vz"];

export function start(self: Entity): void {
  for (const name of ARRAYS) engine.jobs.set(name, new Float32Array(COUNT));
  engine.jobs.run(JOB, "seed", [...ARRAYS, engine.jobs.chunkStart, 7]);
}

export function update(self: Entity): void {
  if (engine.tick % 60 !== 0) return;
  const e = engine.jobs.sum(JOB, "energy", [engine.jobs.read("vx"), engine.jobs.read("vy"), engine.jobs.read("vz")]);
  engine.emit("swarm_energy", { energy: Math.round(e) });
}
