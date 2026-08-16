//! The `parse-check` command: fetch a real URL and show exactly what the parser extracts (M2-T11.4).
//!
//! This is the tool for authoring per-domain rules with evidence instead of guesses. A rule is only
//! worth adding where generic extraction actually fails ([[Content Parser]] §4.1) — a selector for a
//! site that already ships JSON-LD is dead weight that silently shadows correct metadata. So the
//! workflow is: run this against an article, see whether the title and (especially) the date came
//! through and by which method, and add a rule only for the fields that came back empty.
//!
//! With `--date/--title/--body` it also tries a candidate selector against the fetched HTML and
//! reports what it would capture — so a rule is verified against the real page before it is written
//! into `data/parsers/domains.toml`, never after.

use anyhow::{Context, Result};

use xustive_core::{DatePrecision, SourceType};
use xustive_ingest::{FetchConfig, Fetcher, ParseError, Parser};

pub struct CheckOptions {
    pub url: String,
    pub rules_path: String,
    /// Candidate selectors to try against the fetched HTML, without writing a rule.
    pub date: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
}

pub async fn run(opts: &CheckOptions) -> Result<()> {
    let fetcher = Fetcher::new(FetchConfig::default()).context("building the fetcher")?;
    let fetched = fetcher
        .get(&opts.url)
        .await
        .with_context(|| format!("fetching {}", opts.url))?;

    println!(
        "fetched  {}  ({} bytes)",
        fetched.final_url,
        fetched.body.len()
    );

    // Run the real parser with the shipped rules, so the output matches what the crawler would do.
    let rules = xustive_ingest::rules::Rules::load(&opts.rules_path);
    let host = url::Url::parse(&fetched.final_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();
    let has_rule = rules.for_host(&host).is_some();
    let parser = Parser::default().with_rules(rules);

    match parser.parse(
        &fetched.body,
        &fetched.final_url,
        "parse-check",
        SourceType::Web,
    ) {
        Ok(p) => {
            let d = &p.document;
            let date = match d.published_at_precision {
                DatePrecision::Unknown => "UNKNOWN (fell back to crawl date)".to_string(),
                prec => format!("{} (precision {:?})", d.published_at, prec),
            };
            println!(
                "host     {host}{}",
                if has_rule {
                    "  [per-domain rule applies]"
                } else {
                    ""
                }
            );
            println!("method   {:?}", p.method);
            println!("title    {}", show(&d.title));
            println!("date     {date}");
            println!("body     {} words", d.body.split_whitespace().count());
            println!("outlinks {}", p.outlinks.len());
            if matches!(d.published_at_precision, DatePrecision::Unknown) {
                println!(
                    "\n⚠ no date — this is the field a per-domain `date` selector should fix."
                );
            }
        }
        Err(ParseError::TooLittleContent { chars, .. }) => {
            println!("host     {host}");
            println!("result   too thin to index ({chars} chars) — a listing page, or extraction missed the body");
        }
        Err(e) => println!("result   parse failed: {e}"),
    }

    // Candidate selectors, tried directly against the DOM so a rule can be verified before writing.
    if opts.date.is_some() || opts.title.is_some() || opts.body.is_some() {
        println!("\ncandidate selectors:");
        for (field, sel) in [
            ("date", &opts.date),
            ("title", &opts.title),
            ("body", &opts.body),
        ] {
            if let Some(s) = sel {
                match xustive_ingest::rules::capture_selector(&fetched.body, s) {
                    Some(text) => println!("  {field:<6} {s:?} → {}", show(&text)),
                    None => println!("  {field:<6} {s:?} → (no match / invalid selector)"),
                }
            }
        }
    }
    Ok(())
}

/// Trim a field for one-line display so a whole article body does not flood the terminal.
fn show(s: &str) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.is_empty() {
        "(empty)".into()
    } else if one.chars().count() > 100 {
        format!("{}…", one.chars().take(100).collect::<String>())
    } else {
        one
    }
}
