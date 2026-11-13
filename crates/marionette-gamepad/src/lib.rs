//! Gamepads for windowed play (decision 0172).
//!
//! [`Gamepads::poll`] drains the OS controller events and returns device inputs in the action
//! map format: `pad:<Button>` (value 0..1, analog for triggers) and `axis:<Axis>` (-1..1, stick
//! up and right are +). Names are gilrs's (`pad:South`, `pad:LeftTrigger2`, `pad:DPadUp`,
//! `axis:LeftStickX`, `axis:RightStickY`). All connected pads feed the same names (one local
//! player); a game that needs per-pad players reads [`Gamepads::poll_by_pad`].
//!
//! This crate knows nothing about the simulation: the caller queues the inputs for the next
//! tick, exactly like keys, so recordings replay without a controller.

use gilrs::{EventType, Gilrs};
use std::collections::BTreeMap;

/// Axis values are rounded to this step so stick noise does not flood the input queue.
pub const AXIS_STEP: f32 = 1.0 / 256.0;

/// Connected controllers.
pub struct Gamepads {
    gilrs: Gilrs,
    /// Last value sent per (pad, input), so repeats are dropped and focus loss can release.
    held: BTreeMap<(usize, String), f32>,
}

/// One input change from one controller.
#[derive(Debug, Clone, PartialEq)]
pub struct PadInput {
    pub pad: usize,
    pub input: String,
    pub value: f32,
}

impl Gamepads {
    /// Opens the controller backend. Fails (with a message) when the OS backend is missing,
    /// for example no udev on Linux; windowed play then runs without gamepads.
    pub fn new() -> Result<Self, String> {
        let gilrs = match Gilrs::new() {
            Ok(g) => g,
            Err(gilrs::Error::NotImplemented(g)) => g,
            Err(e) => return Err(format!("gamepads unavailable: {e}")),
        };
        Ok(Self {
            gilrs,
            held: BTreeMap::new(),
        })
    }

    /// Names of the controllers connected now.
    pub fn connected(&self) -> Vec<String> {
        self.gilrs.gamepads().map(|(_, g)| g.name().to_string()).collect()
    }

    /// Every change since the last call, per controller.
    pub fn poll_by_pad(&mut self) -> Vec<PadInput> {
        let mut out = Vec::new();
        while let Some(ev) = self.gilrs.next_event() {
            let pad = usize::from(ev.id);
            let (input, value) = match ev.event {
                EventType::ButtonChanged(b, v, _) => (format!("pad:{b:?}"), v.clamp(0.0, 1.0)),
                EventType::AxisChanged(a, v, _) => (format!("axis:{a:?}"), quantize(v)),
                EventType::Disconnected => {
                    out.extend(self.release_pad(Some(pad)));
                    continue;
                }
                _ => continue,
            };
            if input.ends_with("Unknown") {
                continue;
            }
            let last = self.held.get(&(pad, input.clone())).copied().unwrap_or(0.0);
            if last == value {
                continue;
            }
            if value == 0.0 {
                self.held.remove(&(pad, input.clone()));
            } else {
                self.held.insert((pad, input.clone()), value);
            }
            out.push(PadInput { pad, input, value });
        }
        out
    }

    /// Every change since the last call, all controllers merged (one local player): per input
    /// name the value of the controller that changed last.
    pub fn poll(&mut self) -> Vec<(String, f32)> {
        merge(self.poll_by_pad())
    }

    /// Sets every held button and axis back to 0 (window lost focus, pad unplugged).
    pub fn release_all(&mut self) -> Vec<(String, f32)> {
        merge(self.release_pad(None))
    }

    fn release_pad(&mut self, pad: Option<usize>) -> Vec<PadInput> {
        let keys: Vec<_> = self
            .held
            .keys()
            .filter(|(p, _)| pad.is_none_or(|x| x == *p))
            .cloned()
            .collect();
        keys.into_iter()
            .map(|k| {
                self.held.remove(&k);
                PadInput {
                    pad: k.0,
                    input: k.1,
                    value: 0.0,
                }
            })
            .collect()
    }
}

/// Rounds an axis value to [`AXIS_STEP`] and clamps it to -1..1.
pub fn quantize(v: f32) -> f32 {
    ((v / AXIS_STEP).round() * AXIS_STEP).clamp(-1.0, 1.0)
}

/// Keeps the last change per input name, in first-seen order.
fn merge(inputs: Vec<PadInput>) -> Vec<(String, f32)> {
    let mut out: Vec<(String, f32)> = Vec::new();
    for i in inputs {
        match out.iter_mut().find(|(n, _)| *n == i.input) {
            Some(slot) => slot.1 = i.value,
            None => out.push((i.input, i.value)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_values_are_quantized_and_clamped() {
        assert_eq!(quantize(0.0), 0.0);
        assert_eq!(quantize(0.001), 0.0);
        assert_eq!(quantize(0.5), 0.5);
        assert_eq!(quantize(1.7), 1.0);
        assert_eq!(quantize(-1.2), -1.0);
    }

    #[test]
    fn merge_keeps_the_last_value_per_name() {
        let p = |pad, input: &str, value| PadInput {
            pad,
            input: input.into(),
            value,
        };
        let merged = merge(vec![
            p(0, "axis:LeftStickX", 0.5),
            p(0, "pad:South", 1.0),
            p(1, "axis:LeftStickX", -0.25),
        ]);
        assert_eq!(
            merged,
            vec![("axis:LeftStickX".into(), -0.25), ("pad:South".into(), 1.0)]
        );
    }
}
