use std::cmp::Ordering;
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

/// Physical speed of light in meters per second.
///
/// Minkowski-KV typically overrides this with a smaller value for interactive simulations.
pub const DEFAULT_C: f64 = 299_792_458.0;

static SPEED_OF_LIGHT: OnceLock<RwLock<f64>> = OnceLock::new();

fn c_cell() -> &'static RwLock<f64> {
    SPEED_OF_LIGHT.get_or_init(|| RwLock::new(DEFAULT_C))
}

/// Returns the currently configured speed of light (m/s).
///
/// This is a *simulation parameter* rather than a compile-time constant so test scenarios can
/// compress astronomical distances into human-scale delays.
pub fn speed_of_light() -> f64 {
    *c_cell().read().expect("speed_of_light poisoned")
}

/// Overrides the simulated speed of light (m/s).
///
/// Safety note: this is a global setting (shared across threads). It is intended for tests and
/// single-process simulations. A real distributed deployment would treat $c$ as a physical constant
/// and would not dynamically mutate it.
pub fn set_speed_of_light(c: f64) {
    if let Ok(mut guard) = c_cell().write() {
        *guard = c;
    }
}

/// A spacetime coordinate used by Minkowski-KV to reason about causality.
///
/// The project is a *relativistic* distributed database: we treat each write/event as a spacetime
/// event and use the Minkowski interval to decide whether two events are causally orderable.
///
/// - `t` is expressed in nanoseconds (as a coordinate-time in some chosen frame).
/// - `(x, y, z)` are meters in the same frame.
///
/// The core invariant we care about is the Minkowski interval:
///
/// $s^2 = c^2\Delta t^2 - \Delta x^2 - \Delta y^2 - \Delta z^2$
///
/// With this sign convention (“West Coast” / $(+,-,-,-)$):
/// - $s^2 > 0$ (timelike): events can be causally related; an ordering exists.
/// - $s^2 = 0$ (lightlike): events lie on the light cone.
/// - $s^2 < 0$ (spacelike): no causal order is enforced by physics alone; we treat them as
///   concurrent for database purposes (e.g., both may become DAG heads).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct SpacetimeCoord {
    /// Nanoseconds from epoch.
    pub t: u128,
    /// Position in meters.
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl SpacetimeCoord {
    /// Computes the Minkowski interval squared $s^2$ between two coordinates.
    ///
    /// This function deliberately returns the *squared* interval to avoid an unnecessary `sqrt` and
    /// to preserve the sign, which is what we care about for causal classification.
    pub fn interval_sq(&self, other: &Self) -> f64 {
        let c = speed_of_light();
        let delta_t_ns = other.t as i128 - self.t as i128;
        let delta_t_s = delta_t_ns as f64 * 1e-9;

        let delta_x = other.x - self.x;
        let delta_y = other.y - self.y;
        let delta_z = other.z - self.z;

        let spatial_sq = delta_x * delta_x + delta_y * delta_y + delta_z * delta_z;
        let ct = c * delta_t_s;
        (ct * ct) - spatial_sq
    }
}

impl PartialOrd for SpacetimeCoord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let s_sq = self.interval_sq(other);
        if s_sq < 0.0 {
            // Spacelike-separated events have no physically mandated order.
            return None;
        }

        let delta_t_ns = other.t as i128 - self.t as i128;
        if delta_t_ns > 0 {
            Some(Ordering::Less)
        } else if delta_t_ns < 0 {
            Some(Ordering::Greater)
        } else {
            Some(Ordering::Equal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timelike_ordering() {
        let a = SpacetimeCoord {
            t: 0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let b = SpacetimeCoord {
            t: 10,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };

        assert!(a < b, "Timelike future event should order as less");
        assert_eq!(a.partial_cmp(&b), Some(Ordering::Less));
        assert_eq!(b.partial_cmp(&a), Some(Ordering::Greater));
    }

    #[test]
    fn spacelike_concurrent() {
        let earth = SpacetimeCoord {
            t: 0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        // Approximate Mars distance requiring ~3 minutes of light travel, but only 1s apart in time.
        let mars = SpacetimeCoord {
            t: 1_000_000_000, // 1 second later in nanoseconds
            x: 5.4e10,
            y: 0.0,
            z: 0.0,
        };

        assert_eq!(earth.partial_cmp(&mars), None);
        assert_eq!(mars.partial_cmp(&earth), None);
    }
}
