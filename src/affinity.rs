//! Keep parallel decode workers on one CPU core tier.
//!
//! On hybrid CPUs (Intel P/E cores, ARM big.LITTLE) a parallel decode split
//! into equal strips finishes only when its slowest strip does. One strip
//! scheduled on an efficiency core erased the whole gain in measurement
//! (`docs/IDOT_PARALLEL_PNG.md`). On Linux, workers are pinned to the
//! fastest tier the process is allowed to run on; elsewhere this is a no-op.
//!
//! Tiers come from sysfs: `cpu_capacity` (ARM) when present, otherwise
//! `cpufreq/cpuinfo_max_freq`. CPUs within 15% of the fastest form the top
//! tier. Homogeneous machines, and tiers with fewer than two allowed CPUs,
//! get no pinning.

/// How parallel decode treats core tiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "_dev"), allow(dead_code))] // Off/Workers: benchmark overrides
pub(crate) enum PinMode {
    /// Let the OS schedule freely.
    Off,
    /// Pin spawned workers to the top tier; leave the calling thread alone.
    Workers,
    /// Also pin the calling thread for the duration of the decode, restoring
    /// its original affinity afterwards.
    All,
}

/// Default mode. See `docs/IDOT_PARALLEL_PNG.md` ("Core tiers") for the
/// measurement behind it.
const DEFAULT_MODE: PinMode = PinMode::All;

pub(crate) fn mode() -> PinMode {
    #[cfg(feature = "_dev")]
    {
        // Benchmark override: ZENPNG_PIN=off|workers|all.
        if let Ok(v) = std::env::var("ZENPNG_PIN") {
            return match v.as_str() {
                "off" => PinMode::Off,
                "workers" => PinMode::Workers,
                _ => PinMode::All,
            };
        }
    }
    DEFAULT_MODE
}

#[cfg(target_os = "linux")]
mod imp {
    use std::sync::OnceLock;

    use rustix::thread::{CpuSet, sched_getaffinity, sched_setaffinity};

    /// The top-tier CPU ids (process-independent), or `None` when the
    /// machine has a single tier or sysfs gives no answer.
    fn top_tier() -> Option<&'static [usize]> {
        static TIER: OnceLock<Option<Vec<usize>>> = OnceLock::new();
        TIER.get_or_init(|| {
            let mut perf = Vec::new();
            for entry in std::fs::read_dir("/sys/devices/system/cpu").ok()?.flatten() {
                let name = entry.file_name();
                let Some(id) = name
                    .to_str()
                    .and_then(|n| n.strip_prefix("cpu"))
                    .and_then(|n| n.parse::<usize>().ok())
                else {
                    continue;
                };
                if id >= CpuSet::MAX_CPU {
                    continue;
                }
                let read = |f: &str| {
                    std::fs::read_to_string(entry.path().join(f))
                        .ok()
                        .and_then(|s| s.trim().parse::<u64>().ok())
                };
                if let Some(v) = read("cpu_capacity").or_else(|| read("cpufreq/cpuinfo_max_freq")) {
                    perf.push((id, v));
                }
            }
            let max = perf.iter().map(|p| p.1).max()?;
            let min = perf.iter().map(|p| p.1).min()?;
            if min * 100 >= max * 85 {
                return None; // one tier
            }
            Some(
                perf.iter()
                    .filter(|p| p.1 * 100 >= max * 85)
                    .map(|p| p.0)
                    .collect(),
            )
        })
        .as_deref()
    }

    pub(crate) struct Pin {
        set: CpuSet,
        cpus: usize,
    }

    impl Pin {
        /// Number of CPUs in the pinned set.
        pub(crate) fn cpus(&self) -> usize {
            self.cpus
        }

        pub(crate) fn apply_to_current_thread(&self) {
            // Best effort: a failure just leaves scheduling to the OS.
            let _ = sched_setaffinity(None, &self.set);
        }

        /// Pin the calling thread; the guard restores its previous mask.
        pub(crate) fn pin_caller(&self) -> Option<CallerGuard> {
            let old = sched_getaffinity(None).ok()?;
            sched_setaffinity(None, &self.set).ok()?;
            Some(CallerGuard { old })
        }
    }

    pub(crate) struct CallerGuard {
        old: CpuSet,
    }

    impl Drop for CallerGuard {
        fn drop(&mut self) {
            let _ = sched_setaffinity(None, &self.old);
        }
    }

    pub(crate) fn top_tier_pin() -> Option<Pin> {
        let tier = top_tier()?;
        let allowed = sched_getaffinity(None).ok()?;
        let mut set = CpuSet::new();
        let mut count = 0;
        for &cpu in tier {
            if allowed.is_set(cpu) {
                set.set(cpu);
                count += 1;
            }
        }
        (count >= 2).then_some(Pin { set, cpus: count })
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    pub(crate) struct Pin;

    impl Pin {
        pub(crate) fn cpus(&self) -> usize {
            usize::MAX
        }

        pub(crate) fn apply_to_current_thread(&self) {}

        pub(crate) fn pin_caller(&self) -> Option<CallerGuard> {
            None
        }
    }

    pub(crate) struct CallerGuard;

    pub(crate) fn top_tier_pin() -> Option<Pin> {
        None
    }
}

pub(crate) use imp::{CallerGuard, Pin};

/// The pin for spawned workers, if the current mode and machine call for one.
pub(crate) fn worker_pin() -> Option<Pin> {
    match mode() {
        PinMode::Off => None,
        PinMode::Workers | PinMode::All => imp::top_tier_pin(),
    }
}

/// Pin the calling thread too, when the mode says so.
pub(crate) fn caller_guard(pin: Option<&Pin>) -> Option<CallerGuard> {
    match mode() {
        PinMode::All => pin?.pin_caller(),
        _ => None,
    }
}
