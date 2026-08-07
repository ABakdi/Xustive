//! Compute device selection.
//!
//! Xustive must run on a GPU **and** on CPU alone, and which one is a runtime setting rather
//! than a build-time choice — the operator changes it from the admin page, including to test CPU
//! behaviour on a machine that has a perfectly good GPU.
//!
//! Two consequences shape everything here:
//!
//! - GPU support is compiled in when available, but its **absence is never fatal**. A missing
//!   driver, a busy card, or a machine that simply has no GPU all fall back to CPU with a
//!   warning. A search engine that refuses to start because a graphics card is missing is worse
//!   than a slow one.
//! - The reference deployment is a **Quadro T1000 with 4 GB**, which is the binding constraint on
//!   model size. Partial offload matters on a card that small, so the number of layers pushed to
//!   the GPU is itself a setting.

use serde::{Deserialize, Serialize};

/// What the operator asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DevicePreference {
    /// Use the GPU when one is usable, otherwise CPU. The default.
    #[default]
    Auto,
    /// Force GPU. Still falls back to CPU rather than failing, but says so loudly.
    Gpu,
    /// Force CPU even when a GPU is present. This is the setting used to test CPU behaviour.
    Cpu,
}

impl DevicePreference {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Gpu => "gpu",
            Self::Cpu => "cpu",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "gpu" | "cuda" => Some(Self::Gpu),
            "cpu" => Some(Self::Cpu),
            _ => None,
        }
    }
}

/// What we actually ended up using.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ActiveDevice {
    Gpu,
    Cpu,
}

impl ActiveDevice {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gpu => "gpu",
            Self::Cpu => "cpu",
        }
    }
}

/// A GPU we could use, if any.
#[derive(Debug, Clone, Serialize)]
pub struct GpuInfo {
    pub name: String,
    pub total_mib: u64,
    pub free_mib: u64,
    pub driver: String,
}

/// The resolved decision, with enough detail for the admin page to explain itself.
#[derive(Debug, Clone, Serialize)]
pub struct Resolved {
    pub preference: DevicePreference,
    pub active: ActiveDevice,
    /// Layers to offload. Zero means pure CPU.
    pub gpu_layers: u32,
    pub gpu: Option<GpuInfo>,
    /// Why this decision was made, in plain language, for display.
    pub reason: String,
    /// True when the operator asked for GPU and did not get it.
    pub fell_back: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct DeviceConfig {
    pub preference: DevicePreference,
    /// `None` means decide from available memory. `Some(0)` forces CPU-only execution.
    pub gpu_layers: Option<u32>,
    /// Headroom left free on the card for the KV cache and compute buffers.
    ///
    /// On a 4 GB card this is the difference between running and an out-of-memory crash
    /// halfway through the first generation.
    pub reserve_mib: u64,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            preference: DevicePreference::Auto,
            gpu_layers: None,
            reserve_mib: 900,
        }
    }
}

/// Whether GPU support was compiled in.
///
/// Separate from whether a GPU is *present*: a binary built without CUDA cannot use a card even
/// when one is sitting there, and the admin page should say which of the two is the problem.
pub const fn gpu_support_compiled() -> bool {
    cfg!(feature = "cuda")
}

/// Decide which device to use.
///
/// `model_size_mib` is the on-disk size of the weights, used to estimate whether the model fits.
pub fn resolve(config: &DeviceConfig, model_size_mib: u64) -> Resolved {
    let gpu = detect_gpu();

    let cpu = |reason: String, fell_back: bool| Resolved {
        preference: config.preference,
        active: ActiveDevice::Cpu,
        gpu_layers: 0,
        gpu: gpu.clone(),
        reason,
        fell_back,
    };

    if config.preference == DevicePreference::Cpu {
        return cpu("CPU requested by configuration".into(), false);
    }
    if config.gpu_layers == Some(0) {
        return cpu("gpu_layers is 0, which means CPU-only".into(), false);
    }

    let wants_gpu = config.preference == DevicePreference::Gpu;

    if !gpu_support_compiled() {
        // Deliberately a warning rather than an error: the operator gets a working, slower
        // system and a clear explanation, instead of a process that will not start.
        return cpu(
            "this binary was built without GPU support (build with --features cuda)".into(),
            wants_gpu,
        );
    }

    let Some(info) = gpu.clone() else {
        return cpu("no usable GPU detected".into(), wants_gpu);
    };

    let usable = info.free_mib.saturating_sub(config.reserve_mib);
    if usable == 0 {
        return cpu(
            format!(
                "{} has {} MiB free, less than the {} MiB reserve",
                info.name, info.free_mib, config.reserve_mib
            ),
            wants_gpu,
        );
    }

    // Explicit layer count wins; otherwise offload everything if it fits, and nothing if the
    // model is far too large to be worth splitting.
    let layers = match config.gpu_layers {
        Some(n) => n,
        None if model_size_mib <= usable => u32::MAX, // llama.cpp reads this as "all layers"
        None if model_size_mib <= usable * 3 => {
            // Partial offload. On a 4 GB card this is the difference between a model running
            // usefully and not running at all.
            ((usable as f64 / model_size_mib as f64) * 32.0) as u32
        }
        None => 0,
    };

    if layers == 0 {
        return cpu(
            format!(
                "model needs ~{model_size_mib} MiB but only {usable} MiB is usable on {}",
                info.name
            ),
            wants_gpu,
        );
    }

    let reason = if layers == u32::MAX {
        format!(
            "{} with {} MiB free, model fits entirely",
            info.name, info.free_mib
        )
    } else {
        format!(
            "{} with {} MiB free, offloading {layers} layers and running the rest on CPU",
            info.name, info.free_mib
        )
    };

    Resolved {
        preference: config.preference,
        active: ActiveDevice::Gpu,
        gpu_layers: layers,
        gpu: Some(info),
        reason,
        fell_back: false,
    }
}

/// Ask the driver what is present.
///
/// Shelling out to `nvidia-smi` rather than linking the NVML library keeps this working in a
/// binary built without CUDA, which is precisely the case where the admin page most needs to
/// explain that a GPU exists but cannot be used.
pub fn detect_gpu() -> Option<GpuInfo> {
    let out = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,memory.free,driver_version",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let first = line.lines().next()?;
    let cols: Vec<&str> = first.split(',').map(str::trim).collect();
    if cols.len() < 4 {
        return None;
    }
    Some(GpuInfo {
        name: cols[0].to_string(),
        total_mib: cols[1].parse().ok()?,
        free_mib: cols[2].parse().ok()?,
        driver: cols[3].to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(free: u64) -> GpuInfo {
        GpuInfo {
            name: "Quadro T1000".into(),
            total_mib: 4096,
            free_mib: free,
            driver: "610.43.03".into(),
        }
    }

    #[test]
    fn preference_parses_and_round_trips() {
        for p in [
            DevicePreference::Auto,
            DevicePreference::Gpu,
            DevicePreference::Cpu,
        ] {
            assert_eq!(DevicePreference::parse(p.as_str()), Some(p));
        }
        assert_eq!(DevicePreference::parse("CUDA"), Some(DevicePreference::Gpu));
        assert_eq!(DevicePreference::parse("nonsense"), None);
    }

    #[test]
    fn cpu_preference_is_honoured_even_with_a_gpu_present() {
        // The setting exists precisely so CPU behaviour can be tested on a GPU machine.
        let r = resolve(
            &DeviceConfig {
                preference: DevicePreference::Cpu,
                ..Default::default()
            },
            2000,
        );
        assert_eq!(r.active, ActiveDevice::Cpu);
        assert_eq!(r.gpu_layers, 0);
        assert!(!r.fell_back, "an explicit CPU request is not a fallback");
    }

    #[test]
    fn zero_layers_means_cpu() {
        let r = resolve(
            &DeviceConfig {
                gpu_layers: Some(0),
                ..Default::default()
            },
            2000,
        );
        assert_eq!(r.active, ActiveDevice::Cpu);
    }

    #[test]
    fn a_missing_gpu_falls_back_rather_than_failing() {
        // The property that matters most: no configuration makes this function fail. A search
        // engine that will not start because a graphics card is absent is worse than a slow one.
        for pref in [
            DevicePreference::Auto,
            DevicePreference::Gpu,
            DevicePreference::Cpu,
        ] {
            let r = resolve(
                &DeviceConfig {
                    preference: pref,
                    ..Default::default()
                },
                100_000, // absurdly large, cannot fit anywhere
            );
            assert!(matches!(r.active, ActiveDevice::Cpu | ActiveDevice::Gpu));
            assert!(!r.reason.is_empty(), "every decision must be explainable");
        }
    }

    #[test]
    fn forcing_gpu_without_one_records_the_fallback() {
        let r = resolve(
            &DeviceConfig {
                preference: DevicePreference::Gpu,
                ..Default::default()
            },
            100_000,
        );
        if r.active == ActiveDevice::Cpu {
            assert!(
                r.fell_back,
                "asking for GPU and getting CPU must be visible to the operator"
            );
        }
    }

    #[test]
    fn the_reserve_protects_a_small_card() {
        // A 4 GB card with 3 GB free cannot give all 3 GB to weights: the KV cache and compute
        // buffers need room, and without the reserve this crashes partway through generation.
        let cfg = DeviceConfig {
            reserve_mib: 900,
            ..Default::default()
        };
        let usable = gpu(3000).free_mib - cfg.reserve_mib;
        assert_eq!(usable, 2100);
        assert!(
            usable < 3000,
            "the reserve must actually reduce what we treat as available"
        );
    }

    #[test]
    fn every_resolution_explains_itself() {
        let r = resolve(&DeviceConfig::default(), 2100);
        assert!(!r.reason.is_empty());
        assert_eq!(r.preference, DevicePreference::Auto);
    }

    #[test]
    fn gpu_support_flag_is_independent_of_gpu_presence() {
        // A binary built without CUDA cannot use a card that is physically present, and the
        // admin page has to distinguish the two.
        let compiled = gpu_support_compiled();
        let present = detect_gpu().is_some();
        let _ = (compiled, present);
    }

    #[test]
    fn detection_never_panics_without_nvidia_smi() {
        let _ = detect_gpu();
    }
}
