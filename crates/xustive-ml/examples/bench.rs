//! Measure generation speed for each present model on the current device.
//!
//! Exists because the performance budget in the specification was written before any of this ran,
//! and the only honest way to hold it is to measure. Run with
//! `cargo run --release -p xustive-ml --example bench -- [cpu|gpu|auto]`.

use std::time::Duration;

use xustive_ml::device::{DeviceConfig, DevicePreference};
use xustive_ml::engine::{Engine, Sampling};
use xustive_ml::prompt::{self, OutputLang, Passage};
use xustive_ml::registry::Registry;

#[tokio::main]
async fn main() {
    let dir = std::env::var("XUSTIVE_MODEL_DIR").unwrap_or_else(|_| "models".into());
    let preference = match std::env::args().nth(1).as_deref() {
        Some("gpu") => DevicePreference::Gpu,
        Some("auto") => DevicePreference::Auto,
        _ => DevicePreference::Cpu,
    };

    let passages: Vec<Passage> = [
        ("doc1", "أعلنت شركة سونلغاز أن استهلاك الكهرباء في الجزائر بلغ مستوى قياسيا خلال شهر جويلية الماضي، مسجلا 18500 ميغاواط، وهو أعلى رقم في تاريخ الشركة.", "elkhabar.com"),
        ("doc2", "أرجعت وزارة الطاقة هذا الارتفاع إلى موجة الحر التي عرفتها عدة ولايات، وأكدت أن الشبكة الوطنية استوعبت الطلب دون انقطاعات كبيرة.", "aps.dz"),
        ("doc3", "سجلت ولايات الجنوب أعلى نسب استهلاك بسبب التكييف، حسب حصيلة أولية نشرتها الشركة.", "ennaharonline.com"),
    ]
    .iter()
    .map(|(id, text, domain)| Passage {
        id: (*id).into(),
        title: String::new(),
        text: (*text).into(),
        domain: (*domain).into(),
        published_at: Some(1_754_438_400),
        quality_score: 1.0,
        spam_score: 0.0,
    })
    .collect();

    println!(
        "{:<28} {:>7} {:>9} {:>9} {:>9}",
        "model", "tokens", "ttft", "total", "tok/s"
    );

    for status in Registry::new(&dir).status().iter().filter(|s| s.present) {
        let engine = match Engine::load(
            &status.path,
            &DeviceConfig {
                preference,
                ..Default::default()
            },
            1,
        ) {
            Ok(e) => e,
            Err(e) => {
                println!("{:<28} failed to load: {e}", status.spec.id);
                continue;
            }
        };

        let p = prompt::build("استهلاك الكهرباء في الجزائر", OutputLang::Arabic, &passages)
            .expect("passages should survive selection");

        match engine
            .generate(p, Sampling::default(), Duration::from_secs(300))
            .await
        {
            Ok(g) => println!(
                "{:<28} {:>7} {:>8.1}s {:>8.1}s {:>9.1}",
                status.spec.id,
                g.tokens,
                g.time_to_first_token.as_secs_f64(),
                g.total.as_secs_f64(),
                // Decode rate excluding prefill, which is the number that scales with output
                // length. Time to first token is reported separately because it is the one the
                // user actually feels.
                g.tokens as f64 / (g.total - g.time_to_first_token).as_secs_f64().max(0.001),
            ),
            Err(e) => println!("{:<28} failed: {e}", status.spec.id),
        }
    }
}
