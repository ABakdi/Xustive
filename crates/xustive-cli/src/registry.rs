//! The `registry` command: curate the data sources registry from the terminal (M2-T11).
//!
//! Every action loads the JSON-Lines file, mutates it, and writes it back — so a change is a
//! one-line git diff the curator can review and commit. This is the operator side of the two rules
//! the store enforces: nothing crawls until a human moves it out of `proposed` and approves it
//! ([[Data Sources Registry]] §1, §6). `approve`/`activate` are that human act, made explicit.

use std::path::Path;

use anyhow::{bail, Context, Result};
use clap::Subcommand;

use xustive_core::{now_unix, Lifecycle, Registry};

#[derive(Subcommand, Debug)]
pub enum RegistryAction {
    /// List sources, optionally filtered.
    List {
        /// Only this category (matches the `seed:<category>` note the seed writes).
        #[arg(long)]
        category: Option<String>,
        /// Only sources that would actually be crawled right now.
        #[arg(long)]
        crawlable: bool,
    },
    /// Summarise the registry: counts by lifecycle and how many are crawlable.
    Stats,
    /// Validate the file and report anything an operator should see. Exits non-zero on a problem.
    Lint,
    /// Re-export the file to its canonical form (sorted, one compact record per line). Run after a
    /// hand-edit so the next machine write is a clean one-line diff, not a whole-file reformat.
    Fmt,
    /// Approve a source: a human vouches for it. Sets `approved`, moves it to `approved`.
    Approve { id: String },
    /// Put an approved source into service: `approved` + `active`, so the crawler will seed it.
    Activate { id: String },
    /// Disable a source (opt-out, takedown, persistent failure). Starts the 90-day archival clock.
    Disable {
        id: String,
        #[arg(long, default_value = "operator disabled")]
        reason: String,
    },
}

pub fn run(path: &Path, action: &RegistryAction) -> Result<()> {
    let path = path
        .to_str()
        .context("registry path is not valid UTF-8")?
        .to_string();

    match action {
        RegistryAction::List {
            category,
            crawlable,
        } => list(&path, category.as_deref(), *crawlable),
        RegistryAction::Stats => stats(&path),
        RegistryAction::Lint => lint(&path),
        RegistryAction::Fmt => fmt(&path),
        RegistryAction::Approve { id } => transition(&path, id, Transition::Approve),
        RegistryAction::Activate { id } => transition(&path, id, Transition::Activate),
        RegistryAction::Disable { id, reason } => {
            transition(&path, id, Transition::Disable(reason.clone()))
        }
    }
}

fn load(path: &str) -> Result<Registry> {
    Registry::load(path).with_context(|| format!("loading registry from {path}"))
}

fn list(path: &str, category: Option<&str>, crawlable_only: bool) -> Result<()> {
    let reg = load(path)?;
    let want_note = category.map(|c| format!("seed:{c}"));
    let mut shown = 0;
    for s in reg.sources() {
        if crawlable_only && !s.is_crawlable() {
            continue;
        }
        if let Some(note) = &want_note {
            if s.notes.as_deref() != Some(note.as_str()) {
                continue;
            }
        }
        let mark = if s.is_crawlable() { "●" } else { "○" };
        println!(
            "{mark} {:<28} {:<9} tier {:?}  {}",
            s.id,
            format!("{:?}", s.lifecycle).to_lowercase(),
            s.trust_tier,
            s.entry_points.first().map(String::as_str).unwrap_or("-"),
        );
        shown += 1;
    }
    println!("\n{shown} shown, {} total", reg.len());
    Ok(())
}

fn stats(path: &str) -> Result<()> {
    let reg = load(path)?;
    let mut by_state: std::collections::BTreeMap<String, usize> = Default::default();
    for s in reg.sources() {
        *by_state
            .entry(format!("{:?}", s.lifecycle).to_lowercase())
            .or_default() += 1;
    }
    println!("{} sources", reg.len());
    for (state, n) in &by_state {
        println!("  {state:<10} {n}");
    }
    println!("  {:<10} {}", "crawlable", reg.crawlable().count());
    Ok(())
}

fn lint(path: &str) -> Result<()> {
    // Loading already enforces the hard rule: every record parses and carries a legal_basis, or
    // load() fails. Beyond that, report the soft anomalies a curator wants to know about.
    let reg = load(path)?;
    let mut warnings = 0;
    for s in reg.sources() {
        if s.entry_points.is_empty() {
            eprintln!("warn: {} has no entry points", s.id);
            warnings += 1;
        }
        if s.approved && s.lifecycle == Lifecycle::Proposed {
            eprintln!("warn: {} is approved but still proposed", s.id);
            warnings += 1;
        }
    }
    println!("{} sources, {warnings} warning(s)", reg.len());
    if warnings > 0 {
        bail!("registry lint found {warnings} warning(s)");
    }
    Ok(())
}

fn fmt(path: &str) -> Result<()> {
    let reg = load(path)?;
    reg.save(path)
        .with_context(|| format!("writing registry back to {path}"))?;
    println!("normalised {} sources in {path}", reg.len());
    Ok(())
}

enum Transition {
    Approve,
    Activate,
    Disable(String),
}

fn transition(path: &str, id: &str, t: Transition) -> Result<()> {
    let mut reg = load(path)?;
    let s = reg
        .get_mut(id)
        .with_context(|| format!("no source with id {id:?}"))?;

    match t {
        Transition::Approve => {
            if s.lifecycle == Lifecycle::Archived {
                bail!("{id} is archived; re-propose it before approving");
            }
            s.approved = true;
            s.lifecycle = Lifecycle::Approved;
            println!("approved {id}");
        }
        Transition::Activate => {
            if s.lifecycle == Lifecycle::Archived {
                bail!("{id} is archived; re-propose it before activating");
            }
            s.approved = true;
            s.lifecycle = Lifecycle::Active;
            println!("activated {id} — the crawler will now seed it");
        }
        Transition::Disable(reason) => {
            if !s.disable_at(&reason, now_unix()) {
                bail!("{id} is already disabled or archived");
            }
            println!("disabled {id} ({reason})");
        }
    }

    reg.save(path)
        .with_context(|| format!("writing registry back to {path}"))?;
    Ok(())
}
