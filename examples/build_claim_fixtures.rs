//! Builds the claim verifier's fixtures from real citations in the vault.
//!
//! Run through `scripts/build-claim-fixtures`. It reads bibliography entries of held papers
//! from the dev database (read only), builds the real, fake and near-miss sets, runs the
//! verifier once against the live `OpenAlex` and Crossref APIs in a scratch database while
//! recording every response, and writes `cases.json` and `responses.json`. Tests then
//! replay the recording and never touch the network.
//!
//! Deviations from the spec's set sizes, all forced by the data: the dev vault has 6 papers
//! with accepted entries (the spec wants 10 or more, with at most 5 entries each) and only
//! one entry prints a DOI, so real DOI cases take the DOI `OpenAlex` holds for the resolved
//! Work; the DOI path is therefore a check of the lookup and the title agreement rule, not
//! of DOI parsing. arXiv cases are kept only when `OpenAlex` resolves the arXiv DOI to the
//! accepted Work, so preprint-versus-published splits are not scored (the count left out is
//! printed). A `--rerecord` run records each case's source paper in `cases.json` and prints how
//! many papers contribute and the largest share; the committed fixtures predate that field. `OpenAlex`'s free-tier search budget caps
//! the set sizes.
//!
//! `--set holdout` builds the held-out fixtures for the verifier evaluation instead (see
//! the section at the end of this file): E1 samples real Works cited by vault papers that
//! the base fixtures never used, and E3 is a hand-specified set of hard cases read from
//! `eval/claims/e3/spec.json`. The base set is unchanged by it.
//!
//! It refuses to overwrite existing fixtures without `--rerecord`. `OpenAlex`'s free tier
//! charges for title searches, so the run stops at `--max-searches` live searches and keeps
//! what it recorded in a scratch file, which the next run resumes from.

#[allow(dead_code)]
#[path = "../tests/claims_support/mod.rs"]
mod claims_support;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use claims_support::{CaseSets, ClaimCase, Failure, Outcome, ScratchDatabase, run_cases};
use lysilogy_core::names::citation_family;
use lysilogy_core::{Claim, ClaimState, title_key, title_similarity};
use lysilogy_ingest::openalex::{
    JsonTransport, OpenAlexClient, ProviderError, ProviderFuture, ReqwestTransport, normalize_doi,
    short_openalex_id,
};
use lysilogy_ingest::verify_claim::fixtures::{CapturingTransport, ReplayTransport, Responses};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

const DEFAULT_DEV_DATABASE: &str = "lysilogy";
const DEFAULT_TEST_DATABASE: &str = "lysilogy_test";
const DEFAULT_MAX_SEARCHES: usize = 80;
/// Entries whose parsed title is this similar to the resolved Work's title are trusted.
const TRUTH_TITLE_MIN_SIMILARITY: f64 = 0.8;
const MIN_TITLE_WORDS: usize = 4;
/// A title cut at its subtitle must stay this specific; a four-word head names many papers.
const SUBTITLE_HEAD_MIN_WORDS: usize = 5;
const MIN_TITLE_CHARS: usize = 20;
const REAL_DOI: usize = 8;
const REAL_ARXIV: usize = 8;
const REAL_TITLE: usize = 14;
const NEAR_TYPO: usize = 4;
const NEAR_SUBTITLE: usize = 4;
const NEAR_ABBREVIATED: usize = 4;
const FAKE_FABRICATED_DOI: usize = 5;
const FAKE_WRONG_TITLE: usize = 5;
const FAKE_RECOMBINED: usize = 4;
const FAKE_YEAR_OFF: usize = 4;
const FAKE_AUTHORS_SWAPPED: usize = 5;
const FAKE_INVENTED: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SetKind {
    Base,
    Holdout,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    E1,
    E3,
    All,
}

struct Args {
    rerecord: bool,
    allow_shrink: bool,
    out: PathBuf,
    dev_url: String,
    test_url: String,
    max_searches: usize,
    scratch_file: PathBuf,
    set: SetKind,
    part: Part,
    seed: u64,
    real: usize,
    fake: usize,
    near: usize,
    max_per_paper: usize,
    doi_percent: usize,
    exclude: PathBuf,
    spec: Option<PathBuf>,
    replan: bool,
    scratch_dir: PathBuf,
}

fn user() -> String {
    std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned())
}

fn parse_args() -> Result<Args, Failure> {
    let home = std::env::var("HOME")?;
    let mut args = Args {
        rerecord: false,
        allow_shrink: false,
        out: PathBuf::from("tests/fixtures/claims"),
        dev_url: std::env::var("LYSILOGY_DATABASE_URL").unwrap_or_else(|_| {
            format!(
                "postgresql://{}@127.0.0.1:5433/{DEFAULT_DEV_DATABASE}",
                user()
            )
        }),
        test_url: std::env::var("LYSILOGY_TEST_DATABASE_URL").unwrap_or_else(|_| {
            format!(
                "postgresql://{}@127.0.0.1:5433/{DEFAULT_TEST_DATABASE}",
                user()
            )
        }),
        max_searches: DEFAULT_MAX_SEARCHES,
        scratch_file: PathBuf::from(&home).join(".cache/lysilogy-scratch/claims-responses.json"),
        set: SetKind::Base,
        part: Part::All,
        seed: HOLDOUT_SEED,
        real: E1_REAL,
        fake: E1_FAKE,
        near: E1_NEAR,
        max_per_paper: E1_MAX_PER_PAPER,
        doi_percent: E1_DOI_PERCENT,
        exclude: PathBuf::from("tests/fixtures/claims/cases.json"),
        spec: None,
        replan: false,
        scratch_dir: PathBuf::from(&home).join(".cache/lysilogy-scratch"),
    };
    let mut out_given = false;
    let mut searches_given = false;
    let mut iter = std::env::args().skip(1);
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--rerecord" => args.rerecord = true,
            "--allow-shrink" => args.allow_shrink = true,
            "--out" => {
                args.out = PathBuf::from(iter.next().ok_or("--out needs a directory")?);
                out_given = true;
            }
            "--max-searches" => {
                args.max_searches = iter
                    .next()
                    .ok_or("--max-searches needs a number")?
                    .parse()?;
                searches_given = true;
            }
            "--set" => {
                args.set = match iter.next().ok_or("--set needs base or holdout")?.as_str() {
                    "base" => SetKind::Base,
                    "holdout" => SetKind::Holdout,
                    other => return Err(format!("unknown set {other}").into()),
                };
            }
            "--part" => {
                args.part = match iter.next().ok_or("--part needs e1, e3 or all")?.as_str() {
                    "e1" => Part::E1,
                    "e3" => Part::E3,
                    "all" => Part::All,
                    other => return Err(format!("unknown part {other}").into()),
                };
            }
            "--seed" => args.seed = iter.next().ok_or("--seed needs a number")?.parse()?,
            "--real" => args.real = iter.next().ok_or("--real needs a number")?.parse()?,
            "--fake" => args.fake = iter.next().ok_or("--fake needs a number")?.parse()?,
            "--near" => args.near = iter.next().ok_or("--near needs a number")?.parse()?,
            "--max-per-paper" => {
                args.max_per_paper = iter
                    .next()
                    .ok_or("--max-per-paper needs a number")?
                    .parse()?;
            }
            "--doi-percent" => {
                args.doi_percent = iter.next().ok_or("--doi-percent needs a number")?.parse()?;
            }
            "--exclude" => {
                args.exclude = PathBuf::from(iter.next().ok_or("--exclude needs a cases file")?);
            }
            "--spec" => {
                args.spec = Some(PathBuf::from(iter.next().ok_or("--spec needs a file")?));
            }
            "--replan" => args.replan = true,
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    if args.set == SetKind::Holdout {
        if !out_given {
            args.out = PathBuf::from("eval/claims");
        }
        if !searches_given {
            args.max_searches = HOLDOUT_DEFAULT_SEARCHES;
        }
    }
    Ok(args)
}

/// A small deterministic generator, so a rebuilt fixture set picks the same entries.
struct Rng(u64);

impl Rng {
    const fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    #[allow(clippy::cast_possible_truncation)] // the remainder is below bound
    const fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for index in (1..items.len()).rev() {
            let other = self.below(index + 1);
            items.swap(index, other);
        }
    }
}

#[derive(Clone, Debug)]
struct Entry {
    paper: String,
    title: String,
    year: Option<i32>,
    authors: Vec<String>,
    venue: Option<String>,
    arxiv: Option<String>,
    has_doi: bool,
    truth: String,
}

fn field_text(fields: &Value, name: &str) -> Option<String> {
    let value = fields.get(name)?.get("value")?;
    let text = value.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

async fn load_entries(url: &str) -> Result<Vec<Entry>, Failure> {
    let pool = lysilogy_db::connect(url).await?;
    let rows: Vec<(String, Value, String, Option<String>)> = sqlx::query_as(
        "select e.citing_version_id::text, e.fields, i.value_normalized, w.display_title \
         from source.bibliography_entry e \
         join graph.entry_resolution r on r.entry_id = e.id and r.status = 'accepted' \
         join graph.work w on w.id = r.candidate_work_id \
         join graph.work_identifier i \
           on i.work_id = w.id and i.scheme = 'openalex' and i.status = 'active' \
         where e.fields #>> '{title,value}' is not null \
         order by e.citing_version_id, e.ordinal, e.id",
    )
    .fetch_all(&pool)
    .await?;
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for (paper, fields, truth, work_title) in rows {
        let Some(title) = field_text(&fields, "title") else {
            continue;
        };
        let authors: Vec<String> = fields
            .pointer("/authors/value")
            .and_then(Value::as_array)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| name.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let year = field_text(&fields, "year").and_then(|year| year.parse().ok());
        let agrees = work_title
            .as_deref()
            .is_some_and(|known| title_similarity(&title, known) >= TRUTH_TITLE_MIN_SIMILARITY);
        let specific = title.chars().count() >= MIN_TITLE_CHARS
            && title.split_whitespace().count() >= MIN_TITLE_WORDS;
        if !agrees || !specific || authors.is_empty() || year.is_none() {
            continue;
        }
        // One entry per Work, so every real case has its own ground truth.
        if !seen.insert(truth.to_ascii_lowercase()) {
            continue;
        }
        entries.push(Entry {
            paper,
            title,
            year,
            authors,
            venue: field_text(&fields, "venue"),
            arxiv: field_text(&fields, "arxiv_id"),
            has_doi: field_text(&fields, "doi").is_some(),
            truth,
        });
    }
    Ok(entries)
}

/// Spread entries across papers: shuffle within each, then deal them out round-robin.
fn spread(entries: Vec<Entry>, rng: &mut Rng) -> Vec<Entry> {
    let mut by_paper: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    for entry in entries {
        by_paper.entry(entry.paper.clone()).or_default().push(entry);
    }
    for papers in by_paper.values_mut() {
        rng.shuffle(papers);
    }
    let mut queues: Vec<std::vec::IntoIter<Entry>> =
        by_paper.into_values().map(Vec::into_iter).collect();
    let mut dealt = Vec::new();
    loop {
        let before = dealt.len();
        for queue in &mut queues {
            dealt.extend(queue.next());
        }
        if dealt.len() == before {
            return dealt;
        }
    }
}

fn claim_of(entry: &Entry) -> Claim {
    Claim {
        title: entry.title.clone(),
        authors: entry.authors.clone(),
        year: entry.year,
        venue: entry.venue.clone(),
        ..Claim::default()
    }
}

fn case(id: &str, kind: &str, claim: Claim, truth: Option<&str>) -> ClaimCase {
    ClaimCase {
        id: id.to_owned(),
        kind: kind.to_owned(),
        claim,
        truth: truth.map(str::to_ascii_lowercase),
        source_paper: None,
    }
}

/// Swap two adjacent letters in the middle of the title's longest word.
fn with_typo(title: &str) -> Option<String> {
    let word = title
        .split_whitespace()
        .filter(|word| word.chars().all(char::is_alphabetic))
        .max_by_key(|word| word.chars().count())?;
    let letters: Vec<char> = word.chars().collect();
    if letters.len() < 6 {
        return None;
    }
    let middle = letters.len() / 2;
    let mut typo = letters.clone();
    typo.swap(middle, middle + 1);
    if typo == letters {
        return None;
    }
    Some(title.replacen(word, &typo.iter().collect::<String>(), 1))
}

/// The title before its subtitle, if that leaves a specific head.
fn without_subtitle(title: &str) -> Option<String> {
    let (head, _) = title.split_once(':')?;
    let head = head.trim();
    (head.split_whitespace().count() >= SUBTITLE_HEAD_MIN_WORDS).then(|| head.to_owned())
}

/// `Jordan T Ash` becomes `J. T. Ash`.
fn abbreviate(author: &str) -> String {
    let words: Vec<&str> = author.split_whitespace().collect();
    let Some((family, given)) = words.split_last() else {
        return author.to_owned();
    };
    let mut parts: Vec<String> = given
        .iter()
        .filter_map(|word| word.chars().next())
        .map(|initial| format!("{initial}."))
        .collect();
    parts.push((*family).to_owned());
    parts.join(" ")
}

const INVENTED_ADJECTIVES: [&str; 6] = [
    "Anisotropic",
    "Recursive",
    "Entangled",
    "Heteroscedastic",
    "Modular",
    "Perturbative",
];
const INVENTED_NOUNS: [&str; 6] = [
    "Manifold",
    "Scheduling",
    "Plasmon",
    "Hypergraph",
    "Topology",
    "Reservoir",
];
const INVENTED_DOMAINS: [&str; 6] = [
    "Glacier Retreat",
    "Tidal Turbine Arrays",
    "Enzyme Cascades",
    "Basalt Weathering",
    "Pollinator Networks",
    "Seismic Tomography",
];
const INVENTED_GIVEN: [&str; 6] = ["Marta", "Oskar", "Priya", "Teodor", "Ines", "Kwame"];
const INVENTED_FAMILY: [&str; 6] = [
    "Velasquez",
    "Lindqvist",
    "Raghunathan",
    "Brzezinski",
    "Okonkwo",
    "Haverford",
];

const fn pick<'a>(rng: &mut Rng, list: &[&'a str]) -> &'a str {
    list[rng.below(list.len())]
}

fn fabricated_doi(rng: &mut Rng) -> String {
    format!(
        "10.1145/{}.{}",
        3_000_000 + rng.below(900_000),
        3_000_000 + rng.below(900_000)
    )
}

/// The `OpenAlex` ID of the work an identifier (`doi:...` or an ID) resolves to.
async fn openalex_id<T: JsonTransport>(client: &OpenAlexClient<T>, key: &str) -> Option<String> {
    let value = client.fetch_work_value(key).await.ok()?;
    Some(short_openalex_id(value.get("id")?.as_str()?))
}

/// Ask `OpenAlex` for the Work's DOI. Singleton lookups are free.
async fn openalex_doi<T: JsonTransport>(client: &OpenAlexClient<T>, truth: &str) -> Option<String> {
    let value = client.fetch_work_value(truth).await.ok()?;
    let doi = normalize_doi(value.get("doi")?.as_str()?)?;
    (!doi.starts_with("10.48550/")).then_some(doi)
}

#[allow(clippy::too_many_lines)] // one linear recipe per set; splitting hides the order
async fn build_cases<T: JsonTransport>(
    pool: Vec<Entry>,
    plain: &OpenAlexClient<T>,
    rng: &mut Rng,
) -> Result<CaseSets, Failure> {
    let mut sets = CaseSets::default();
    let papers_of: BTreeMap<String, String> = pool
        .iter()
        .map(|entry| (entry.truth.to_ascii_lowercase(), entry.paper.clone()))
        .collect();
    let mut remaining = pool;

    // DOI entries: the dev vault's parsed entries rarely print a DOI, so the claim carries
    // the DOI `OpenAlex` holds for the resolved Work, as if the entry had printed it.
    let mut doi_cases: Vec<(Entry, String)> = Vec::new();
    let mut index = 0;
    while doi_cases.len() < REAL_DOI && index < remaining.len().min(40) {
        let entry = remaining[index].clone();
        match openalex_doi(plain, &entry.truth).await {
            Some(doi) => {
                remaining.remove(index);
                doi_cases.push((entry, doi));
            }
            None => index += 1,
        }
    }
    let take = |remaining: &mut Vec<Entry>, wanted: usize, keep: &dyn Fn(&Entry) -> bool| {
        let mut taken = Vec::new();
        let mut index = 0;
        while taken.len() < wanted && index < remaining.len() {
            if keep(&remaining[index]) {
                taken.push(remaining.remove(index));
            } else {
                index += 1;
            }
        }
        taken
    };
    // An arXiv ID is only ground truth when `OpenAlex` resolves it to the same record the
    // vault accepted. Preprints that were retitled or duplicated resolve elsewhere, and
    // either record would then be a fair answer, so those entries are left out.
    let mut arxiv_entries = Vec::new();
    let mut arxiv_skipped = 0;
    let mut index = 0;
    while arxiv_entries.len() < REAL_ARXIV && index < remaining.len() {
        let entry = &remaining[index];
        let Some(arxiv) = entry.arxiv.as_deref().filter(|_| !entry.has_doi) else {
            index += 1;
            continue;
        };
        let holder = openalex_id(plain, &format!("doi:10.48550/arxiv.{arxiv}")).await;
        if holder.is_some_and(|id| id.eq_ignore_ascii_case(&entry.truth)) {
            arxiv_entries.push(remaining.remove(index));
        } else {
            eprintln!("skipping arXiv entry {arxiv}: it resolves to another record");
            arxiv_skipped += 1;
            index += 1;
        }
    }
    let bare = |e: &Entry| e.arxiv.is_none() && !e.has_doi;
    let title_entries = take(&mut remaining, REAL_TITLE, &bare);
    let typo_entries = take(&mut remaining, NEAR_TYPO, &|e| {
        bare(e) && with_typo(&e.title).is_some()
    });
    let subtitle_entries = take(&mut remaining, NEAR_SUBTITLE, &|e| {
        bare(e) && without_subtitle(&e.title).is_some()
    });
    let abbreviated_entries = take(&mut remaining, NEAR_ABBREVIATED, &bare);

    for (number, (entry, doi)) in doi_cases.iter().enumerate() {
        let claim = Claim {
            doi: Some(doi.clone()),
            ..claim_of(entry)
        };
        sets.real.push(case(
            &format!("real-doi-{number:02}"),
            "doi",
            claim,
            Some(&entry.truth),
        ));
    }
    for (number, entry) in arxiv_entries.iter().enumerate() {
        let claim = Claim {
            arxiv: entry.arxiv.clone(),
            ..claim_of(entry)
        };
        sets.real.push(case(
            &format!("real-arxiv-{number:02}"),
            "arxiv",
            claim,
            Some(&entry.truth),
        ));
    }
    for (number, entry) in title_entries.iter().enumerate() {
        sets.real.push(case(
            &format!("real-title-{number:02}"),
            "title",
            claim_of(entry),
            Some(&entry.truth),
        ));
    }

    for (number, entry) in typo_entries.iter().enumerate() {
        let claim = Claim {
            title: with_typo(&entry.title).unwrap_or_default(),
            ..claim_of(entry)
        };
        sets.near_miss.push(case(
            &format!("near-typo-{number:02}"),
            "typo",
            claim,
            Some(&entry.truth),
        ));
    }
    for (number, entry) in subtitle_entries.iter().enumerate() {
        let claim = Claim {
            title: without_subtitle(&entry.title).unwrap_or_default(),
            ..claim_of(entry)
        };
        sets.near_miss.push(case(
            &format!("near-subtitle-{number:02}"),
            "dropped_subtitle",
            claim,
            Some(&entry.truth),
        ));
    }
    for (number, entry) in abbreviated_entries.iter().enumerate() {
        let claim = Claim {
            authors: entry
                .authors
                .iter()
                .map(|author| abbreviate(author))
                .collect(),
            ..claim_of(entry)
        };
        sets.near_miss.push(case(
            &format!("near-authors-{number:02}"),
            "abbreviated_authors",
            claim,
            Some(&entry.truth),
        ));
    }

    // Fakes are built on the real set, so a fake's title usually has its search cached.
    let real_titles: Vec<(Entry, &ClaimCase)> = title_entries
        .iter()
        .cloned()
        .zip(sets.real.iter().filter(|c| c.kind == "title"))
        .collect();
    let mut fakes: Vec<ClaimCase> = Vec::new();
    for (number, (entry, _)) in real_titles.iter().take(FAKE_FABRICATED_DOI).enumerate() {
        let claim = Claim {
            doi: Some(fabricated_doi(rng)),
            ..claim_of(entry)
        };
        fakes.push(case(
            &format!("fake-fabricated-doi-{number:02}"),
            "fabricated_doi",
            claim,
            None,
        ));
    }
    for (number, ((source, doi), (other, _))) in doi_cases
        .iter()
        .zip(real_titles.iter())
        .take(FAKE_WRONG_TITLE)
        .enumerate()
    {
        // The DOI and authors are one real paper's; the title and year are another's.
        let claim = Claim {
            title: other.title.clone(),
            year: other.year,
            doi: Some(doi.clone()),
            ..claim_of(source)
        };
        fakes.push(case(
            &format!("fake-wrong-title-{number:02}"),
            "real_doi_wrong_title",
            claim,
            None,
        ));
    }
    let titles: Vec<&Entry> = real_titles.iter().map(|(entry, _)| entry).collect();
    for number in 0..FAKE_RECOMBINED.min(titles.len().saturating_sub(2)) {
        let (a, b, c) = (titles[number], titles[number + 1], titles[number + 2]);
        let words = |entry: &Entry| {
            entry
                .title
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let (wa, wb, wc) = (words(a), words(b), words(c));
        let mut mixed: Vec<String> = wa.iter().take(3).cloned().collect();
        let middle = wb.len() / 2;
        mixed.extend(wb.iter().skip(middle).take(3).cloned());
        mixed.extend(wc.iter().rev().take(3).rev().cloned());
        let claim = Claim {
            title: mixed.join(" "),
            ..claim_of(a)
        };
        fakes.push(case(
            &format!("fake-recombined-{number:02}"),
            "recombined_title",
            claim,
            None,
        ));
    }
    for (number, entry) in titles.iter().take(FAKE_YEAR_OFF).enumerate() {
        let offset = if number % 2 == 0 { 4 } else { -4 };
        let claim = Claim {
            year: entry.year.map(|year| year + offset),
            ..claim_of(entry)
        };
        fakes.push(case(
            &format!("fake-year-off-{number:02}"),
            "year_off",
            claim,
            None,
        ));
    }
    for (number, entry) in titles.iter().take(FAKE_AUTHORS_SWAPPED).enumerate() {
        let other = titles[(number + titles.len() / 2 + 1) % titles.len()];
        let claim = Claim {
            authors: other.authors.clone(),
            ..claim_of(entry)
        };
        fakes.push(case(
            &format!("fake-authors-swapped-{number:02}"),
            "authors_swapped",
            claim,
            None,
        ));
    }
    for number in 0..FAKE_INVENTED {
        let title = format!(
            "{} {} Methods for {}",
            pick(rng, &INVENTED_ADJECTIVES),
            pick(rng, &INVENTED_NOUNS),
            pick(rng, &INVENTED_DOMAINS)
        );
        let authors = (0..2)
            .map(|_| {
                format!(
                    "{} {}",
                    pick(rng, &INVENTED_GIVEN),
                    pick(rng, &INVENTED_FAMILY)
                )
            })
            .collect();
        let claim = Claim {
            title,
            authors,
            year: Some(2015 + year_offset(rng)),
            ..Claim::default()
        };
        fakes.push(case(
            &format!("fake-invented-{number:02}"),
            "invented",
            claim,
            None,
        ));
    }
    sets.fake = fakes;
    for case in sets.real.iter_mut().chain(sets.near_miss.iter_mut()) {
        case.source_paper = case
            .truth
            .as_ref()
            .and_then(|truth| papers_of.get(truth))
            .cloned();
    }
    let mut per_paper: BTreeMap<&str, usize> = BTreeMap::new();
    for paper in sets
        .real
        .iter()
        .chain(&sets.near_miss)
        .filter_map(|case| case.source_paper.as_deref())
    {
        *per_paper.entry(paper).or_default() += 1;
    }
    eprintln!(
        "cases come from {} papers; the largest contributes {} (the spec asks for at least 10 papers and at most 5 each, which the dev vault cannot supply)",
        per_paper.len(),
        per_paper.values().max().copied().unwrap_or(0)
    );
    eprintln!(
        "{arxiv_skipped} arXiv entries were left out of the real set because OpenAlex resolves their arXiv DOI to a different record than the vault accepted"
    );
    Ok(sets)
}

/// Lets the runner own a transport the caller can still read afterwards.
struct Shared<T>(Arc<T>);

impl<T: JsonTransport> JsonTransport for Shared<T> {
    fn get_json<'a>(
        &'a self,
        url: &'a str,
    ) -> ProviderFuture<'a, Result<Value, lysilogy_ingest::openalex::ProviderError>> {
        self.0.get_json(url)
    }
}

fn read_responses(path: &Path) -> Responses {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_json<V: serde::Serialize>(path: &Path, value: &V) -> Result<(), Failure> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    std::fs::write(path, text)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let args = parse_args()?;
    match args.set {
        SetKind::Base => run_base(&args).await,
        SetKind::Holdout => run_holdout(&args).await,
    }
}

async fn run_base(args: &Args) -> Result<(), Failure> {
    let cases_path = args.out.join("cases.json");
    let responses_path = args.out.join("responses.json");
    if (cases_path.exists() || responses_path.exists()) && !args.rerecord {
        return Err(format!(
            "{} already holds fixtures; pass --rerecord to replace them",
            args.out.display()
        )
        .into());
    }

    let entries = load_entries(&args.dev_url).await?;
    eprintln!("{} eligible bibliography entries", entries.len());
    let mut rng = Rng(0x00C1_A1D5);
    let pool = spread(entries, &mut rng);
    let plain = OpenAlexClient::new(ReqwestTransport::from_environment()?);
    let sets = build_cases(pool, &plain, &mut rng).await?;
    eprintln!(
        "built {} real, {} near-miss, {} fake cases",
        sets.real.len(),
        sets.near_miss.len(),
        sets.fake.len()
    );

    let seed = read_responses(&args.scratch_file);
    eprintln!("resuming from {} recorded responses", seed.len());
    let capture = Arc::new(CapturingTransport::new(
        ReqwestTransport::from_environment()?,
        seed,
        args.max_searches,
    ));
    let scratch = ScratchDatabase::create(&args.test_url).await?;
    let recorded = run_cases(&scratch.pool, Shared(Arc::clone(&capture)), &sets).await;
    scratch.drop_database().await?;
    write_json(&args.scratch_file, &capture.captured())?;
    let report = recorded?;
    eprintln!(
        "{} live requests, {} live title searches",
        capture.live_requests(),
        capture.paid_requests()
    );
    if capture.exhausted() {
        return Err(format!(
            "the request budget ran out; {} responses are kept in {}, rerun to continue",
            capture.captured().len(),
            args.scratch_file.display()
        )
        .into());
    }
    eprintln!("{}", report.render());

    // Prove the recording is complete: replay it in a fresh scratch database.
    let replay = ReplayTransport::new(capture.captured());
    let scratch = ScratchDatabase::create(&args.test_url).await?;
    let replayed = run_cases(&scratch.pool, replay.clone(), &sets).await;
    scratch.drop_database().await?;
    let replayed = replayed?;
    eprintln!("replay: {}", replayed.render());
    if !replay.misses().is_empty() {
        return Err(format!("replay missed {:?}", replay.misses()).into());
    }

    // Only a recording that replays completely is written, so a failed run leaves nothing
    // that a later run would refuse to overwrite.
    write_json(&cases_path, &sets)?;
    write_json(&responses_path, &capture.captured())?;
    Ok(())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // below 8
const fn year_offset(rng: &mut Rng) -> i32 {
    rng.below(8) as i32
}

// ---------------------------------------------------------------------------------------
// Held-out sets (`--set holdout`).
//
// E1 replays real citations the base fixtures never touched; E3 is hand-specified hard
// cases. Each part writes `<out>/e1` or `<out>/e3`:
//
// - `cases.planned.json`: the whole plan, so a later run needs no database and cannot drift;
//   `--replan` rebuilds it. `order` lists the recording order, cheapest cases first.
// - `cases.json` and `responses.json`: the cases whose recording is complete, and the
//   responses that replay them. They match the base layout, so `claims_support` loads them.
//
// `OpenAlex`'s free tier allows about 100 title searches a day, so a part may not finish in
// one run. The run stops when `--max-searches` is spent (or on a 429), keeps every response
// in `~/.cache/lysilogy-scratch/claims-holdout-<part>-responses.json`, and writes the cases
// recorded so far. Run it again with `--rerecord` after the budget resets: recorded cases
// replay from that file at no cost and the run goes on where it stopped. The verifier is not
// touched; only recording is added here.
// ---------------------------------------------------------------------------------------

const HOLDOUT_SEED: u64 = 0x5EED_E1CA;
const HOLDOUT_DEFAULT_SEARCHES: usize = 90;
const E1_REAL: usize = 150;
const E1_FAKE: usize = 150;
const E1_NEAR: usize = 50;
const E1_MAX_PER_PAPER: usize = 8;
const E1_DOI_PERCENT: usize = 30;
/// A planner-style claim names at most this many authors.
const CLAIM_AUTHORS_MAX: usize = 3;
const HOLDOUT_MIN_TITLE_WORDS: usize = 3;
const HOLDOUT_MIN_TITLE_CHARS: usize = 15;
const HOLDOUT_FABRICATED_DOI_PREFIX: &str = "10.1145/";
const ARXIV_DOI_PREFIX: &str = "10.48550/";
/// The planned truth title of an E3 case must match the title `OpenAlex` holds this well.
const E3_TRUTH_TITLE_MIN_SIMILARITY: f64 = 0.9;
const LOOKUP_PAUSE: Duration = Duration::from_millis(150);
const WORK_SELECT: &str =
    "id,doi,display_name,publication_year,type,authorships,primary_location,is_retracted";

/// A Work as `OpenAlex` describes it, with the fields a claim is built from.
#[derive(Clone, Debug)]
struct OaRecord {
    id: String,
    title: String,
    year: Option<i32>,
    doi: Option<String>,
    authors: Vec<String>,
    venue: Option<String>,
}

fn parse_record(value: &Value) -> Option<OaRecord> {
    let title = value.get("display_name")?.as_str()?.trim().to_owned();
    let authors = value
        .get("authorships")
        .and_then(Value::as_array)
        .map(|authorships| {
            authorships
                .iter()
                .filter_map(|authorship| authorship.pointer("/author/display_name")?.as_str())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Some(OaRecord {
        id: short_openalex_id(value.get("id")?.as_str()?),
        title,
        year: value
            .get("publication_year")
            .and_then(Value::as_i64)
            .and_then(|year| i32::try_from(year).ok()),
        doi: value
            .get("doi")
            .and_then(Value::as_str)
            .and_then(normalize_doi),
        authors,
        venue: value
            .pointer("/primary_location/source/display_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn single_work_url(key: &str) -> String {
    let target = key
        .strip_prefix("doi:")
        .map_or_else(|| key.to_owned(), |doi| format!("https://doi.org/{doi}"));
    format!(
        "https://api.openalex.org/works/{}?select={WORK_SELECT}",
        utf8_percent_encode(&target, NON_ALPHANUMERIC)
    )
}

/// Free singleton lookups, kept in a file so a rerun asks `OpenAlex` nothing it was asked.
struct OaLookup {
    transport: ReqwestTransport,
    cache: BTreeMap<String, Value>,
    path: PathBuf,
}

impl OaLookup {
    fn open(path: PathBuf) -> Result<Self, Failure> {
        let cache = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Ok(Self {
            transport: ReqwestTransport::from_environment()?,
            cache,
            path,
        })
    }

    /// The Work for an `OpenAlex` ID or `doi:...`; `None` when `OpenAlex` has no such Work.
    async fn get(&mut self, key: &str) -> Result<Option<OaRecord>, Failure> {
        let key = key.to_ascii_lowercase();
        if let Some(cached) = self.cache.get(&key) {
            return Ok(parse_record(cached));
        }
        tokio::time::sleep(LOOKUP_PAUSE).await;
        let found = match self.transport.get_json(&single_work_url(&key)).await {
            Ok(value) => value,
            Err(ProviderError::Status(404)) => Value::Null,
            Err(error) => {
                self.save()?;
                return Err(format!("OpenAlex lookup of {key} failed: {error:?}").into());
            }
        };
        let record = parse_record(&found);
        self.cache.insert(key, found);
        Ok(record)
    }

    fn save(&self) -> Result<(), Failure> {
        write_json(&self.path, &self.cache)
    }
}

/// What the base fixtures used; held-out cases must avoid all of it.
#[derive(Default)]
struct Exclusions {
    ids: HashSet<String>,
    dois: HashSet<String>,
    titles: HashSet<String>,
}

fn load_exclusions(path: &Path) -> Result<Exclusions, Failure> {
    let sets: CaseSets = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut exclusions = Exclusions::default();
    for (_, case) in sets.in_run_order() {
        exclusions.ids.extend(case.truth.clone());
        exclusions
            .dois
            .extend(case.claim.doi.as_deref().and_then(normalize_doi));
        exclusions.titles.insert(title_key(&case.claim.title));
    }
    Ok(exclusions)
}

/// Whether a Work could be in the held-out set: titled, authored, dated, not excluded and
/// specific enough that its title does not name hundreds of papers.
fn holdout_eligible(record: &OaRecord, exclusions: &Exclusions) -> bool {
    let key = title_key(&record.title);
    record.year.is_some()
        && !record.authors.is_empty()
        && !record.title.contains('<')
        && record.title.chars().count() >= HOLDOUT_MIN_TITLE_CHARS
        && record.title.split_whitespace().count() >= HOLDOUT_MIN_TITLE_WORDS
        && !exclusions.ids.contains(&record.id.to_ascii_lowercase())
        && !exclusions.titles.contains(&key)
        && !record
            .doi
            .as_ref()
            .is_some_and(|doi| exclusions.dois.contains(doi))
}

/// A planner-style author list: the first few surnames.
fn surnames(authors: &[String]) -> Vec<String> {
    authors
        .iter()
        .filter_map(|author| citation_family(author))
        .take(CLAIM_AUTHORS_MAX)
        .collect()
}

fn full_names(authors: &[String]) -> Vec<String> {
    authors.iter().take(CLAIM_AUTHORS_MAX).cloned().collect()
}

/// A planner-style claim: title, surnames, year, venue, and a DOI when asked.
fn planner_claim(record: &OaRecord, with_doi: bool) -> Claim {
    Claim {
        title: record.title.clone(),
        authors: surnames(&record.authors),
        year: record.year,
        venue: record.venue.clone(),
        doi: with_doi.then(|| record.doi.clone()).flatten(),
        ..Claim::default()
    }
}

/// Every `(cited OpenAlex ID, citing Work)` pair from vault papers' citations.
async fn load_citation_pairs(url: &str) -> Result<Vec<(String, String)>, Failure> {
    let pool = lysilogy_db::connect(url).await?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "select distinct i.value_normalized, ca.citing_work_id::text \
         from graph.citation_assertion ca \
         join reader.document d on d.work_id = ca.citing_work_id \
         join graph.work w on w.id = ca.cited_work_id \
         join graph.work_identifier i \
           on i.work_id = w.id and i.scheme = 'openalex' and i.status = 'active' \
         where ca.status in ('asserted', 'accepted') \
           and w.display_title is not null and btrim(w.display_title) <> '' \
         order by 1, 2",
    )
    .fetch_all(&pool)
    .await?;
    Ok(rows)
}

/// Candidates in recording order: each cited Work goes to one random citing paper, each
/// paper keeps at most `cap` of its Works, and the papers are dealt round-robin, so any
/// prefix of the list is spread across papers.
fn deal_candidates(
    pairs: Vec<(String, String)>,
    cap: usize,
    rng: &mut Rng,
) -> Vec<(String, String)> {
    let mut citers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (cited, citing) in pairs {
        citers.entry(cited).or_default().push(citing);
    }
    let mut by_paper: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (cited, papers) in citers {
        let paper = papers[rng.below(papers.len())].clone();
        by_paper.entry(paper).or_default().push(cited);
    }
    for works in by_paper.values_mut() {
        rng.shuffle(works);
        works.truncate(cap);
    }
    let mut queues: Vec<(String, std::vec::IntoIter<String>)> = by_paper
        .into_iter()
        .map(|(paper, works)| (paper, works.into_iter()))
        .collect();
    let mut dealt = Vec::new();
    loop {
        let before = dealt.len();
        for (paper, queue) in &mut queues {
            dealt.extend(queue.next().map(|cited| (cited, paper.clone())));
        }
        if dealt.len() == before {
            return dealt;
        }
    }
}

#[derive(Clone, Debug)]
struct Picked {
    record: OaRecord,
    paper: String,
}

/// Remove and return up to `wanted` picks that pass `keep`.
fn take_picks(
    pool: &mut Vec<Picked>,
    wanted: usize,
    keep: &dyn Fn(&Picked) -> bool,
) -> Vec<Picked> {
    let mut taken = Vec::new();
    let mut index = 0;
    while taken.len() < wanted && index < pool.len() {
        if keep(&pool[index]) {
            taken.push(pool.remove(index));
        } else {
            index += 1;
        }
    }
    taken
}

fn can_abbreviate(record: &OaRecord) -> bool {
    record
        .authors
        .first()
        .is_some_and(|author| author.split_whitespace().count() >= 2)
}

/// The split of a number of fakes into the six kinds, in kind order: an even share each,
/// with the remainder going to the first kinds.
fn fake_shares(total: usize) -> [usize; 6] {
    let mut shares = [total / 6; 6];
    for share in shares.iter_mut().take(total % 6) {
        *share += 1;
    }
    shares
}

/// A claim with a title and nothing else usable made from invented parts.
fn invented_claim(rng: &mut Rng, exclusions: &Exclusions, used: &mut HashSet<String>) -> Claim {
    loop {
        let title = format!(
            "{} {} Methods for {}",
            pick(rng, &INVENTED_ADJECTIVES),
            pick(rng, &INVENTED_NOUNS),
            pick(rng, &INVENTED_DOMAINS)
        );
        let key = title_key(&title);
        if exclusions.titles.contains(&key) || !used.insert(key) {
            continue;
        }
        let authors = (0..2)
            .map(|_| {
                format!(
                    "{} {}",
                    pick(rng, &INVENTED_GIVEN),
                    pick(rng, &INVENTED_FAMILY)
                )
            })
            .collect();
        return Claim {
            title,
            authors,
            year: Some(2015 + year_offset(rng)),
            ..Claim::default()
        };
    }
}

/// The plan, as written to `cases.planned.json`.
struct Plan {
    value: Value,
    sets: CaseSets,
    order: Vec<String>,
}

impl Plan {
    fn from_value(value: Value) -> Result<Self, Failure> {
        let sets = serde_json::from_value(value.clone())?;
        let order = value
            .get("order")
            .and_then(Value::as_array)
            .ok_or("the plan has no order")?
            .iter()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .collect();
        Ok(Self { value, sets, order })
    }

    /// The planned cases in `done`, extra fields and all, in set order.
    fn recorded(&self, done: &BTreeSet<String>) -> Value {
        let mut out = serde_json::Map::new();
        for set in ["real", "fake", "near_miss"] {
            let kept: Vec<Value> = self
                .value
                .get(set)
                .and_then(Value::as_array)
                .map(|cases| {
                    cases
                        .iter()
                        .filter(|case| {
                            case.get("id")
                                .and_then(Value::as_str)
                                .is_some_and(|id| done.contains(id))
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            out.insert(set.to_owned(), Value::Array(kept));
        }
        Value::Object(out)
    }
}

/// Serialize the sets, merge each case's extra fields in, and add the recording order.
fn plan_value(
    sets: &CaseSets,
    extras: &BTreeMap<String, Value>,
    order: &[String],
    meta: Value,
) -> Result<Value, Failure> {
    let mut value = serde_json::to_value(sets)?;
    for set in ["real", "fake", "near_miss"] {
        let Some(cases) = value.get_mut(set).and_then(Value::as_array_mut) else {
            continue;
        };
        for case in cases {
            let extra = case
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| extras.get(id));
            if let (Some(Value::Object(extra)), Some(case)) = (extra, case.as_object_mut()) {
                case.extend(extra.clone());
            }
        }
    }
    if let Some(object) = value.as_object_mut() {
        object.insert("order".to_owned(), serde_json::to_value(order)?);
        object.insert("meta".to_owned(), meta);
    }
    Ok(value)
}

#[allow(clippy::too_many_lines)] // one linear recipe; splitting hides the order
async fn plan_e1(args: &Args, oa: &mut OaLookup) -> Result<Value, Failure> {
    let exclusions = load_exclusions(&args.exclude)?;
    eprintln!(
        "excluding {} OpenAlex IDs, {} DOIs and {} titles used by {}",
        exclusions.ids.len(),
        exclusions.dois.len(),
        exclusions.titles.len(),
        args.exclude.display()
    );
    let mut rng = Rng(args.seed);
    let pairs = load_citation_pairs(&args.dev_url).await?;
    let dealt = deal_candidates(pairs, args.max_per_paper, &mut rng);
    eprintln!(
        "{} candidate cited Works across the citing papers",
        dealt.len()
    );

    let near_subtitle = args.near / 3;
    let near_typo = args.near / 3;
    let near_abbreviated = args.near - near_subtitle - near_typo;
    let mut qualified: Vec<Picked> = Vec::new();
    let mut seen_titles = HashSet::new();
    let mut merged_away = 0;
    for (cited, paper) in dealt {
        let enough = qualified.len() >= args.real + args.near
            && qualified
                .iter()
                .filter(|picked| without_subtitle(&picked.record.title).is_some())
                .count()
                >= near_subtitle;
        if enough {
            break;
        }
        let Some(record) = oa.get(&cited).await? else {
            continue;
        };
        // An ID that OpenAlex has merged into another Work has no stable ground truth.
        if !record.id.eq_ignore_ascii_case(&cited) {
            merged_away += 1;
            continue;
        }
        if !holdout_eligible(&record, &exclusions) || !seen_titles.insert(title_key(&record.title))
        {
            continue;
        }
        qualified.push(Picked { record, paper });
    }
    oa.save()?;
    eprintln!(
        "{} eligible Works ({merged_away} set aside as merged IDs)",
        qualified.len()
    );

    let mut pool = qualified;
    let subtitle = take_picks(&mut pool, near_subtitle, &|picked| {
        without_subtitle(&picked.record.title).is_some()
    });
    let typo = take_picks(&mut pool, near_typo, &|picked| {
        with_typo(&picked.record.title).is_some()
    });
    let abbreviated = take_picks(&mut pool, near_abbreviated, &|picked| {
        can_abbreviate(&picked.record)
    });
    pool.truncate(args.real);
    let reals = pool;
    if reals.len() < args.real
        || subtitle.len() < near_subtitle
        || typo.len() < near_typo
        || abbreviated.len() < near_abbreviated
    {
        return Err(format!(
            "too few eligible Works: {} real, {}/{}/{} near-miss (subtitle/typo/abbreviated)",
            reals.len(),
            subtitle.len(),
            typo.len(),
            abbreviated.len()
        )
        .into());
    }

    // About `doi_percent` of the real cases carry the Work's DOI.
    let wanted_doi = reals.len() * args.doi_percent / 100;
    let mut with_doi: Vec<usize> = reals
        .iter()
        .enumerate()
        .filter(|(_, picked)| {
            picked
                .record
                .doi
                .as_deref()
                .is_some_and(|doi| !doi.starts_with(ARXIV_DOI_PREFIX))
        })
        .map(|(index, _)| index)
        .collect();
    rng.shuffle(&mut with_doi);
    with_doi.truncate(wanted_doi);
    let doi_set: HashSet<usize> = with_doi.into_iter().collect();

    let mut sets = CaseSets::default();
    let mut extras: BTreeMap<String, Value> = BTreeMap::new();
    let mut doi_reals: Vec<(Picked, String)> = Vec::new();
    let mut title_reals: Vec<(Picked, String)> = Vec::new();
    let (mut doi_count, mut title_count) = (0, 0);
    for (index, picked) in reals.iter().enumerate() {
        let is_doi = doi_set.contains(&index);
        let (kind, number) = if is_doi {
            ("doi", &mut doi_count)
        } else {
            ("title", &mut title_count)
        };
        let id = format!("e1-real-{kind}-{number:03}");
        *number += 1;
        let mut case = case(
            &id,
            kind,
            planner_claim(&picked.record, is_doi),
            Some(&picked.record.id),
        );
        case.source_paper = Some(picked.paper.clone());
        sets.real.push(case);
        if is_doi {
            doi_reals.push((picked.clone(), id));
        } else {
            title_reals.push((picked.clone(), id));
        }
    }

    let near_sources: [(&str, &str, &Vec<Picked>); 3] = [
        ("typo", "typo", &typo),
        ("subtitle", "dropped_subtitle", &subtitle),
        ("authors", "abbreviated_authors", &abbreviated),
    ];
    let mut near_streams: Vec<Vec<String>> = Vec::new();
    for (label, kind, picks) in near_sources {
        let mut stream = Vec::new();
        for (number, picked) in picks.iter().enumerate() {
            let mut claim = planner_claim(&picked.record, false);
            match kind {
                "typo" => claim.title = with_typo(&picked.record.title).unwrap_or_default(),
                "dropped_subtitle" => {
                    claim.title = without_subtitle(&picked.record.title).unwrap_or_default();
                }
                _ => {
                    claim.authors = full_names(&picked.record.authors)
                        .iter()
                        .map(|author| abbreviate(author))
                        .collect();
                }
            }
            let id = format!("e1-near-{label}-{number:03}");
            let mut case = case(&id, kind, claim, Some(&picked.record.id));
            case.source_paper = Some(picked.paper.clone());
            sets.near_miss.push(case);
            stream.push(id);
        }
        near_streams.push(stream);
    }

    // Fakes are perturbations of the real cases, so a fake's title usually has its search
    // cached from the real case it came from.
    let [n_doi, n_wrong, n_recombined, n_year, n_swapped, n_invented] = fake_shares(args.fake);
    let titles = &title_reals;
    if titles.len() < 3 || doi_reals.is_empty() {
        return Err("too few real cases to perturb".into());
    }
    let wrap = |index: usize| index % titles.len();
    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut group_of: Vec<Vec<String>> = vec![Vec::new(); titles.len()];
    let mut loose: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut fake = |kind: &'static str, number: usize, claim: Claim, base: Option<&str>| {
        let id = format!("e1-fake-{}-{number:03}", kind.replace('_', "-"));
        let fake_case = case(&id, kind, claim, None);
        extras.insert(id.clone(), serde_json::json!({ "perturbation_of": base }));
        sets.fake.push(fake_case);
        id
    };
    for number in 0..n_doi {
        let (base, base_id) = &titles[wrap(number)];
        let claim = Claim {
            doi: Some(format!(
                "{HOLDOUT_FABRICATED_DOI_PREFIX}{}.{}",
                3_000_000 + rng.below(900_000),
                3_000_000 + rng.below(900_000)
            )),
            ..planner_claim(&base.record, false)
        };
        group_of[wrap(number)].push(fake("fabricated_doi", number, claim, Some(base_id)));
    }
    for number in 0..n_wrong {
        let (source, source_id) = &doi_reals[number % doi_reals.len()];
        let (other, _) = &titles[wrap(number)];
        // The DOI and authors are one real paper's; the title and year are another's.
        let claim = Claim {
            title: other.record.title.clone(),
            year: other.record.year,
            ..planner_claim(&source.record, true)
        };
        group_of[wrap(number)].push(fake("real_doi_wrong_title", number, claim, Some(source_id)));
    }
    for number in 0..n_swapped {
        let (base, base_id) = &titles[wrap(number)];
        let (other, _) = &titles[(number + titles.len() / 2 + 1) % titles.len()];
        let claim = Claim {
            authors: surnames(&other.record.authors),
            ..planner_claim(&base.record, false)
        };
        group_of[wrap(number)].push(fake("authors_swapped", number, claim, Some(base_id)));
    }
    for number in 0..n_recombined {
        let offset = titles.len() / 3;
        let words = |index: usize| -> Vec<String> {
            titles[wrap(index)]
                .0
                .record
                .title
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        };
        let (wa, wb, wc) = (
            words(offset + number),
            words(offset + number + 1),
            words(offset + number + 2),
        );
        let mut mixed: Vec<String> = wa.iter().take(3).cloned().collect();
        mixed.extend(wb.iter().skip(wb.len() / 2).take(3).cloned());
        mixed.extend(wc.iter().rev().take(3).rev().cloned());
        let claim = Claim {
            title: mixed.join(" "),
            ..planner_claim(&titles[wrap(offset + number)].0.record, false)
        };
        let id = fake("recombined_title", number, claim, None);
        loose.entry("recombined").or_default().push(id);
    }
    for number in 0..n_year {
        let (base, base_id) = &titles[wrap(number * 2 + 1)];
        let delta = if number % 2 == 0 { 4 } else { -4 };
        let claim = Claim {
            year: base.record.year.map(|year| year + delta),
            ..planner_claim(&base.record, false)
        };
        let id = fake("year_off", number, claim, Some(base_id));
        loose.entry("year").or_default().push(id);
    }
    let mut used_titles = HashSet::new();
    for number in 0..n_invented {
        let claim = invented_claim(&mut rng, &exclusions, &mut used_titles);
        let id = fake("invented", number, claim, None);
        loose.entry("invented").or_default().push(id);
    }
    for ids in group_of {
        groups.push(ids);
    }

    // Recording order: the cases that cost no title search first, then each real case with
    // the fakes built on it (their searches are shared), then the rest round-robin.
    let mut order: Vec<String> = doi_reals.iter().map(|(_, id)| id.clone()).collect();
    let grouped = groups.iter().filter(|ids| !ids.is_empty()).count();
    for (index, (_, id)) in titles.iter().enumerate().take(grouped) {
        order.push(id.clone());
        order.extend(groups[index].iter().cloned());
    }
    let mut streams: Vec<std::vec::IntoIter<String>> = Vec::new();
    streams.push(
        titles
            .iter()
            .skip(grouped)
            .map(|(_, id)| id.clone())
            .collect::<Vec<_>>()
            .into_iter(),
    );
    for stream in near_streams {
        streams.push(stream.into_iter());
    }
    for key in ["year", "recombined", "invented"] {
        streams.push(loose.remove(key).unwrap_or_default().into_iter());
    }
    loop {
        let before = order.len();
        for stream in &mut streams {
            order.extend(stream.next());
        }
        if order.len() == before {
            break;
        }
    }

    let mut per_paper: BTreeMap<&str, usize> = BTreeMap::new();
    for paper in sets
        .real
        .iter()
        .chain(&sets.near_miss)
        .filter_map(|case| case.source_paper.as_deref())
    {
        *per_paper.entry(paper).or_default() += 1;
    }
    eprintln!(
        "E1 plan: {} real ({} with a DOI), {} near-miss, {} fake; {} citing papers, at most {} each",
        sets.real.len(),
        doi_reals.len(),
        sets.near_miss.len(),
        sets.fake.len(),
        per_paper.len(),
        per_paper.values().max().copied().unwrap_or(0)
    );
    plan_value(
        &sets,
        &extras,
        &order,
        serde_json::json!({
            "part": "e1",
            "seed": args.seed,
            "citing_papers": per_paper.len(),
            "max_per_paper": per_paper.values().max().copied().unwrap_or(0),
        }),
    )
}

/// One hand-specified E3 case, as written in `spec.json`.
#[derive(serde::Deserialize)]
struct E3Spec {
    id: String,
    kind: String,
    /// `accept` or `must_not_accept`.
    expect: String,
    claim: Claim,
    truth_id: Option<String>,
    truth_title: Option<String>,
    #[serde(default)]
    alt_truth: Vec<String>,
    note: Option<String>,
}

/// Resolve and check every E3 case's ground truth against `OpenAlex`.
async fn plan_e3(args: &Args, oa: &mut OaLookup) -> Result<Value, Failure> {
    let spec_path = args
        .spec
        .clone()
        .unwrap_or_else(|| args.out.join("e3/spec.json"));
    let specs: Vec<E3Spec> = serde_json::from_slice(&std::fs::read(&spec_path)?)?;
    let mut sets = CaseSets::default();
    let mut extras = BTreeMap::new();
    let mut order = Vec::new();
    let mut problems = Vec::new();
    for spec in specs {
        let mut extra = serde_json::json!({ "expected": spec.expect, "note": spec.note });
        let mut truth = None;
        match spec.expect.as_str() {
            "accept" => {
                let (Some(id), Some(expected_title)) = (&spec.truth_id, &spec.truth_title) else {
                    problems.push(format!(
                        "{}: an accept case needs truth_id and truth_title",
                        spec.id
                    ));
                    continue;
                };
                let Some(record) = oa.get(id).await? else {
                    problems.push(format!("{}: OpenAlex has no Work {id}", spec.id));
                    continue;
                };
                let fit = title_similarity(expected_title, &record.title);
                eprintln!(
                    "{:<22} {:<10} {:<48} {:?} sim(claim) {:.2}",
                    spec.id,
                    record.id,
                    record.title.chars().take(48).collect::<String>(),
                    record.year,
                    title_similarity(&spec.claim.title, &record.title)
                );
                if !record.id.eq_ignore_ascii_case(id) || fit < E3_TRUTH_TITLE_MIN_SIMILARITY {
                    problems.push(format!(
                        "{}: {id} is {:?} on OpenAlex, not the expected title",
                        spec.id, record.title
                    ));
                }
                let mut acceptable = vec![record.id.to_ascii_lowercase()];
                for alt in &spec.alt_truth {
                    if oa.get(alt).await?.is_none() {
                        problems.push(format!("{}: OpenAlex has no alternate {alt}", spec.id));
                    }
                    acceptable.push(alt.to_ascii_lowercase());
                }
                if let Some(doi) = spec.claim.doi.as_deref().and_then(normalize_doi) {
                    let resolved = oa.get(&format!("doi:{doi}")).await?;
                    if !resolved
                        .is_some_and(|work| acceptable.contains(&work.id.to_ascii_lowercase()))
                    {
                        problems.push(format!(
                            "{}: the claimed DOI {doi} is not the truth",
                            spec.id
                        ));
                    }
                }
                extra["truth_title"] = Value::String(record.title.clone());
                extra["truth_year"] = serde_json::json!(record.year);
                extra["alt_truth"] = serde_json::json!(acceptable[1..]);
                truth = Some(record.id);
            }
            "must_not_accept" => {}
            other => problems.push(format!("{}: unknown expect {other}", spec.id)),
        }
        extras.insert(spec.id.clone(), extra);
        let planned = case(&spec.id, &spec.kind, spec.claim, truth.as_deref());
        if truth.is_some() {
            sets.real.push(planned);
        } else {
            sets.fake.push(planned);
        }
        order.push(spec.id);
    }
    oa.save()?;
    if !problems.is_empty() {
        return Err(format!(
            "E3 ground truth does not check out:\n{}",
            problems.join("\n")
        )
        .into());
    }
    eprintln!(
        "E3 plan: {} accept, {} must-not-accept",
        sets.real.len(),
        sets.fake.len()
    );
    plan_value(
        &sets,
        &extras,
        &order,
        serde_json::json!({ "part": "e3", "spec": spec_path.display().to_string() }),
    )
}

struct Recorded {
    done: BTreeSet<String>,
    responses: Responses,
    complete: bool,
}

/// Record the plan's cases one at a time, in plan order, until the budget runs out. A case
/// is kept only when every request it made was served.
async fn record_plan(args: &Args, plan: &Plan, scratch_file: &Path) -> Result<Recorded, Failure> {
    let seed = read_responses(scratch_file);
    eprintln!("resuming from {} recorded responses", seed.len());
    let capture = Arc::new(CapturingTransport::new(
        ReqwestTransport::from_environment()?,
        seed.clone(),
        args.max_searches,
    ));
    let scratch = ScratchDatabase::create(&args.test_url).await?;
    let by_id: BTreeMap<&str, (&'static str, &ClaimCase)> = plan
        .sets
        .in_run_order()
        .map(|(set, case)| (case.id.as_str(), (set, case)))
        .collect();
    let mut done = BTreeSet::new();
    let mut failure: Option<Failure> = None;
    let mut complete = true;
    for id in &plan.order {
        let Some((set, case)) = by_id.get(id.as_str()) else {
            continue;
        };
        let mut single = CaseSets::default();
        match *set {
            "real" => single.real.push((*case).clone()),
            "near_miss" => single.near_miss.push((*case).clone()),
            _ => single.fake.push((*case).clone()),
        }
        let report = match run_cases(&scratch.pool, Shared(Arc::clone(&capture)), &single).await {
            Ok(report) => report,
            Err(error) => {
                failure = Some(error);
                break;
            }
        };
        let outcome = report
            .real
            .iter()
            .chain(&report.near_miss)
            .chain(&report.fake)
            .map(|(_, outcome)| outcome)
            .next();
        if capture.exhausted() {
            eprintln!("budget exhausted at {id}; it is not recorded");
            complete = false;
            break;
        }
        if matches!(outcome, Some(Outcome::Unresolved(ClaimState::Pending))) {
            eprintln!("a request failed in transit at {id}; it is not recorded");
            complete = false;
            break;
        }
        eprintln!(
            "{id} [{}] -> {} (truth {})",
            case.kind,
            outcome.map_or_else(String::new, Outcome::describe),
            case.truth.as_deref().unwrap_or("-")
        );
        done.insert(id.clone());
    }
    scratch.drop_database().await?;
    let mut kept = seed;
    kept.extend(capture.captured());
    write_json(scratch_file, &kept)?;
    if let Some(error) = failure {
        return Err(error);
    }
    eprintln!(
        "{} live requests, {} live title searches",
        capture.live_requests(),
        capture.paid_requests()
    );
    Ok(Recorded {
        done,
        responses: capture.captured(),
        complete,
    })
}

/// The committed set's `(recorded, planned)` counts from its `status`, if it has one.
fn committed_recording(cases_path: &Path) -> Option<(usize, usize)> {
    let value: Value = serde_json::from_slice(&std::fs::read(cases_path).ok()?).ok()?;
    let status = value.get("status")?;
    let count = |key: &str| {
        status
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
    };
    Some((count("recorded")?, count("planned")?))
}

async fn run_part(args: &Args, part: Part) -> Result<(), Failure> {
    let label = if part == Part::E1 { "e1" } else { "e3" };
    let directory = args.out.join(label);
    let cases_path = directory.join("cases.json");
    let responses_path = directory.join("responses.json");
    let committed_status = committed_recording(&cases_path);
    let resumable = committed_status.is_some_and(|(recorded, planned)| recorded < planned);
    if (cases_path.exists() || responses_path.exists()) && !args.rerecord && !resumable {
        return Err(format!(
            "{} already holds fixtures; pass --rerecord to replace them",
            directory.display()
        )
        .into());
    }
    std::fs::create_dir_all(&args.scratch_dir)?;
    let planned_path = directory.join("cases.planned.json");
    let plan_value = if planned_path.exists() && !args.replan {
        serde_json::from_slice(&std::fs::read(&planned_path)?)?
    } else {
        let mut oa = OaLookup::open(args.scratch_dir.join("claims-holdout-openalex-works.json"))?;
        let value = if part == Part::E1 {
            plan_e1(args, &mut oa).await?
        } else {
            plan_e3(args, &mut oa).await?
        };
        write_json(&planned_path, &value)?;
        value
    };
    let plan = Plan::from_value(plan_value)?;

    let scratch_file = args
        .scratch_dir
        .join(format!("claims-holdout-{label}-responses.json"));
    let recorded = record_plan(args, &plan, &scratch_file).await?;
    eprintln!(
        "{label}: {} of {} planned cases recorded",
        recorded.done.len(),
        plan.order.len()
    );
    if recorded.done.is_empty() {
        return Err("nothing was recorded".into());
    }
    if let Some((committed, _)) = committed_status
        && recorded.done.len() < committed
        && !args.allow_shrink
    {
        return Err(format!(
            "this run recorded {} cases but {} already holds {committed}; refusing to shrink it \
             (restore the scratch responses file, or pass --allow-shrink)",
            recorded.done.len(),
            directory.display()
        )
        .into());
    }

    // Prove the recording is complete: replay the recorded cases, in set order, in a fresh
    // scratch database. Only a recording that replays completely is written.
    let committed: CaseSets = serde_json::from_value(plan.recorded(&recorded.done))?;
    let replay = ReplayTransport::new(recorded.responses.clone());
    let scratch = ScratchDatabase::create(&args.test_url).await?;
    let replayed = run_cases(&scratch.pool, replay.clone(), &committed).await;
    scratch.drop_database().await?;
    eprintln!("replay: {}", replayed?.render());
    if !replay.misses().is_empty() {
        return Err(format!("replay missed {:?}", replay.misses()).into());
    }
    let mut kept = plan.recorded(&recorded.done);
    if let Some(object) = kept.as_object_mut() {
        object.insert(
            "status".to_owned(),
            serde_json::json!({
                "recorded": recorded.done.len(),
                "planned": plan.order.len(),
                "complete": recorded.done.len() == plan.order.len(),
            }),
        );
    }
    write_json(&cases_path, &kept)?;
    write_json(&responses_path, &recorded.responses)?;
    if !recorded.complete {
        eprintln!(
            "{label} is partial: rerun with --rerecord once the OpenAlex search budget resets"
        );
    }
    Ok(())
}

async fn run_holdout(args: &Args) -> Result<(), Failure> {
    let parts = match args.part {
        Part::E1 => vec![Part::E1],
        Part::E3 => vec![Part::E3],
        Part::All => vec![Part::E3, Part::E1],
    };
    for part in parts {
        run_part(args, part).await?;
    }
    Ok(())
}
