//! The tool inventory.
//!
//! Exists so the client does not carry its own copy of the list. A settings page that lets someone
//! turn tools off has to enumerate them, and a hardcoded TypeScript array would drift the first
//! time a tool is added or renamed — silently, because a missing entry looks exactly like a tool
//! that is switched off.
//!
//! Static content. Nothing here depends on the query, the user, or any request state, so it is
//! cacheable and carries no privacy weight at all.

use axum::Json;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ToolInfo {
    /// The value that appears in `Answer.tool`. This is what an opt-out is keyed on.
    pub id: &'static str,
    /// The explicit invocation keyword, without the `!`.
    pub keyword: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ToolsResponse {
    pub tools: Vec<ToolInfo>,
}

/// Every tool that can produce an instant answer.
pub fn inventory() -> Vec<ToolInfo> {
    let mut tools: Vec<ToolInfo> = xustive_tools::registry()
        .iter()
        .map(|t| ToolInfo {
            id: t.name(),
            keyword: t.keyword(),
        })
        .collect();

    // Weather is not in the matcher registry: its answer needs the cache, and a matcher that did
    // I/O would put a Redis round trip on every search that is not about weather. It is still a
    // tool from the reader's side, so it belongs in a list they use to switch tools off.
    tools.push(ToolInfo {
        id: "weather",
        keyword: "weather",
    });

    tools.sort_by_key(|t| t.id);
    tools
}

pub async fn handler() -> Json<ToolsResponse> {
    Json(ToolsResponse { tools: inventory() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_tool_is_listed() {
        let listed = inventory();
        // The registry plus weather. If a tool is added to the registry and this count is not
        // updated, the assertion below still holds — the point is that nothing is dropped.
        assert_eq!(listed.len(), xustive_tools::registry().len() + 1);
        for tool in xustive_tools::registry() {
            assert!(
                listed.iter().any(|t| t.id == tool.name()),
                "{} is registered but not listed",
                tool.name()
            );
        }
    }

    #[test]
    fn identifiers_are_stable_and_url_safe() {
        // These are persisted in a cookie and matched against `Answer.tool`. A space or an
        // uppercase letter would survive a round trip through one browser and not another.
        for t in inventory() {
            assert!(!t.id.is_empty() && !t.keyword.is_empty());
            assert!(
                t.id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{} is not a stable identifier",
                t.id
            );
        }
    }

    #[test]
    fn every_tool_has_a_label_in_the_message_catalogue() {
        // Found by eye: `prayer-times` rendered as its raw identifier on the settings page,
        // because the catalogue key said `prayer` and `Answer.tool` says `prayer-times`. Nothing
        // failed — a tool labelled with its id looks like a tool nobody got round to naming.
        //
        // Scoped to the `ar` block, not the whole file. Searching the file passes as soon as
        // *any* of the three catalogues has the key, which makes it vacuous — verified by
        // deleting the Arabic and English entries and watching it still pass. The `Messages` type
        // forces the other two to match `ar`'s key set, so checking `ar` alone is both accurate
        // and sufficient.
        let catalogue = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/lib/i18n/messages.ts"),
        )
        .expect("message catalogue");

        let start = catalogue
            .find("const ar = {")
            .expect("the ar catalogue defines the key set");
        let block = &catalogue[start..];
        let block = &block[..block.find("} as const").expect("ar block is terminated")];

        let missing: Vec<&str> = inventory()
            .iter()
            .map(|t| t.id)
            .filter(|id| {
                // Keys are written bare when they are valid identifiers and quoted when they are
                // not — `calculator:` but `'unit-converter':`.
                !block.contains(&format!("\n  {id}: ")) && !block.contains(&format!("\n  '{id}': "))
            })
            .collect();

        assert!(
            missing.is_empty(),
            "these tools would render as their raw identifiers: {missing:?}. \
             Add a key for each to web/lib/i18n/messages.ts."
        );
    }

    #[test]
    fn identifiers_are_unique() {
        // Two tools sharing an id would make one impossible to switch off independently.
        let ids: Vec<&str> = inventory().iter().map(|t| t.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "duplicate tool id in {ids:?}");
    }

    #[test]
    fn weather_is_listed_despite_not_being_in_the_registry() {
        // The reason it is absent from the registry is an implementation detail of matcher
        // purity. From the reader's side it is a tool like any other, and a settings page that
        // omitted it would offer no way to turn off the one tool that reads external data.
        assert!(inventory().iter().any(|t| t.id == "weather"));
    }
}
