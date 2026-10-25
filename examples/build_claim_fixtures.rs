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
//! It refuses to overwrite existing fixtures without `--rerecord`. `OpenAlex`'s free tier
//! charges for title searches, so the run stops at `--max-searches` live searches and keeps
//! what it recorded in a scratch file, which the next run resumes from.

#[allow(dead_code)]
#[path = "../tests/claims_support/mod.rs"]
mod claims_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use claims_support::{CaseSets, ClaimCase, Failure, ScratchDatabase, run_cases};
use lysilogy_core::{Claim, title_similarity};
use lysilogy_ingest::openalex::{
    JsonTransport, OpenAlexClient, ProviderFuture, ReqwestTransport, normalize_doi,
    short_openalex_id,
};
use lysilogy_ingest::verify_claim::fixtures::{CapturingTransport, ReplayTransport, Responses};
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

struct Args {
    rerecord: bool,
    out: PathBuf,
    dev_url: String,
    test_url: String,
    max_searches: usize,
    scratch_file: PathBuf,
}

fn user() -> String {
    std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned())
}

fn parse_args() -> Result<Args, Failure> {
    let home = std::env::var("HOME")?;
    let mut args = Args {
        rerecord: false,
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
        scratch_file: PathBuf::from(home).join(".cache/lysilogy-scratch/claims-responses.json"),
    };
    let mut iter = std::env::args().skip(1);
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--rerecord" => args.rerecord = true,
            "--out" => args.out = PathBuf::from(iter.next().ok_or("--out needs a directory")?),
            "--max-searches" => {
                args.max_searches = iter
                    .next()
                    .ok_or("--max-searches needs a number")?
                    .parse()?;
            }
            other => return Err(format!("unknown argument {other}").into()),
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
