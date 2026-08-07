//! Local model inference.
//!
//! Everything here runs on the operator's own hardware. No inference request leaves the machine,
//! which is what makes the privacy claim structural rather than contractual — a hosted API would
//! send both the user's query and the retrieved passages to a third party on every search.
//!
//! - [`device`] — GPU or CPU, chosen at runtime from the admin page.
//! - [`registry`] — which models exist, their sizes, and whether they are present on disk.

pub mod device;
#[cfg(feature = "llama")]
pub mod engine;
pub mod prompt;
pub mod registry;
pub mod validate;

pub use device::{ActiveDevice, DeviceConfig, DevicePreference, Resolved};
pub use prompt::{OutputLang, Passage, Prompt};
pub use registry::{ModelSpec, ModelStatus, Registry};
pub use validate::{Rejection, Summary};
