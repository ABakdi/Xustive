//! The model registry.
//!
//! Models are not baked into container images: they are large, their licences differ, and a
//! 2 GB layer makes every deploy slow. They live in a directory the operator manages, and this
//! module is the single place that knows what should be there.
//!
//! Every entry records its licence. A research-only model would invalidate the whole
//! self-hosting design, and finding that out after launch is much worse than before.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// What a model is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Generates the short answer above the results.
    Summariser,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelSpec {
    pub id: &'static str,
    pub role: Role,
    pub file: &'static str,
    /// Size in MiB, used by device selection to decide whether it fits in VRAM.
    pub size_mib: u64,
    pub licence: &'static str,
    /// Whether the licence permits **commercial** use. Not every model that is free to run locally
    /// is free to ship: Qwen2.5-3B, for one, is under a non-commercial research licence. The engine
    /// warns at load when a non-commercial model is selected, and the admin console shows this.
    pub commercial_use: bool,
    /// Where it came from, so a replacement can be fetched without archaeology.
    pub source: &'static str,
    pub notes: &'static str,
}

/// The models Xustive knows about.
///
/// Qwen is the default family throughout: it has unusually strong Arabic for its size and its small
/// quantised variants fit the 4 GB reference card. **Licence is not uniform across sizes**, though —
/// 0.5B/1.5B/7B/14B/32B are Apache-2.0, while **3B and 72B are the non-commercial `qwen-research`
/// licence**. Each entry records the truth, and `commercial_use` is the flag callers key off.
pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "qwen2.5-3b-instruct-q4",
        role: Role::Summariser,
        file: "qwen2.5-3b-instruct-q4_k_m.gguf",
        size_mib: 2007,
        // Qwen2.5-3B is NOT Apache-2.0 — it is the non-commercial qwen-research licence, unlike the
        // other sizes here. Verified on the model card 2026-08-21. Fine for local eval, not for a
        // commercial launch: the engine warns when it is loaded. See models/LICENSES.md. The order
        // is unchanged (this stays the resolve default) — switching the default is a product choice,
        // not something to do silently under a licence fix.
        licence: "Qwen-Research (non-commercial)",
        commercial_use: false,
        source: "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF",
        notes: "Best Arabic that fits 4 GB, but non-commercial. Pin a commercial-safe size to ship.",
    },
    ModelSpec {
        id: "qwen2.5-1.5b-instruct-q4",
        role: Role::Summariser,
        file: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        size_mib: 1070,
        licence: "Apache-2.0",
        commercial_use: true,
        source: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF",
        notes: "Commercial-safe. Fits any card; faster on CPU. Weaker Arabic than 3B/7B.",
    },
    ModelSpec {
        id: "qwen2.5-7b-instruct-q4",
        role: Role::Summariser,
        file: "qwen2.5-7b-instruct-q4_k_m.gguf",
        size_mib: 4680,
        licence: "Apache-2.0",
        commercial_use: true,
        source: "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF",
        notes:
            "Best Arabic and commercial-safe. Exceeds 4 GB VRAM; needs partial offload or a bigger card.",
    },
];

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub spec: ModelSpec,
    pub present: bool,
    pub path: String,
    /// Actual size on disk, which is how a truncated download is caught.
    pub actual_mib: u64,
}

pub struct Registry {
    dir: PathBuf,
}

impl Registry {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
        }
    }

    pub fn path_for(&self, spec: &ModelSpec) -> PathBuf {
        self.dir.join(spec.file)
    }

    pub fn status(&self) -> Vec<ModelStatus> {
        MODELS
            .iter()
            .map(|spec| {
                let path = self.path_for(spec);
                let actual = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                ModelStatus {
                    spec: spec.clone(),
                    // A file that exists but is far smaller than expected is a truncated
                    // download, and reporting it as present would produce a confusing load
                    // failure much later.
                    present: actual > 0 && actual / 1_048_576 >= spec.size_mib * 9 / 10,
                    path: path.display().to_string(),
                    actual_mib: actual / 1_048_576,
                }
            })
            .collect()
    }

    pub fn find(&self, id: &str) -> Option<&'static ModelSpec> {
        MODELS.iter().find(|m| m.id == id)
    }

    /// The model to use for a role, preferring an explicit choice and otherwise the first
    /// present entry.
    pub fn resolve(&self, role: Role, preferred: Option<&str>) -> Option<ModelStatus> {
        let statuses = self.status();
        if let Some(id) = preferred {
            if let Some(s) = statuses.iter().find(|s| s.spec.id == id && s.present) {
                return Some(s.clone());
            }
        }
        statuses
            .into_iter()
            .find(|s| s.spec.role == role && s.present)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commercial_use_flag_matches_the_licence() {
        // The trap this guards against is a model *mislabelled* as permissive — which is exactly the
        // bug found in the 3B entry (it claimed Apache-2.0 but is qwen-research). A permissive
        // licence string must carry commercial_use=true, and anything else must carry false, so the
        // flag can never quietly disagree with the licence it is derived from.
        for m in MODELS {
            let looks_permissive = m.licence.contains("Apache") || m.licence.contains("MIT");
            assert_eq!(
                m.commercial_use, looks_permissive,
                "{} licence {:?} disagrees with commercial_use={}",
                m.id, m.licence, m.commercial_use
            );
        }
    }

    #[test]
    fn a_commercial_safe_summariser_exists() {
        // The self-hosting design must not be a dead end for a commercial deployment: there has to
        // be at least one summariser an operator can actually ship.
        assert!(
            MODELS
                .iter()
                .any(|m| m.role == Role::Summariser && m.commercial_use),
            "no commercially usable summariser is registered"
        );
    }

    #[test]
    fn every_model_records_where_it_came_from() {
        for m in MODELS {
            assert!(m.source.starts_with("https://"), "{} has no source", m.id);
        }
    }

    #[test]
    fn the_default_summariser_fits_the_reference_card() {
        // Quadro T1000, 4096 MiB, minus headroom for the KV cache and compute buffers.
        let default = MODELS
            .iter()
            .find(|m| m.id == "qwen2.5-3b-instruct-q4")
            .unwrap();
        assert!(
            default.size_mib < 4096 - 900,
            "the default model does not fit the reference GPU"
        );
    }

    #[test]
    fn a_missing_directory_reports_absent_rather_than_failing() {
        let r = Registry::new("/nonexistent/models");
        assert!(r.status().iter().all(|s| !s.present));
        assert!(r.resolve(Role::Summariser, None).is_none());
    }

    #[test]
    fn a_truncated_download_is_not_reported_as_present() {
        let dir = std::env::temp_dir().join("xustive-registry-test");
        std::fs::create_dir_all(&dir).unwrap();
        let spec = &MODELS[0];
        let path = dir.join(spec.file);
        std::fs::write(&path, b"GGUF truncated").unwrap();

        let r = Registry::new(&dir);
        let s = r
            .status()
            .into_iter()
            .find(|s| s.spec.id == spec.id)
            .unwrap();
        assert!(!s.present, "a 14-byte file must not count as a 2 GB model");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<&str> = MODELS.iter().map(|m| m.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate model ids");
    }
}
