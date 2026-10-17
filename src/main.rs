use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    net::SocketAddr,
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
};

use clap::{Parser, Subcommand, ValueEnum};
use lysilogy::acquisition::{AcquireOptions, AcquireReport};
use lysilogy::expansion::{ExpandOptions, ExpansionReport, WalkReport};
use lysilogy::literature::evidence::LOCAL_EVIDENCE_REQUIRES_WORK;
use lysilogy::prominence::ProminenceScope;
use lysilogy::provenance::{ReselectScope, cli_override_value, parse_field, render_provenance};
use lysilogy::ramps::{RampDetail, RampItemView, RampSummaryView};
use lysilogy::{
    AppState, Error, Result, build_router,
    domain::{
        AnalysisProvider, ExperimentArmScore, ExperimentJudgmentRequest, ExperimentRecord,
        ExperimentStatus, LearningRamp, PaperId, ProcessingStatus, StartExperimentRequest,
    },
};
use lysilogy::{
    analysis::LocalCliAnalyzer, field_extraction::SidecarFieldExtractor, labeling::SidecarLabeler,
};
use lysilogy_core::{ExpansionState, RampItemStatus, WALK_EXPANSIONS_MAX, WalkBounds, WorkId};
use lysilogy_ingest::acquire::AcquisitionOutcome;
use lysilogy_ingest::crossref::{PublisherFetch, PublisherOutcome};
use lysilogy_ingest::field_model::{FieldHarvestSummary, FieldOptions};
use lysilogy_ingest::labels::{LabelOptions, LabelSummary};
use lysilogy_ingest::literature::{LiteratureStatus, LiteratureSummary};
use lysilogy_ingest::local_evidence::LocalEvidenceSummary;
use lysilogy_ingest::prominence::RefreshSummary;
use tokio::{sync::Semaphore, task::JoinSet};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Directory containing the PDF library.
    #[arg(
        long,
        env = "LYSILOGY_LIBRARY",
        default_value = "local-articles",
        global = true
    )]
    library: PathBuf,

    /// Directory for portable text, Markdown, and JSON artifacts.
    #[arg(
        long,
        env = "LYSILOGY_DATA",
        default_value = ".lysilogy",
        global = true
    )]
    data: PathBuf,

    /// Directory for paper Markdown notes, separate from generated artifacts.
    #[arg(long, env = "LYSILOGY_NOTES", default_value = "Notes", global = true)]
    notes: PathBuf,

    /// JSON settings file; defaults to optional ./lysilogy.config.json.
    #[arg(long, env = "LYSILOGY_CONFIG", global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

/// The local model CLI that writes relationship labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum LabelSidecar {
    Claude,
    Codex,
    #[value(name = "openrouter")]
    OpenRouter,
}

impl LabelSidecar {
    const fn provider(self) -> AnalysisProvider {
        match self {
            Self::Claude => AnalysisProvider::Claude,
            Self::Codex => AnalysisProvider::Codex,
            Self::OpenRouter => AnalysisProvider::OpenRouter,
        }
    }
}

/// How `lit` should label edges, when asked to.
struct LabelRun {
    sidecar: LabelSidecar,
    limit: usize,
    scratch: PathBuf,
}

/// Field provenance subcommands. `<work>` is a `w...` ID, an `OpenAlex` `W...` ID,
/// `doi:<doi>`, or `arxiv:<id>`.
#[derive(Debug, Subcommand)]
enum ProvenanceCommand {
    /// Show every claim for a Work's title, year, authors, venue and DOI, and which one is selected.
    Show {
        work: String,
        /// Treat <work> as a paper (path, ID, or title fragment) and use the Work it is linked to.
        #[arg(long)]
        paper: bool,
    },
    /// Override a field: title, year, authors ("A. One; B. Two"), venue, or doi.
    Set {
        work: String,
        /// Treat <work> as a paper (path, ID, or title fragment) and use the Work it is linked to.
        #[arg(long)]
        paper: bool,
        field: String,
        value: String,
    },
    /// Remove your override of a field, restoring the tiered choice.
    Clear {
        work: String,
        /// Treat <work> as a paper (path, ID, or title fragment) and use the Work it is linked to.
        #[arg(long)]
        paper: bool,
        field: String,
    },
    /// Recompute the selected title, year and authors from claims.
    #[command(group(
        clap::ArgGroup::new("scope")
            .required(true)
            .multiple(false)
            .args(["work", "all"])
    ))]
    Reselect {
        work: Option<String>,
        /// Treat <work> as a paper (path, ID, or title fragment) and use the Work it is linked to.
        #[arg(long, requires = "work")]
        paper: bool,
        /// Reselect every Work that has a claim.
        #[arg(long)]
        all: bool,
    },
}

/// Reading-ramp subcommands. `<ramp>` is a `p...` ID or a unique title prefix; `<work>`
/// is a `w...` ID, an `OpenAlex` `W...` ID, `doi:<doi>`, or `arxiv:<id>`.
#[derive(Debug, Subcommand)]
enum RampCommand {
    /// Create a ramp seeded at a Work.
    New {
        work: String,
        /// Treat <work> as a paper (path, ID, or title fragment) and use the Work it is linked to.
        #[arg(long)]
        paper: bool,
        /// Ramp title; defaults to the seed's title.
        #[arg(long)]
        title: Option<String>,
    },
    /// List ramps, most recently updated first.
    List,
    /// Print a ramp's items in order.
    Show { ramp: String },
    /// Add a Work, optionally reached from an item that cites it.
    Add {
        ramp: String,
        work: String,
        /// The ramp item this Work was reached from.
        #[arg(long)]
        via: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Change an item's status or note.
    Set {
        ramp: String,
        work: String,
        #[arg(
            long,
            value_parser = lysilogy::ramps::parse_status,
            required_unless_present_any = ["note", "clear_note"]
        )]
        status: Option<RampItemStatus>,
        #[arg(long, conflicts_with = "clear_note")]
        note: Option<String>,
        #[arg(long)]
        clear_note: bool,
    },
    /// Remove an item (not the seed).
    Remove { ramp: String, work: String },
    /// Reorder a ramp: suggest "foundational first", or list every item once.
    Order {
        ramp: String,
        #[arg(long, conflicts_with = "works")]
        suggest: bool,
        #[arg(required_unless_present = "suggest")]
        works: Vec<String>,
    },
    /// Delete a ramp and its items.
    Delete { ramp: String },
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the API and built frontend.
    Serve {
        #[arg(long, default_value = "127.0.0.1:7319")]
        bind: SocketAddr,
        #[arg(long, default_value = "web/dist")]
        web: PathBuf,
    },
    /// Refresh discovery and report the library state.
    Scan,
    /// Report model spend by substage from the JSONL ledger.
    Spend,
    /// Report deterministic extraction counts and mangle signals; no model calls.
    Audit {
        /// A paper ID or an unambiguous title fragment.
        query: Option<String>,
        /// Audit every paper in the library instead of one.
        #[arg(long)]
        all: bool,
        /// Emit JSON instead of a text summary.
        #[arg(long)]
        json: bool,
    },
    /// Fill the paper-artifacts schema from a PDF with one model (extraction spike).
    Extract {
        /// A paper ID or an unambiguous title fragment; or pass --pdf.
        #[arg(required_unless_present = "pdf", conflicts_with = "pdf")]
        query: Option<String>,
        /// Any PDF path, inside the library or not.
        #[arg(long)]
        pdf: Option<PathBuf>,
        /// The `OpenRouter` model.
        #[arg(long, default_value = lysilogy::analysis::openrouter::DEFAULT_MODEL)]
        model: String,
        /// 1 for a single pass, 2 to add the self-check pass.
        #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=2))]
        passes: u8,
        /// Where the JSON record goes; defaults to <data>/extract/<stem>.json.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Where page text and images go; defaults to <data>/extract/<stem>/.
        #[arg(long)]
        work: Option<PathBuf>,
        /// Page image resolution.
        #[arg(long, default_value_t = lysilogy::paper_artifacts::resources::DEFAULT_DPI)]
        dpi: u32,
        /// No further model call starts once this paper has spent this much (USD).
        #[arg(long, default_value_t = 0.25)]
        paper_budget_usd: f64,
    },
    /// Analyze one paper selected by ID or an unambiguous title fragment.
    Analyze {
        query: String,
        #[arg(long, default_value = "openrouter")]
        provider: AnalysisProvider,
        #[arg(long)]
        force: bool,
    },
    /// Refresh only the authored abstract, preserving the map and context.
    RefreshAbstract {
        query: String,
        #[arg(long, default_value = "openrouter")]
        provider: AnalysisProvider,
        #[arg(long)]
        force: bool,
    },
    /// Refresh only cited research history and subsequent influence.
    RefreshContext {
        query: String,
        #[arg(long, default_value = "openrouter")]
        provider: AnalysisProvider,
        #[arg(long)]
        force: bool,
    },
    /// Regenerate only the section map, claims, and glossary as coherent reading units.
    RefreshStructure {
        query: String,
        #[arg(long, default_value = "openrouter")]
        provider: AnalysisProvider,
        #[arg(long)]
        force: bool,
    },
    /// Fetch public citation graph evidence for an exact paper identifier.
    CitationGraph {
        query: String,
        #[arg(long)]
        identifier: String,
        #[arg(long, value_enum, value_delimiter = ',')]
        provider: Vec<lysilogy::citation_graph::Provider>,
        #[arg(long, value_enum, value_delimiter = ',')]
        direction: Vec<lysilogy::citation_graph::Direction>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Convert one paper to Markdown and print it to standard output.
    Convert {
        /// Paper ID or an unambiguous title fragment.
        query: String,
    },
    /// Link one paper to its Work and ingest what it cites (needs `LYSILOGY_DATABASE_URL`).
    Lit {
        /// A PDF path inside the library, a paper ID, or an unambiguous title fragment.
        paper: String,
        /// Also persist and resolve the paper's local bibliography and in-text mentions.
        #[arg(long)]
        local: bool,
        /// Label local citation edges with a model sidecar (implies --local).
        #[arg(long)]
        labels: bool,
        /// Which local CLI writes the labels.
        #[arg(long, value_enum, default_value = "openrouter")]
        sidecar: LabelSidecar,
        /// Stop after this many model calls; cached answers do not count.
        #[arg(long, default_value_t = 25)]
        label_limit: usize,
        /// Also fetch the publisher (Crossref) record of every cited Work with a DOI.
        #[arg(long)]
        publisher: bool,
        /// Skip the cheap-model pass that reads missing authors and years
        /// (it runs by default with --local or --labels).
        #[arg(long)]
        no_fields: bool,
        /// Stop the field pass after this many model calls; cached answers do not count.
        #[arg(long, default_value_t = 10)]
        fields_limit: usize,
    },
    /// Recompute the relevance (prominence) scores for a paper's neighborhood or every Work (needs `LYSILOGY_DATABASE_URL`).
    #[command(group(
        clap::ArgGroup::new("scope")
            .required(true)
            .multiple(false)
            .args(["paper", "all"])
    ))]
    Prominence {
        /// A PDF path inside the library, a paper ID, or an unambiguous title fragment.
        paper: Option<String>,
        /// Recompute every Work that is in a citation or has a document.
        #[arg(long)]
        all: bool,
        /// Also fetch cited-by counts from `OpenAlex` (cached counts are reused).
        #[arg(long)]
        fetch_metrics: bool,
    },
    /// Fetch open-access copies of Works and map them into the library (needs `LYSILOGY_DATABASE_URL`).
    Fetch {
        /// A Work: a w... ID, an `OpenAlex` W... ID, `doi:<doi>`, or `arxiv:<id>`.
        #[arg(required_unless_present = "cited_by", conflicts_with = "cited_by")]
        work: Option<String>,
        /// Instead fetch the Works this paper cites that have no local copy: a PDF
        /// path inside the library, a paper ID, or an unambiguous title fragment.
        #[arg(long)]
        cited_by: Option<String>,
        /// With --cited-by, try at most this many Works (1 to 100).
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Look again at Works recently found unavailable, bypassing cached answers.
        #[arg(long)]
        retry_unavailable: bool,
        /// Let a local model pick the PDF link on publisher landing pages.
        #[arg(long, value_enum)]
        model_fallback: Option<LabelSidecar>,
    },
    /// Make a Work's references known and optionally walk backward from it (needs `LYSILOGY_DATABASE_URL`).
    Expand {
        /// A Work: a w... ID, an `OpenAlex` W... ID, `doi:<doi>`, or `arxiv:<id>`.
        /// With --paper: a PDF path inside the library, a paper ID, or a title fragment.
        work: String,
        /// Treat <work> as a paper and expand the Work it is linked to.
        #[arg(long)]
        paper: bool,
        /// How many levels to walk (1 is just this Work).
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=3))]
        depth: u8,
        /// How many children per Work to expand at each further level.
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u8).range(1..=10))]
        fanout: u8,
        /// Rerun the flow even when the references are already known.
        #[arg(long)]
        refresh: bool,
        /// For papers held locally, skip the local bibliography and in-text evidence.
        #[arg(long)]
        no_local: bool,
        /// Skip the cheap-model pass that reads missing authors and years.
        #[arg(long)]
        no_fields: bool,
    },
    /// Inspect and recompute where each Work's title, year and authors come from (needs `LYSILOGY_DATABASE_URL`).
    Provenance {
        #[command(subcommand)]
        command: ProvenanceCommand,
    },
    /// Build and manage reading ramps: ordered paths backward through a seed's references (needs `LYSILOGY_DATABASE_URL`).
    Ramp {
        #[command(subcommand)]
        command: RampCommand,
    },
    /// Analyze all papers that are not ready, continuing past individual faults.
    Ingest {
        #[arg(long, default_value = "openrouter")]
        provider: AnalysisProvider,
        #[arg(long)]
        force: bool,
        /// Stop after this many matching papers (useful while evaluating prompts).
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Run blind A/B learning-ramp experiments and persist both arms.
    Experiment {
        /// One or more paper IDs or unambiguous title fragments.
        #[arg(required = true, num_args = 1..)]
        queries: Vec<String>,
        /// Experiment ID from experiments/catalog.json.
        #[arg(long, default_value = "conceptual-bridge")]
        experiment: String,
        #[arg(long, default_value = "openrouter")]
        provider: AnalysisProvider,
        /// Independent A/B replications for each paper.
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=20))]
        repeat: u8,
        /// Maximum papers processed concurrently; arms within each run are also parallel.
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=8))]
        concurrency: u8,
    },
    /// Render an aggregate Markdown report from persisted runs and blind judgments.
    ExperimentReport {
        /// Write the report to this path instead of standard output.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Prepare blind run records without invoking a model (for isolated external workers).
    ExperimentPrepare {
        #[arg(required = true, num_args = 1..)]
        queries: Vec<String>,
        #[arg(long, default_value = "conceptual-bridge")]
        experiment: String,
        #[arg(long, default_value = "openrouter")]
        provider: AnalysisProvider,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=20))]
        repeat: u8,
    },
    /// Validate and import two independently generated JSON arms into a prepared run.
    ExperimentImport {
        /// Paper ID or unambiguous title fragment.
        query: String,
        #[arg(long)]
        run: String,
        #[arg(long)]
        arm_a: PathBuf,
        #[arg(long)]
        arm_b: PathBuf,
    },
    /// Persist a blind evaluator's absolute scorecards and comparative judgment.
    ExperimentJudge {
        /// Paper ID or unambiguous title fragment.
        query: String,
        #[arg(long)]
        run: String,
        #[arg(long)]
        input: PathBuf,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error);
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[allow(clippy::too_many_lines)] // Keep the CLI command dispatch in one match.
async fn run(cli: Cli) -> Result<()> {
    let config = lysilogy::config::AppConfig::load(
        cli.config
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("lysilogy.config.json")),
        cli.config.is_some(),
    )
    .await?;
    let state = AppState::new(&cli.library, &cli.data)
        .await?
        .with_graph_from_environment()
        .await
        .with_notes_root(&cli.notes)
        .with_vimrc(config.vim.vimrc)
        .with_notes_template(config.notes)?;
    let skip_fields = matches!(
        cli.command,
        Some(Command::Expand {
            no_fields: true,
            ..
        })
    );
    let state = if skip_fields {
        state
    } else {
        state.with_field_extractor(SidecarFieldExtractor::new(
            AnalysisProvider::OpenRouter,
            LocalCliAnalyzer::default(),
            cli.data.join("fields"),
        )?)
    };
    match cli.command.unwrap_or(Command::Serve {
        bind: "127.0.0.1:7319"
            .parse()
            .map_err(|error| Error::InvalidRequest(format!("invalid default address: {error}")))?,
        web: PathBuf::from("web/dist"),
    }) {
        Command::Serve { bind, web } => serve(state, bind, &web).await,
        Command::Scan => {
            let library = state.refresh().await?;
            let ready = library
                .papers
                .iter()
                .filter(|paper| matches!(paper.status, ProcessingStatus::Ready))
                .count();
            println!(
                "Discovered {} PDFs in {} ({} analyzed).",
                library.papers.len(),
                library.name,
                ready
            );
            Ok(())
        }
        Command::Audit { query, all, json } => audit_command(&state, query, all, json).await,
        Command::Extract {
            query,
            pdf,
            model,
            passes,
            out,
            work,
            dpi,
            paper_budget_usd,
        } => {
            let pdf = match (pdf, query) {
                (Some(pdf), _) => pdf,
                (None, Some(query)) => {
                    let id = resolve_paper(&state, &query).await?;
                    let paper = state.paper(&id).await?;
                    cli.library.join(&paper.paper.relative_path)
                }
                (None, None) => {
                    return Err(Error::InvalidRequest("provide a paper or --pdf".into()));
                }
            };
            let stem = pdf.file_stem().map_or_else(
                || "paper".to_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            );
            let extract_root = cli.data.join("extract");
            let options = lysilogy::paper_artifacts::run::ExtractOptions {
                output: out.unwrap_or_else(|| extract_root.join(format!("{stem}.json"))),
                work_directory: work.unwrap_or_else(|| extract_root.join(&stem)),
                pdf,
                model,
                passes,
                dpi,
                paper_budget_usd,
                timeout: std::time::Duration::from_secs(20 * 60),
            };
            let ledger = lysilogy::analysis::spend::SpendLedger::with_defaults();
            let record = lysilogy::paper_artifacts::run::extract(&options, &ledger).await?;
            print_extract_summary(&record, &options.output);
            if record.succeeded() {
                Ok(())
            } else {
                Err(Error::Task(format!(
                    "extraction incomplete; see {}",
                    options.output.display()
                )))
            }
        }
        Command::Spend => {
            let path = lysilogy::analysis::spend::default_ledger_path();
            let budget = lysilogy::analysis::spend::budget_usd();
            let records = lysilogy::analysis::spend::read_records(&path);
            let total: f64 = records.iter().map(|record| record.cost_usd).sum();
            println!("Model spend ({})", path.display());
            if records.is_empty() {
                println!("No calls recorded.");
            }
            for entry in lysilogy::analysis::spend::stage_totals(&records) {
                println!(
                    "{:<24} {:>4} calls  {:>7} in  {:>7} out  ${:.4}",
                    entry.stage,
                    entry.calls,
                    entry.prompt_tokens,
                    entry.completion_tokens,
                    entry.cost_usd
                );
            }
            println!("Total ${total:.4} of ${budget:.2}");
            Ok(())
        }
        Command::Analyze {
            query,
            provider,
            force,
        } => {
            let id = resolve_paper(&state, &query).await?;
            println!("Analyzing {id} with {provider}…");
            let view = state.analyze_now(&id, provider, force).await?;
            let analysis = view.analysis.ok_or_else(|| {
                Error::Task("analysis completed without a stored artifact".to_owned())
            })?;
            println!("Ready: {}\n{}", view.paper.metadata.title, analysis.thesis);
            Ok(())
        }
        Command::RefreshAbstract {
            query,
            provider,
            force,
        } => {
            let id = resolve_paper(&state, &query).await?;
            let view = state
                .refresh_component(
                    &id,
                    provider,
                    force,
                    lysilogy::domain::AnalysisComponent::Abstract,
                )
                .await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&view.analysis.and_then(|a| a.abstract_extraction))?
            );
            Ok(())
        }
        Command::RefreshContext {
            query,
            provider,
            force,
        } => {
            let id = resolve_paper(&state, &query).await?;
            let view = state
                .refresh_component(
                    &id,
                    provider,
                    force,
                    lysilogy::domain::AnalysisComponent::Context,
                )
                .await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&view.analysis.and_then(|a| a.context_assessment))?
            );
            Ok(())
        }
        Command::CitationGraph {
            query,
            identifier,
            provider,
            direction,
            limit,
        } => {
            use lysilogy::citation_graph::{Direction, GraphRequest, Provider};
            let id = resolve_paper(&state, &query).await?;
            let request = GraphRequest {
                identifier,
                providers: if provider.is_empty() {
                    Provider::ALL.to_vec()
                } else {
                    provider
                },
                directions: if direction.is_empty() {
                    vec![Direction::References, Direction::Citations]
                } else {
                    direction
                },
                limit,
            };
            let snapshot = state.fetch_citation_graph(&id, &request).await?;
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
            Ok(())
        }
        Command::Convert { query } => {
            let id = resolve_paper(&state, &query).await?;
            print!("{}", state.markdown(&id).await?);
            Ok(())
        }
        Command::Lit {
            paper,
            local,
            labels,
            sidecar,
            label_limit,
            publisher,
            no_fields,
            fields_limit,
        } => {
            let label_run = labels.then(|| LabelRun {
                sidecar,
                limit: label_limit,
                scratch: cli.data.join("labels"),
            });
            let field_run = (!no_fields).then(|| LabelRun {
                sidecar,
                limit: fields_limit,
                scratch: cli.data.join("fields"),
            });
            literature_command(
                &state,
                &cli.library,
                &paper,
                LiteratureRun {
                    local,
                    labels: label_run,
                    fields: field_run,
                    publisher,
                },
            )
            .await
        }
        Command::Prominence {
            paper,
            all,
            fetch_metrics,
        } => prominence_command(&state, &cli.library, paper.as_deref(), all, fetch_metrics).await,
        Command::Fetch {
            work,
            cited_by,
            limit,
            retry_unavailable,
            model_fallback,
        } => {
            let options = AcquireOptions {
                retry_unavailable,
                model_fallback: model_fallback.map(LabelSidecar::provider),
            };
            match (work, cited_by) {
                (Some(work), None) => fetch_work_command(&state, &work, options).await,
                (None, Some(paper)) => {
                    fetch_cited_by_command(&state, &cli.library, &paper, limit, options).await
                }
                _ => Err(Error::InvalidRequest(
                    "give exactly one of <work> or --cited-by".to_owned(),
                )),
            }
        }
        Command::Expand {
            work,
            paper,
            depth,
            fanout,
            refresh,
            no_local,
            no_fields: _,
        } => {
            let options = ExpandOptions {
                bounds: WalkBounds { depth, fanout },
                refresh,
                local: !no_local,
            };
            expand_command(&state, &cli.library, &work, paper, options).await
        }
        Command::Provenance { command } => provenance_command(&state, &cli.library, command).await,
        Command::Ramp { command } => ramp_command(&state, &cli.library, command).await,
        Command::RefreshStructure {
            query,
            provider,
            force,
        } => {
            let id = resolve_paper(&state, &query).await?;
            println!("Regrouping sections for {id} with {provider}…");
            let view = state
                .refresh_component(
                    &id,
                    provider,
                    force,
                    lysilogy::domain::AnalysisComponent::Structure,
                )
                .await?;
            if let Some(analysis) = view.analysis {
                println!(
                    "{}: {} reading units",
                    view.paper.metadata.title,
                    analysis.sections.len()
                );
                for section in analysis.sections {
                    println!(
                        "Pages {}–{}: {}",
                        section.pages.start, section.pages.end, section.title
                    );
                }
            }
            Ok(())
        }
        Command::Ingest {
            provider,
            force,
            limit,
        } => ingest(&state, provider, force, limit).await,
        Command::Experiment {
            queries,
            experiment,
            provider,
            repeat,
            concurrency,
        } => {
            experiment_campaign(&state, &queries, &experiment, provider, repeat, concurrency).await
        }
        Command::ExperimentReport { output } => experiment_report_command(&state, output).await,
        Command::ExperimentPrepare {
            queries,
            experiment,
            provider,
            repeat,
        } => prepare_experiment_campaign(&state, &queries, &experiment, provider, repeat).await,
        Command::ExperimentImport {
            query,
            run,
            arm_a,
            arm_b,
        } => import_experiment_command(&state, &query, &run, &arm_a, &arm_b).await,
        Command::ExperimentJudge { query, run, input } => {
            let id = resolve_paper(&state, &query).await?;
            let bytes = tokio::fs::read(&input)
                .await
                .map_err(|error| Error::io(&input, error))?;
            let judgment: ExperimentJudgmentRequest = serde_json::from_slice(&bytes)?;
            let view = state.judge_experiment(&id, &run, judgment).await?;
            println!("Judged and revealed {}: {}", view.run.id, view.judged);
            Ok(())
        }
    }
}

async fn experiment_report_command(state: &AppState, output: Option<PathBuf>) -> Result<()> {
    let report = render_experiment_report(&state.experiment_records().await?);
    if let Some(path) = output {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| Error::io(parent, error))?;
        }
        tokio::fs::write(&path, report)
            .await
            .map_err(|error| Error::io(&path, error))?;
        println!("Wrote experiment report to {}", path.display());
    } else {
        print!("{report}");
    }
    Ok(())
}

async fn import_experiment_command(
    state: &AppState,
    query: &str,
    run_id: &str,
    arm_a_path: &std::path::Path,
    arm_b_path: &std::path::Path,
) -> Result<()> {
    let id = resolve_paper(state, query).await?;
    let outputs = tokio::try_join!(
        read_learning_ramp(arm_a_path),
        read_learning_ramp(arm_b_path)
    )?;
    let view = state
        .import_experiment_outputs(&id, run_id, outputs.into())
        .await?;
    println!(
        "Imported and verified {}: {:?}",
        view.run.id, view.run.status
    );
    Ok(())
}

async fn prepare_experiment_campaign(
    state: &AppState,
    queries: &[String],
    experiment: &str,
    provider: AnalysisProvider,
    repeat: u8,
) -> Result<()> {
    for query in queries {
        let id = resolve_paper(state, query).await?;
        for replication in 1..=repeat {
            let run = state
                .prepare_experiment_only(
                    &id,
                    &StartExperimentRequest {
                        experiment_id: experiment.to_owned(),
                        provider,
                    },
                )
                .await?;
            let assignments = run
                .arms
                .iter()
                .map(|arm| format!("{}={}", arm.blind_label, arm.variant_id))
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "{} [{replication}/{repeat}] {}: {assignments}",
                run.paper_title, run.id
            );
        }
    }
    Ok(())
}

async fn read_learning_ramp(path: &std::path::Path) -> Result<LearningRamp> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| Error::io(path, error))?;
    serde_json::from_slice(&bytes).map_err(Error::from)
}

#[derive(Default)]
struct VariantReportStats {
    label: String,
    scored: u32,
    score_sums: [u32; 6],
    target_sum: u32,
    target_count: u32,
    hard_rejects: u32,
    failure_tags: BTreeMap<String, u32>,
    overall_wins: u32,
    traction_wins: u32,
    rigor_wins: u32,
}

#[derive(Default)]
struct ExperimentComparison {
    name: String,
    judged: u32,
    overall_ties: u32,
    traction_ties: u32,
    rigor_ties: u32,
}

fn render_experiment_report(records: &[ExperimentRecord]) -> String {
    let mut report = String::new();
    let papers = records
        .iter()
        .map(|record| record.run.paper_id.clone())
        .collect::<BTreeSet<_>>();
    let completed = records
        .iter()
        .filter(|record| record.run.status == ExperimentStatus::Completed)
        .count();
    let failed = records
        .iter()
        .filter(|record| record.run.status == ExperimentStatus::Failed)
        .count();
    let running = records
        .iter()
        .filter(|record| record.run.status == ExperimentStatus::Running)
        .count();
    let judged = records
        .iter()
        .filter(|record| record.judgment.is_some())
        .count();
    let _ = writeln!(
        report,
        "# Learning-ramp experiment report\n\nGenerated {}.\n",
        chrono::Utc::now().to_rfc3339()
    );
    let _ = writeln!(
        report,
        "## Coverage\n\n- {} persisted runs across {} papers\n- {completed} completed, {failed} failed, {running} running\n- {judged} blind judgments\n",
        records.len(),
        papers.len()
    );

    let mut variants: BTreeMap<(String, String), VariantReportStats> = BTreeMap::new();
    let mut comparisons: BTreeMap<String, ExperimentComparison> = BTreeMap::new();
    let mut global_failure_tags: BTreeMap<String, u32> = BTreeMap::new();
    for record in records {
        initialize_variants(&mut variants, record);
        let Some(judgment) = &record.judgment else {
            continue;
        };
        let comparison = comparisons
            .entry(record.run.experiment_id.clone())
            .or_default();
        comparison.name.clone_from(&record.run.experiment_name);
        comparison.judged += 1;
        record_preference(&mut variants, &record.run, &judgment.overall, |stats| {
            stats.overall_wins += 1;
        });
        record_preference(
            &mut variants,
            &record.run,
            &judgment.early_traction,
            |stats| {
                stats.traction_wins += 1;
            },
        );
        record_preference(&mut variants, &record.run, &judgment.rigor, |stats| {
            stats.rigor_wins += 1;
        });
        comparison.overall_ties += u32::from(judgment.overall == "tie");
        comparison.traction_ties += u32::from(judgment.early_traction == "tie");
        comparison.rigor_ties += u32::from(judgment.rigor == "tie");
        for score in &judgment.arm_scores {
            let Some(arm) = record
                .run
                .arms
                .iter()
                .find(|arm| arm.blind_label == score.blind_label)
            else {
                continue;
            };
            let stats = variants
                .entry((record.run.experiment_id.clone(), arm.variant_id.clone()))
                .or_default();
            record_arm_score(stats, score, &mut global_failure_tags);
        }
    }

    if judged == 0 {
        let _ = writeln!(
            report,
            "## Evidence status\n\nNo completed blind judgment is available. Do not infer a prompt winner from failed or unjudged runs.\n"
        );
    } else {
        render_quality_table(&mut report, &variants);
        render_preference_table(&mut report, &variants, &comparisons);
        render_failure_table(&mut report, &global_failure_tags);
    }
    render_run_log(&mut report, records);
    report
}

fn initialize_variants(
    variants: &mut BTreeMap<(String, String), VariantReportStats>,
    record: &ExperimentRecord,
) {
    for arm in &record.run.arms {
        variants
            .entry((record.run.experiment_id.clone(), arm.variant_id.clone()))
            .or_default()
            .label
            .clone_from(&arm.variant_label);
    }
}

fn record_preference(
    variants: &mut BTreeMap<(String, String), VariantReportStats>,
    run: &lysilogy::domain::ExperimentRun,
    choice: &str,
    update: impl FnOnce(&mut VariantReportStats),
) {
    let Some(arm) = run.arms.iter().find(|arm| arm.blind_label == choice) else {
        return;
    };
    let stats = variants
        .entry((run.experiment_id.clone(), arm.variant_id.clone()))
        .or_default();
    update(stats);
}

fn record_arm_score(
    stats: &mut VariantReportStats,
    score: &ExperimentArmScore,
    global_failure_tags: &mut BTreeMap<String, u32>,
) {
    let values = [
        score.paper_specificity_actionability,
        score.early_traction,
        score.fidelity_rigor,
        score.dependency_flow,
        score.economy,
        score.provenance_uncertainty,
    ];
    stats.scored += 1;
    for (sum, value) in stats.score_sums.iter_mut().zip(values) {
        *sum += u32::from(value);
    }
    if let Some(value) = score.target_dimension {
        stats.target_sum += u32::from(value);
        stats.target_count += 1;
    }
    stats.hard_rejects += u32::from(score.hard_reject);
    for tag in &score.failure_tags {
        *stats.failure_tags.entry(tag.clone()).or_default() += 1;
        *global_failure_tags.entry(tag.clone()).or_default() += 1;
    }
}

fn mean(sum: u32, count: u32) -> String {
    if count == 0 {
        "—".to_owned()
    } else {
        format!("{:.1}", f64::from(sum) / f64::from(count))
    }
}

fn render_quality_table(
    report: &mut String,
    variants: &BTreeMap<(String, String), VariantReportStats>,
) {
    let _ = writeln!(report, "## Absolute quality by variant\n");
    let _ = writeln!(
        report,
        "| Experiment | Variant | N | Specific / actionable | Traction | Fidelity | Dependency | Economy | Provenance | Target | Hard rejects |"
    );
    let _ = writeln!(
        report,
        "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
    );
    for ((experiment, variant), stats) in variants {
        if stats.scored == 0 {
            continue;
        }
        let _ = writeln!(
            report,
            "| {} | {} (`{}`) | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            markdown_cell(experiment),
            markdown_cell(&stats.label),
            markdown_cell(variant),
            stats.scored,
            mean(stats.score_sums[0], stats.scored),
            mean(stats.score_sums[1], stats.scored),
            mean(stats.score_sums[2], stats.scored),
            mean(stats.score_sums[3], stats.scored),
            mean(stats.score_sums[4], stats.scored),
            mean(stats.score_sums[5], stats.scored),
            mean(stats.target_sum, stats.target_count),
            stats.hard_rejects,
        );
    }
    let _ = writeln!(
        report,
        "\nScores are 0–4. Treat fewer than three scored arms per variant as exploratory, and never promote a variant with an unresolved hard reject.\n"
    );
}

fn render_preference_table(
    report: &mut String,
    variants: &BTreeMap<(String, String), VariantReportStats>,
    comparisons: &BTreeMap<String, ExperimentComparison>,
) {
    let _ = writeln!(report, "## Blind comparative choices\n");
    let _ = writeln!(
        report,
        "| Experiment | Variant | Overall wins | Traction wins | Rigor wins | Judged runs | Overall ties |"
    );
    let _ = writeln!(report, "| --- | --- | ---: | ---: | ---: | ---: | ---: |");
    for ((experiment, variant), stats) in variants {
        let Some(comparison) = comparisons.get(experiment) else {
            continue;
        };
        let _ = writeln!(
            report,
            "| {} | {} (`{}`) | {} | {} | {} | {} | {} |",
            markdown_cell(&comparison.name),
            markdown_cell(&stats.label),
            markdown_cell(variant),
            stats.overall_wins,
            stats.traction_wins,
            stats.rigor_wins,
            comparison.judged,
            comparison.overall_ties,
        );
    }
    let _ = writeln!(report);
}

fn render_failure_table(report: &mut String, tags: &BTreeMap<String, u32>) {
    let _ = writeln!(report, "## Anti-slop failure patterns\n");
    if tags.is_empty() {
        let _ = writeln!(report, "No failure tags were recorded.\n");
        return;
    }
    let _ = writeln!(report, "| Failure tag | Count |\n| --- | ---: |");
    let mut ordered = tags.iter().collect::<Vec<_>>();
    ordered.sort_by(|(left_tag, left_count), (right_tag, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_tag.cmp(right_tag))
    });
    for (tag, count) in ordered {
        let _ = writeln!(report, "| {} | {count} |", markdown_cell(tag));
    }
    let _ = writeln!(report);
}

fn render_run_log(report: &mut String, records: &[ExperimentRecord]) {
    let _ = writeln!(report, "## Run log\n");
    let _ = writeln!(
        report,
        "| Created | Paper | Experiment | Provider | Status | Judged | Run |"
    );
    let _ = writeln!(report, "| --- | --- | --- | --- | --- | --- | --- |");
    for record in records {
        let _ = writeln!(
            report,
            "| {} | {} | {} | {} | {:?} | {} | `{}` |",
            record.run.created_at.to_rfc3339(),
            markdown_cell(&record.run.paper_title),
            markdown_cell(&record.run.experiment_name),
            record.run.provider,
            record.run.status,
            if record.judgment.is_some() {
                "yes"
            } else {
                "no"
            },
            markdown_cell(&record.run.id),
        );
    }
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

async fn experiment_campaign(
    state: &AppState,
    queries: &[String],
    experiment: &str,
    provider: AnalysisProvider,
    repeat: u8,
    concurrency: u8,
) -> Result<()> {
    let mut papers = Vec::with_capacity(queries.len());
    for query in queries {
        let id = resolve_paper(state, query).await?;
        if papers
            .iter()
            .all(|(existing, _): &(PaperId, String)| existing != &id)
        {
            let title = state.paper(&id).await?.paper.metadata.title;
            papers.push((id, title));
        }
    }

    let total = papers.len() * usize::from(repeat);
    println!(
        "Running {total} blind A/B `{experiment}` run(s) across {} paper(s) with {provider}; up to {concurrency} papers in parallel…",
        papers.len()
    );
    let gate = Arc::new(Semaphore::new(usize::from(concurrency)));
    let mut tasks = JoinSet::new();
    for (id, title) in papers {
        let state = state.clone();
        let experiment = experiment.to_owned();
        let gate = Arc::clone(&gate);
        tasks.spawn(async move {
            let _permit = gate.acquire_owned().await.map_err(|error| {
                Error::Task(format!("experiment concurrency gate closed: {error}"))
            })?;
            let mut outcomes = Vec::with_capacity(usize::from(repeat));
            for replication in 1..=repeat {
                let result = state
                    .run_experiment_now(
                        &id,
                        StartExperimentRequest {
                            experiment_id: experiment.clone(),
                            provider,
                        },
                    )
                    .await;
                outcomes.push((replication, result));
            }
            Ok::<_, Error>((title, outcomes))
        });
    }

    let mut failures = Vec::new();
    while let Some(task) = tasks.join_next().await {
        let (title, outcomes) =
            task.map_err(|error| Error::Task(format!("experiment worker failed: {error}")))??;
        for (replication, outcome) in outcomes {
            match outcome {
                Ok(view) => println!(
                    "[{replication}/{repeat}] {title}: {} {:?}",
                    view.run.id, view.run.status
                ),
                Err(error) => {
                    eprintln!("[{replication}/{repeat}] {title}: failed: {error}");
                    failures.push(format!("{title} replication {replication}: {error}"));
                }
            }
        }
    }

    if failures.is_empty() {
        println!("Completed {total} run(s); open :experiment in the reader to judge blind arms.");
        Ok(())
    } else {
        Err(Error::Task(format!(
            "{} of {total} experiment runs failed; completed artifacts were preserved",
            failures.len()
        )))
    }
}

async fn serve(state: AppState, bind: SocketAddr, web: &std::path::Path) -> Result<()> {
    let frontend = web.is_dir().then_some(web);
    if frontend.is_none() {
        tracing::warn!(path = %web.display(), "frontend build not found; serving API only");
    }
    let router = build_router(state, frontend);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|error| Error::io(bind.to_string(), error))?;
    tracing::info!(address = %bind, "Lysilogy is listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| Error::Task(format!("server failed: {error}")))
}

async fn ingest(
    state: &AppState,
    provider: AnalysisProvider,
    force: bool,
    limit: Option<usize>,
) -> Result<()> {
    let papers = state.library().await.papers;
    let candidates = papers
        .into_iter()
        .filter(|paper| force || !matches!(paper.status, ProcessingStatus::Ready))
        .take(limit.unwrap_or(usize::MAX))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        println!("All discovered papers are already analyzed.");
        return Ok(());
    }

    let total = candidates.len();
    let mut failures = Vec::new();
    for (index, paper) in candidates.into_iter().enumerate() {
        println!(
            "[{}/{}] {} ({provider})",
            index + 1,
            total,
            paper.metadata.title
        );
        if let Err(error) = state.analyze_now(&paper.id, provider, force).await {
            eprintln!("  failed: {error}");
            failures.push((paper.metadata.title, error.to_string()));
        }
    }
    if failures.is_empty() {
        println!("Ingested {total} papers.");
        Ok(())
    } else {
        Err(Error::Task(format!(
            "{} of {total} papers failed; successful artifacts were preserved",
            failures.len()
        )))
    }
}

/// What `lit` does beyond the provider flow.
struct LiteratureRun {
    local: bool,
    labels: Option<LabelRun>,
    /// The field pass; its scratch directory and call limit.
    fields: Option<LabelRun>,
    publisher: bool,
}

async fn literature_command(
    state: &AppState,
    library: &std::path::Path,
    paper: &str,
    run: LiteratureRun,
) -> Result<()> {
    let id = resolve_lit_target(state, library, paper).await?;
    let summary = state.run_literature_flow(&id, run.publisher).await?;
    print!("{}", render_literature_summary(&summary));
    if run.local || run.labels.is_some() {
        match state.run_local_evidence(&id, &summary).await? {
            Some(mut evidence) => {
                if let Some(label_run) = &run.labels {
                    evidence.labels = label_local_edges(state, &id, &summary, label_run).await?;
                }
                if let Some(field_run) = &run.fields {
                    harvest_fields(state, &id, &summary, field_run, &mut evidence).await?;
                }
                print!("{}", render_local_evidence(&evidence));
            }
            None => println!("Warnings   {LOCAL_EVIDENCE_REQUIRES_WORK}"),
        }
    }
    if let Some(work) = summary.work_id {
        state.refresh_prominence_quietly(work, true).await;
    }
    if let LiteratureStatus::Unmatched { reason } = &summary.status {
        return Err(Error::Literature(format!(
            "no Work found for {id}: {reason}"
        )));
    }
    Ok(())
}

async fn prominence_command(
    state: &AppState,
    library: &std::path::Path,
    paper: Option<&str>,
    all: bool,
    fetch_metrics: bool,
) -> Result<()> {
    let scope = match (paper, all) {
        (Some(text), false) => {
            ProminenceScope::Paper(resolve_lit_target(state, library, text).await?)
        }
        (None, true) => ProminenceScope::All,
        _ => {
            return Err(Error::InvalidRequest(
                "give exactly one of a paper or --all".to_owned(),
            ));
        }
    };
    let started = std::time::Instant::now();
    let summary = state.recompute_prominence(scope, fetch_metrics).await?;
    print!(
        "{}",
        render_prominence_summary(&summary, fetch_metrics, started.elapsed().as_secs_f64())
    );
    Ok(())
}

fn render_prominence_summary(
    summary: &RefreshSummary,
    fetch_metrics: bool,
    seconds: f64,
) -> String {
    let mut text = format!(
        "Recomputed {} works ({} with global counts) in {seconds:.1} s\n",
        summary.works, summary.with_global_count
    );
    if fetch_metrics {
        let _ = writeln!(
            text,
            "Fetched cited-by counts for {} works ({} requests)",
            summary.metrics_fetched, summary.metric_requests
        );
    }
    for warning in &summary.warnings {
        let _ = writeln!(text, "Warnings   {warning}");
    }
    text
}

/// A paper from a PDF path inside the library, a paper ID, or a title fragment.
async fn resolve_lit_target(
    state: &AppState,
    library: &std::path::Path,
    text: &str,
) -> Result<PaperId> {
    let candidate = PathBuf::from(text);
    if tokio::fs::metadata(&candidate)
        .await
        .is_ok_and(|metadata| metadata.is_file())
    {
        let path = tokio::fs::canonicalize(&candidate)
            .await
            .map_err(|error| Error::io(&candidate, error))?;
        let root = tokio::fs::canonicalize(library)
            .await
            .map_err(|error| Error::io(library, error))?;
        let id = PaperId::from_relative_path(&lit_target_relative_path(&root, &path)?);
        state.paper(&id).await?;
        return Ok(id);
    }
    resolve_paper(state, text).await
}

async fn fetch_work_command(state: &AppState, text: &str, options: AcquireOptions) -> Result<()> {
    let work = state.resolve_work_argument(text).await?;
    let report = state.acquire_work(work, options).await?;
    print!("{}", render_acquisition_summary(&report));
    Ok(())
}

async fn expand_command(
    state: &AppState,
    library: &std::path::Path,
    text: &str,
    paper: bool,
    options: ExpandOptions,
) -> Result<()> {
    let seed = if paper {
        let id = resolve_lit_target(state, library, text).await?;
        state.linked_work(&id).await?
    } else {
        state.resolve_work_argument(text).await?
    };
    let started = std::time::Instant::now();
    let report = state.walk_from(seed, options).await?;
    let works: Vec<WorkId> = report
        .expansions
        .iter()
        .map(|expansion| expansion.work_id)
        .collect();
    let titles = state.work_titles(&works).await?;
    print!(
        "{}",
        render_walk_report(&report, &titles, started.elapsed().as_secs_f64())
    );
    Ok(())
}

/// A Work for a ramp command: a Work reference, or with `paper` the Work a paper is linked to.
async fn resolve_ramp_work(
    state: &AppState,
    library: &std::path::Path,
    text: &str,
    paper: bool,
) -> Result<WorkId> {
    if paper {
        let id = resolve_lit_target(state, library, text).await?;
        return state.linked_work(&id).await;
    }
    state.resolve_work_argument(text).await
}

async fn provenance_command(
    state: &AppState,
    library: &std::path::Path,
    command: ProvenanceCommand,
) -> Result<()> {
    match command {
        ProvenanceCommand::Show { work, paper } => {
            let work = resolve_ramp_work(state, library, &work, paper).await?;
            print!("{}", render_provenance(&state.work_provenance(work).await?));
            Ok(())
        }
        ProvenanceCommand::Set {
            work,
            paper,
            field,
            value,
        } => {
            let field = parse_field(&field)?;
            let value = cli_override_value(field, &value)?;
            let work = resolve_ramp_work(state, library, &work, paper).await?;
            let view = state.set_field_override(work, field, &value).await?;
            print!("{}", render_provenance(&view));
            Ok(())
        }
        ProvenanceCommand::Clear { work, paper, field } => {
            let field = parse_field(&field)?;
            let work = resolve_ramp_work(state, library, &work, paper).await?;
            let view = state.clear_field_override(work, field).await?;
            print!("{}", render_provenance(&view));
            Ok(())
        }
        ProvenanceCommand::Reselect { work, paper, all } => {
            let scope = match (work, all) {
                (Some(text), false) => {
                    ReselectScope::Work(resolve_ramp_work(state, library, &text, paper).await?)
                }
                (None, true) => ReselectScope::All,
                _ => {
                    return Err(Error::InvalidRequest(
                        "give exactly one of <work> or --all".to_owned(),
                    ));
                }
            };
            let reselected = state.reselect_claims(scope).await?;
            println!("Reselected {reselected} works");
            Ok(())
        }
    }
}

async fn ramp_command(
    state: &AppState,
    library: &std::path::Path,
    command: RampCommand,
) -> Result<()> {
    match command {
        RampCommand::New { work, paper, title } => {
            let seed = resolve_ramp_work(state, library, &work, paper).await?;
            let title = title
                .as_deref()
                .map(lysilogy::ramps::parse_title)
                .transpose()?;
            let detail = state.create_ramp(seed, title).await?;
            print!("{}", render_ramp_detail(&detail));
        }
        RampCommand::List => {
            let ramps = state.list_ramps(None).await?;
            print!("{}", render_ramp_list(&ramps));
        }
        RampCommand::Show { ramp } => {
            let ramp = state.ramp_reference(&ramp).await?;
            print!("{}", render_ramp_detail(&state.ramp_detail(ramp).await?));
        }
        RampCommand::Add {
            ramp,
            work,
            via,
            note,
        } => {
            let ramp = state.ramp_reference(&ramp).await?;
            let work = state.resolve_work_argument(&work).await?;
            let via = match via {
                Some(via) => Some(state.resolve_work_argument(&via).await?),
                None => None,
            };
            let note = lysilogy::ramps::parse_note(note.as_deref())?;
            let item = state.add_ramp_item(ramp, work, via, note).await?;
            println!("Added: {}", render_ramp_item_line(&item, None));
        }
        RampCommand::Set {
            ramp,
            work,
            status,
            note,
            clear_note,
        } => {
            let ramp = state.ramp_reference(&ramp).await?;
            let work = state.resolve_work_argument(&work).await?;
            let note = if clear_note {
                Some(None)
            } else {
                note.map(|note| lysilogy::ramps::parse_note(Some(&note)))
                    .transpose()?
            };
            let item = state.update_ramp_item(ramp, work, status, note).await?;
            println!("Updated: {}", render_ramp_item_line(&item, None));
        }
        RampCommand::Remove { ramp, work } => {
            let ramp = state.ramp_reference(&ramp).await?;
            let work = state.resolve_work_argument(&work).await?;
            state.remove_ramp_item(ramp, work).await?;
            println!("Removed.");
        }
        RampCommand::Order {
            ramp,
            suggest,
            works,
        } => {
            let ramp = state.ramp_reference(&ramp).await?;
            let detail = if suggest {
                state.suggest_ramp_order(ramp).await?
            } else {
                let mut order = Vec::with_capacity(works.len());
                for work in &works {
                    order.push(state.resolve_work_argument(work).await?);
                }
                state.reorder_ramp(ramp, &order).await?
            };
            print!("{}", render_ramp_detail(&detail));
        }
        RampCommand::Delete { ramp } => {
            let ramp = state.ramp_reference(&ramp).await?;
            state.delete_ramp(ramp).await?;
            println!("Deleted.");
        }
    }
    Ok(())
}

fn render_ramp_list(ramps: &[RampSummaryView]) -> String {
    if ramps.is_empty() {
        return "No ramps.\n".to_owned();
    }
    let mut text = String::new();
    for ramp in ramps {
        let _ = writeln!(
            text,
            "{}  {} ({} items: {} unread, {} reading, {} read)",
            ramp.id,
            ramp.title,
            ramp.item_count,
            ramp.status_counts.unread,
            ramp.status_counts.reading,
            ramp.status_counts.read
        );
    }
    text
}

/// One item: `<ordinal>. [<status>] <title> (<year>) · depth <d> · via <title> · <local>`.
/// `local` is `held`, `fetch:<state>`, or a dash; it is left off when `None`.
fn render_ramp_item_line(item: &RampItemView, local: Option<&str>) -> String {
    let title = item
        .work
        .display_title
        .clone()
        .unwrap_or_else(|| item.work.id.to_string());
    let year = item
        .work
        .display_year
        .map_or_else(|| "n.d.".to_owned(), |year| year.to_string());
    let mut line = format!(
        "{}. [{}] {title} ({year}) · depth {}",
        item.ordinal, item.status, item.depth
    );
    if let Some(via) = item.via_work_id {
        let via_title = item
            .path
            .iter()
            .find(|step| step.work_id == via)
            .and_then(|step| step.title.clone())
            .unwrap_or_else(|| via.to_string());
        let _ = write!(line, " · via {via_title}");
    }
    if let Some(local) = local {
        let _ = write!(line, " · {local}");
    }
    line
}

fn render_ramp_detail(detail: &RampDetail) -> String {
    let mut text = format!(
        "{} ({}) · {} items\n",
        detail.ramp.title, detail.ramp.id, detail.ramp.item_count
    );
    for item in &detail.items {
        let key = item.work.id.to_string();
        let local = if detail.local_papers.contains_key(&key) {
            "held".to_owned()
        } else {
            detail
                .acquisitions
                .get(&key)
                .map_or_else(|| "—".to_owned(), |brief| format!("fetch:{}", brief.state))
        };
        let _ = writeln!(text, "{}", render_ramp_item_line(item, Some(&local)));
        if let Some(note) = &item.note {
            let _ = writeln!(text, "    note: {note}");
        }
    }
    text
}

fn render_expansion_line(report: &ExpansionReport, titles: &BTreeMap<WorkId, String>) -> String {
    let title = titles
        .get(&report.work_id)
        .map_or_else(|| report.work_id.to_string(), Clone::clone);
    let method = report
        .method
        .map_or_else(String::new, |method| method.to_string());
    let outcome = match (report.state, report.cached) {
        (ExpansionState::Expanded, true) => {
            format!("cached ({method}, {} references)", report.references)
        }
        (ExpansionState::Expanded, false) => format!(
            "expanded ({method}, {} references, {} new)",
            report.references, report.new_works
        ),
        (ExpansionState::Unavailable, cached) => {
            let reason = report
                .unavailable_reason
                .map_or_else(|| "unknown".to_owned(), |reason| reason.to_string());
            if cached {
                format!("unavailable ({reason}, cached)")
            } else {
                format!("unavailable ({reason})")
            }
        }
    };
    let mut line = format!("{title}: {outcome}");
    if let Some(paper) = &report.paper_id {
        let _ = write!(line, " [paper {paper}]");
    }
    for warning in &report.warnings {
        let _ = write!(line, "\n  warning: {warning}");
    }
    line
}

fn render_walk_report(
    report: &WalkReport,
    titles: &BTreeMap<WorkId, String>,
    seconds: f64,
) -> String {
    let mut text = String::new();
    for expansion in &report.expansions {
        let _ = writeln!(text, "{}", render_expansion_line(expansion, titles));
    }
    let truncated = if report.truncated {
        format!(", truncated at {WALK_EXPANSIONS_MAX} expansions")
    } else {
        String::new()
    };
    let _ = writeln!(
        text,
        "Walked {} levels, {} expansions{truncated} in {seconds:.1} s",
        report.levels,
        report.expansions.len()
    );
    text
}

async fn fetch_cited_by_command(
    state: &AppState,
    library: &std::path::Path,
    text: &str,
    limit: usize,
    options: AcquireOptions,
) -> Result<()> {
    let paper = resolve_lit_target(state, library, text).await?;
    let targets = state
        .acquisition_targets_cited_by(&paper, options.retry_unavailable)
        .await?;
    let mut totals = CitedByTotals {
        skipped: targets.iter().filter(|target| target.skipped).count(),
        ..CitedByTotals::default()
    };
    let limit = limit.clamp(1, MAX_CITED_BY_LIMIT);
    for target in targets.iter().filter(|target| !target.skipped).take(limit) {
        let report = state.acquire_work(target.work_id, options).await?;
        println!("{}", render_cited_by_line(&report));
        totals.record(&report.summary.outcome);
    }
    println!("{}", render_cited_by_totals(&totals));
    Ok(())
}

async fn harvest_fields(
    state: &AppState,
    id: &PaperId,
    summary: &LiteratureSummary,
    run: &LabelRun,
    evidence: &mut LocalEvidenceSummary,
) -> Result<()> {
    let extractor = SidecarFieldExtractor::new(
        run.sidecar.provider(),
        LocalCliAnalyzer::default(),
        run.scratch.clone(),
    )?;
    let options = FieldOptions {
        limit: run.limit,
        ..FieldOptions::default()
    };
    state
        .harvest_fields_into(id, summary, &extractor, options, evidence)
        .await
}

async fn label_local_edges(
    state: &AppState,
    id: &PaperId,
    summary: &LiteratureSummary,
    run: &LabelRun,
) -> Result<Option<LabelSummary>> {
    let labeler = SidecarLabeler::new(
        run.sidecar.provider(),
        LocalCliAnalyzer::default(),
        run.scratch.clone(),
    )?;
    let options = LabelOptions {
        limit: run.limit,
        ..LabelOptions::default()
    };
    state
        .label_local_edges(id, summary, &labeler, options)
        .await
}

/// The library-relative path of `path`, which must lie under `root`. Both
/// arguments are expected to be canonical, so `..` and symlinks cannot escape.
fn lit_target_relative_path(root: &std::path::Path, path: &std::path::Path) -> Result<PathBuf> {
    path.strip_prefix(root).map(PathBuf::from).map_err(|_| {
        Error::InvalidRequest(format!(
            "{} is outside the library {}",
            path.display(),
            root.display()
        ))
    })
}

/// Most Works one `--cited-by` run will try.
const MAX_CITED_BY_LIMIT: usize = 100;

/// Outcome tallies for a `--cited-by` run.
#[derive(Debug, Default, PartialEq, Eq)]
struct CitedByTotals {
    fetched: usize,
    already_local: usize,
    unavailable: BTreeMap<String, usize>,
    pending: usize,
    skipped: usize,
}

impl CitedByTotals {
    fn record(&mut self, outcome: &AcquisitionOutcome) {
        match outcome {
            AcquisitionOutcome::Mapped => self.fetched += 1,
            AcquisitionOutcome::AlreadyMapped | AcquisitionOutcome::AlreadyLocal => {
                self.already_local += 1;
            }
            AcquisitionOutcome::Unavailable { reason } => {
                *self
                    .unavailable
                    .entry(reason.as_str().to_owned())
                    .or_default() += 1;
            }
            AcquisitionOutcome::Pending { .. } | AcquisitionOutcome::Busy => self.pending += 1,
            AcquisitionOutcome::SkippedUnavailable { .. } => self.skipped += 1,
        }
    }
}

fn render_cited_by_totals(totals: &CitedByTotals) -> String {
    let unavailable: usize = totals.unavailable.values().sum();
    let mut line = format!(
        "Fetched {} \u{b7} already local {} \u{b7} unavailable {unavailable}",
        totals.fetched, totals.already_local
    );
    if !totals.unavailable.is_empty() {
        let reasons: Vec<String> = totals
            .unavailable
            .iter()
            .map(|(reason, count)| format!("{reason} {count}"))
            .collect();
        let _ = write!(line, " ({})", reasons.join(", "));
    }
    let _ = write!(
        line,
        " \u{b7} pending {} \u{b7} skipped {}",
        totals.pending, totals.skipped
    );
    line
}

fn outcome_label(outcome: &AcquisitionOutcome, source: Option<&str>) -> String {
    match outcome {
        AcquisitionOutcome::Mapped => source.map_or_else(
            || "mapped".to_owned(),
            |source| format!("mapped (source {source})"),
        ),
        AcquisitionOutcome::AlreadyMapped => "already_mapped".to_owned(),
        AcquisitionOutcome::AlreadyLocal => "already_local".to_owned(),
        AcquisitionOutcome::Unavailable { reason } => {
            format!("unavailable ({})", reason.as_str())
        }
        AcquisitionOutcome::SkippedUnavailable { reason, checked_at } => format!(
            "skipped_unavailable ({}, checked {})",
            reason.as_str(),
            checked_at.format("%Y-%m-%d")
        ),
        AcquisitionOutcome::Pending { error } => format!("pending ({error})"),
        AcquisitionOutcome::Busy => "busy (another run holds this Work)".to_owned(),
    }
}

fn work_title_line(report: &AcquireReport) -> String {
    let title = report.title.as_deref().unwrap_or("(untitled)");
    report
        .year
        .map_or_else(|| title.to_owned(), |year| format!("{title} ({year})"))
}

fn format_byte_size(bytes: i64) -> String {
    const KIB: f64 = 1024.0;
    // Display only; precision loss above 2^52 bytes is irrelevant.
    #[allow(clippy::cast_precision_loss)]
    let value = bytes.max(0) as f64;
    if value >= KIB * KIB {
        format!("{:.1} MiB", value / (KIB * KIB))
    } else {
        format!("{:.1} KiB", value / KIB)
    }
}

fn render_acquisition_summary(report: &AcquireReport) -> String {
    let summary = &report.summary;
    let mut output = String::new();
    let _ = writeln!(
        output,
        "Work       {}  {}",
        summary.work_id,
        work_title_line(report)
    );
    let source = summary.source.map(lysilogy_core::CandidateOrigin::as_str);
    let _ = writeln!(
        output,
        "Outcome    {}",
        outcome_label(&summary.outcome, source)
    );
    if !summary.stages.is_empty() {
        let stages: Vec<&str> = summary.stages.iter().map(|stage| stage.as_str()).collect();
        let _ = writeln!(output, "Stages     {}", stages.join(" \u{2192} "));
    }
    if let Some(url) = &summary.pdf_url {
        let detail = match (summary.bytes, &summary.sha256) {
            (Some(bytes), Some(sha)) => format!(
                "  ({}, sha256 {}\u{2026})",
                format_byte_size(bytes),
                &sha[..sha.len().min(8)]
            ),
            _ => String::new(),
        };
        let _ = writeln!(output, "PDF        {url}{detail}");
    }
    if let (Some(paper), Some(path)) = (&report.paper_id, &summary.relative_path) {
        let _ = writeln!(output, "Paper      {paper}  {path}");
    }
    for warning in &summary.warnings {
        let _ = writeln!(output, "Warnings   {warning}");
    }
    output
}

fn render_cited_by_line(report: &AcquireReport) -> String {
    let summary = &report.summary;
    let kind = match &summary.outcome {
        AcquisitionOutcome::Mapped => "mapped",
        AcquisitionOutcome::AlreadyMapped => "already_mapped",
        AcquisitionOutcome::AlreadyLocal => "already_local",
        AcquisitionOutcome::Unavailable { .. } => "unavailable",
        AcquisitionOutcome::SkippedUnavailable { .. } => "skipped",
        AcquisitionOutcome::Pending { .. } => "pending",
        AcquisitionOutcome::Busy => "busy",
    };
    let detail = match &summary.outcome {
        AcquisitionOutcome::Unavailable { reason }
        | AcquisitionOutcome::SkippedUnavailable { reason, .. } => reason.as_str().to_owned(),
        AcquisitionOutcome::Pending { error } => error.clone(),
        _ => summary.relative_path.clone().unwrap_or_default(),
    };
    let mut line = format!("{kind} {} {detail}", work_title_line(report))
        .trim_end()
        .to_owned();
    if !summary.warnings.is_empty() {
        let _ = write!(line, " [warnings: {}]", summary.warnings.join(", "));
    }
    line
}

fn render_literature_summary(summary: &LiteratureSummary) -> String {
    let mut output = String::new();
    match (&summary.status, summary.work_id) {
        (LiteratureStatus::Linked, Some(work_id)) => {
            let how = match (summary.link_method, &summary.link_detail) {
                (Some(method), Some(detail)) => format!("  ({method}: {detail})"),
                _ => String::new(),
            };
            let _ = writeln!(output, "Work       {work_id}{how}");
        }
        (LiteratureStatus::Linked, None) => {
            let _ = writeln!(output, "Work       linked without a Work ID");
        }
        (LiteratureStatus::Unmatched { reason }, _) => {
            let _ = writeln!(output, "Unmatched  {reason}");
        }
    }
    if let Some(openalex_id) = &summary.openalex_id {
        let _ = writeln!(output, "OpenAlex   {openalex_id}");
        let _ = writeln!(
            output,
            "References {} (new stub works: {}, titled: {})",
            summary.references, summary.new_stub_works, summary.hydrated_works
        );
    }
    output.push_str(&render_publisher(&summary.publisher));
    for warning in &summary.warnings {
        let _ = writeln!(output, "Warnings   {warning}");
    }
    output
}

/// The paper's own publisher lookup, then the cited ones as counts.
fn render_publisher(fetches: &[PublisherFetch]) -> String {
    let mut output = String::new();
    let label = |outcome: PublisherOutcome| match outcome {
        PublisherOutcome::Fetched => "fetched",
        PublisherOutcome::Cached => "cached",
        PublisherOutcome::NotFound => "not found",
        PublisherOutcome::Failed => "failed",
    };
    for fetch in fetches.iter().filter(|fetch| !fetch.cited) {
        let _ = writeln!(output, "Publisher  {} {}", fetch.doi, label(fetch.outcome));
    }
    let cited: Vec<&PublisherFetch> = fetches.iter().filter(|fetch| fetch.cited).collect();
    if cited.is_empty() {
        return output;
    }
    let count = |outcome: PublisherOutcome| {
        cited
            .iter()
            .filter(|fetch| fetch.outcome == outcome)
            .count()
    };
    let _ = writeln!(
        output,
        "Publisher  cited {} (fetched {}, cached {}, not found {}, failed {})",
        cited.len(),
        count(PublisherOutcome::Fetched),
        count(PublisherOutcome::Cached),
        count(PublisherOutcome::NotFound),
        count(PublisherOutcome::Failed)
    );
    output
}

fn render_local_evidence(summary: &LocalEvidenceSummary) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "Local      entries {} \u{b7} mentions {} (unresolved {}) \u{b7} field claims {}",
        summary.entries, summary.mentions, summary.unresolved_mentions, summary.field_claims
    );
    let counts = &summary.resolutions;
    let _ = writeln!(
        output,
        "Resolved   {} (identifier {}, title {}, stub {}, reused {}) \u{b7} ambiguous {} \u{b7} merged {}",
        summary.resolved_entries,
        counts.identifier,
        counts.title_match,
        counts.local_stub,
        counts.reused,
        summary.ambiguous_entries,
        summary.merged_stubs
    );
    let _ = writeln!(
        output,
        "Edges      local {} (also provider {})",
        summary.local_edges, summary.edges_also_from_provider
    );
    if let Some(labels) = &summary.labels {
        output.push_str(&render_labels(labels));
    }
    if let Some(fields) = &summary.fields {
        output.push_str(&render_fields("Fields    ", fields));
    }
    if let Some(fields) = &summary.paper_fields {
        output.push_str(&render_fields("Paper     ", fields));
    }
    for warning in &summary.warnings {
        let _ = writeln!(output, "Warnings   {warning}");
    }
    output
}

fn render_fields(heading: &str, fields: &FieldHarvestSummary) -> String {
    let mut line = format!(
        "{heading} considered {} \u{b7} model calls {} \u{b7} cached {} \u{b7} claims {} \u{b7} dropped {}",
        fields.entries_considered,
        fields.model_calls,
        fields.cache_hits,
        fields.claims_written,
        fields.dropped_total()
    );
    if !fields.dropped.is_empty() {
        let codes: Vec<String> = fields
            .dropped
            .iter()
            .map(|(code, count)| format!("{code} {count}"))
            .collect();
        let _ = write!(line, " ({})", codes.join(", "));
    }
    let _ = write!(line, " \u{b7} failed {}", fields.failed);
    if fields.skipped_higher_tier > 0 {
        let _ = write!(
            line,
            " \u{b7} skipped (higher tier) {}",
            fields.skipped_higher_tier
        );
    }
    if fields.over_limit > 0 {
        let _ = write!(line, " \u{b7} over limit {}", fields.over_limit);
    }
    line.push('\n');
    line
}

fn render_labels(labels: &LabelSummary) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "Labels     edges {} \u{b7} calls {} \u{b7} cached {} \u{b7} labeled {} \u{b7} rejected (grounding) {} \u{b7} failed {} \u{b7} over limit {}",
        labels.edges_considered,
        labels.model_calls,
        labels.cache_hits,
        labels.labeled,
        labels.rejected_for_grounding,
        labels.failed,
        labels.over_limit
    );
    if !labels.rejections.is_empty() {
        let codes: Vec<String> = labels
            .rejections
            .iter()
            .map(|(code, count)| format!("{code} {count}"))
            .collect();
        let _ = writeln!(output, "Rejected   {}", codes.join(", "));
    }
    output
}

async fn audit_command(
    state: &AppState,
    query: Option<String>,
    all: bool,
    json: bool,
) -> Result<()> {
    let mut targets = Vec::<(PaperId, String)>::new();
    if all {
        for paper in state.library().await.papers {
            targets.push((paper.id, paper.metadata.title));
        }
    } else {
        let query =
            query.ok_or_else(|| Error::InvalidRequest("provide a paper or pass --all".into()))?;
        let id = resolve_paper(state, &query).await?;
        let title = state
            .library()
            .await
            .papers
            .into_iter()
            .find(|paper| paper.id == id)
            .map(|paper| paper.metadata.title)
            .unwrap_or_default();
        targets.push((id, title));
    }

    let mut reports = Vec::new();
    let mut signal_totals = BTreeMap::<&'static str, usize>::new();
    for (id, title) in &targets {
        match state.reading_index(id, false).await {
            Ok(index) => {
                let report = lysilogy::audit::inventory(&index);
                for signal in &report.signals {
                    *signal_totals.entry(signal.code).or_default() += 1;
                }
                if json {
                    reports.push(serde_json::json!({
                        "paper": id.to_string(),
                        "title": title,
                        "inventory": report,
                    }));
                } else if all {
                    println!(
                        "{}\tpages={}\ttokens={}\tfigures={}\tsignals={}\t{}",
                        id,
                        report.pages,
                        report.tokens,
                        report.figures,
                        report.signals.len(),
                        title
                    );
                } else {
                    print_inventory(id, title, &report);
                }
            }
            Err(error) => {
                if json {
                    reports.push(serde_json::json!({
                        "paper": id.to_string(),
                        "title": title,
                        "error": error.to_string(),
                    }));
                } else {
                    println!("{id}\tERROR\t{error}");
                }
            }
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
    } else if all {
        println!();
        println!("Audited {} papers.", targets.len());
        for (code, count) in &signal_totals {
            println!("  {code}: {count}");
        }
    }
    Ok(())
}

fn print_inventory(id: &PaperId, title: &str, report: &lysilogy::audit::ExtractionInventory) {
    println!("{id}  {title}");
    println!(
        "  pages {} (native {}, ocr {}, unavailable {}), {} without text, {} sparse",
        report.pages,
        report.native_pages,
        report.ocr_pages,
        report.unavailable_pages,
        report.pages_without_text,
        report.sparse_pages
    );
    println!(
        "  text {} chars, {} tokens, {} words, {} sentences, {} paragraphs",
        report.text_chars, report.tokens, report.words, report.sentences, report.paragraphs
    );
    if !report.paragraphs_by_kind.is_empty() {
        println!("  kinds: {}", format_counts(&report.paragraphs_by_kind));
    }
    println!(
        "  figures {} ({}), {} references",
        report.figures,
        format_counts(&report.figures_by_kind),
        report.figure_references
    );
    if report.gaps > 0 {
        println!(
            "  gaps {} ({})",
            report.gaps,
            format_counts(&report.gaps_by_reason)
        );
    }
    if report.signals.is_empty() {
        println!("  clean");
    } else {
        for signal in &report.signals {
            println!(
                "  [{:?}] {}: {}",
                signal.severity, signal.code, signal.detail
            );
        }
    }
}

fn format_counts(counts: &BTreeMap<String, usize>) -> String {
    counts
        .iter()
        .map(|(kind, count)| format!("{kind} {count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_extract_summary(
    record: &lysilogy::paper_artifacts::run::ExtractRecord,
    output: &std::path::Path,
) {
    println!(
        "{} · {} pages · {}",
        record.model,
        record.pages,
        record.pdf.display()
    );
    for call in &record.calls {
        println!(
            "  {:<16} {:>7} in {:>7} out  ${:.4}  {:>6.1}s  {}",
            call.stage,
            call.prompt_tokens,
            call.completion_tokens,
            call.cost_usd,
            call.latency_s,
            call.finish_reason.as_deref().unwrap_or("-"),
        );
    }
    println!(
        "  total ${:.4} · {:.1}s wall",
        record.totals.cost_usd, record.totals.wall_s
    );
    if !record.admission_counts.is_empty() {
        let counts = record
            .admission_counts
            .iter()
            .map(|(status, count)| format!("{status} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  quotes: {counts}");
    }
    if !record.correction_outcomes.is_empty() {
        println!(
            "  self-check: {} corrections",
            record.correction_outcomes.len()
        );
    }
    if let Some(reason) = &record.budget_stopped {
        println!("  budget: {reason}");
    }
    for failure in &record.failures {
        println!("  failed {}: {}", failure.stage, failure.error);
    }
    println!("  wrote {}", output.display());
}

async fn resolve_paper(state: &AppState, query: &str) -> Result<PaperId> {
    if let Ok(id) = query.parse::<PaperId>()
        && state.paper(&id).await.is_ok()
    {
        return Ok(id);
    }
    let lowered = query.to_lowercase();
    let matches = state
        .library()
        .await
        .papers
        .into_iter()
        .filter(|paper| paper.metadata.title.to_lowercase().contains(&lowered))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [paper] => Ok(paper.id.clone()),
        [] => Err(Error::PaperNotFound(query.to_owned())),
        _ => Err(Error::InvalidRequest(format!(
            "title fragment matched {} papers; use a more specific title or paper ID",
            matches.len()
        ))),
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("lysilogy=info,tower_http=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn summary(status: LiteratureStatus) -> LiteratureSummary {
        LiteratureSummary {
            status,
            document_id: lysilogy_core::DocumentId::new(),
            work_id: None,
            link_method: None,
            link_detail: None,
            openalex_id: None,
            references: 0,
            new_stub_works: 0,
            hydrated_works: 0,
            publisher: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn should_return_relative_path_when_target_is_inside_library() {
        let relative =
            lit_target_relative_path(Path::new("/lib"), Path::new("/lib/sub/a.pdf")).unwrap();
        assert_eq!(relative, Path::new("sub/a.pdf"));
    }

    #[test]
    fn should_reject_path_outside_library_when_resolving_lit_target() {
        for outside in ["/elsewhere/a.pdf", "/lib2/a.pdf", "/a.pdf"] {
            let error =
                lit_target_relative_path(Path::new("/lib"), Path::new(outside)).expect_err(outside);
            assert!(matches!(error, Error::InvalidRequest(_)), "{outside}");
        }
    }

    #[test]
    fn should_render_three_lines_when_local_evidence_has_no_warnings() {
        let evidence = LocalEvidenceSummary {
            entries: 48,
            mentions: 112,
            unresolved_mentions: 9,
            resolved_entries: 46,
            resolutions: lysilogy_ingest::local_evidence::ResolutionCounts {
                identifier: 5,
                title_match: 30,
                local_stub: 11,
                reused: 0,
            },
            ambiguous_entries: 2,
            merged_stubs: 0,
            local_edges: 44,
            edges_also_from_provider: 31,
            field_claims: 187,
            ..LocalEvidenceSummary::default()
        };
        let text = render_local_evidence(&evidence);
        assert_eq!(
            text,
            "Local      entries 48 \u{b7} mentions 112 (unresolved 9) \u{b7} field claims 187\n\
             Resolved   46 (identifier 5, title 30, stub 11, reused 0) \u{b7} ambiguous 2 \u{b7} merged 0\n\
             Edges      local 44 (also provider 31)\n"
        );
    }

    #[test]
    fn should_render_publisher_lines_when_summary_has_fetches() {
        let fetch = |doi: &str, outcome, cited| PublisherFetch {
            doi: doi.to_owned(),
            outcome,
            cited,
        };
        let mut linked = summary(LiteratureStatus::Linked);
        linked.publisher = vec![
            fetch("10.1/own", PublisherOutcome::NotFound, false),
            fetch("10.1/a", PublisherOutcome::Fetched, true),
            fetch("10.1/b", PublisherOutcome::Cached, true),
            fetch("10.1/c", PublisherOutcome::Failed, true),
        ];
        let text = render_literature_summary(&linked);
        assert!(text.contains("Publisher  10.1/own not found\n"), "{text}");
        assert!(
            text.contains("Publisher  cited 3 (fetched 1, cached 1, not found 0, failed 1)\n"),
            "{text}"
        );
        linked.publisher = vec![fetch("10.1/own", PublisherOutcome::Fetched, false)];
        assert!(render_literature_summary(&linked).contains("Publisher  10.1/own fetched\n"));
    }

    #[test]
    fn should_render_labels_line_when_labels_present() {
        let mut labels = LabelSummary {
            edges_considered: 44,
            model_calls: 20,
            cache_hits: 0,
            labeled: 17,
            rejected_for_grounding: 2,
            failed: 1,
            over_limit: 24,
            ..LabelSummary::default()
        };
        labels.rejections.insert("quote_too_short".to_owned(), 2);
        let evidence = LocalEvidenceSummary {
            labels: Some(labels),
            ..LocalEvidenceSummary::default()
        };
        let text = render_local_evidence(&evidence);
        assert!(
            text.ends_with(
                "Labels     edges 44 \u{b7} calls 20 \u{b7} cached 0 \u{b7} labeled 17 \u{b7} rejected (grounding) 2 \u{b7} failed 1 \u{b7} over limit 24\n\
                 Rejected   quote_too_short 2\n"
            ),
            "{text}"
        );
        let plain = render_local_evidence(&LocalEvidenceSummary::default());
        assert!(!plain.contains("Labels"), "{plain}");
    }

    #[test]
    fn should_map_sidecar_to_provider_when_choosing_label_model() {
        assert_eq!(LabelSidecar::Claude.provider(), AnalysisProvider::Claude);
        assert_eq!(LabelSidecar::Codex.provider(), AnalysisProvider::Codex);
    }

    #[test]
    fn should_print_unmatched_reason_and_warnings_when_no_work_found() {
        let mut result = summary(LiteratureStatus::Unmatched {
            reason: "title_too_generic".to_owned(),
        });
        result.warnings.push("title_too_generic".to_owned());
        let text = render_literature_summary(&result);
        assert!(text.contains("Unmatched  title_too_generic"), "{text}");
        assert!(text.contains("Warnings   title_too_generic"), "{text}");
        assert!(!text.contains("OpenAlex"), "{text}");
    }

    fn report(outcome: AcquisitionOutcome) -> AcquireReport {
        use lysilogy_core::{AcquisitionState, CandidateOrigin, WorkId};
        use lysilogy_ingest::acquire::AcquisitionSummary;

        AcquireReport {
            summary: AcquisitionSummary {
                work_id: WorkId::from_uuid(uuid::Uuid::from_u128(7)),
                outcome,
                state: AcquisitionState::Mapped,
                stages: vec![
                    AcquisitionState::IdentifierFound,
                    AcquisitionState::LinkFound,
                    AcquisitionState::Downloaded,
                    AcquisitionState::Mapped,
                ],
                candidates: 1,
                tried: 1,
                source: Some(CandidateOrigin::Arxiv),
                pdf_url: Some("https://arxiv.org/pdf/1606.03137".to_owned()),
                sha256: Some("3f2a9c4e11223344".to_owned()),
                bytes: Some(1_300_000),
                document_id: None,
                relative_path: Some(
                    "Fetched/2016 - Cooperative Inverse Reinforcement Learning.pdf".to_owned(),
                ),
                warnings: vec!["unpaywall_skipped_no_email".to_owned()],
            },
            paper_id: Some(PaperId::from_relative_path(Path::new("Fetched/a.pdf"))),
            title: Some("Cooperative Inverse Reinforcement Learning".to_owned()),
            year: Some(2016),
        }
    }

    #[test]
    fn should_render_mapped_summary_when_outcome_is_mapped() {
        let output = render_acquisition_summary(&report(AcquisitionOutcome::Mapped));
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 6, "{output}");
        assert!(lines[0].starts_with("Work       w"), "{}", lines[0]);
        assert!(
            lines[0].ends_with("Cooperative Inverse Reinforcement Learning (2016)"),
            "{}",
            lines[0]
        );
        assert_eq!(lines[1], "Outcome    mapped (source arxiv)");
        assert_eq!(
            lines[2],
            "Stages     identifier_found \u{2192} link_found \u{2192} downloaded \u{2192} mapped"
        );
        assert_eq!(
            lines[3],
            "PDF        https://arxiv.org/pdf/1606.03137  (1.2 MiB, sha256 3f2a9c4e\u{2026})"
        );
        assert!(
            lines[4].ends_with("Fetched/2016 - Cooperative Inverse Reinforcement Learning.pdf"),
            "{}",
            lines[4]
        );
        assert_eq!(lines[5], "Warnings   unpaywall_skipped_no_email");
    }

    #[test]
    fn should_render_reason_without_paper_when_outcome_is_unavailable() {
        use lysilogy_core::UnavailableReason;

        let mut unavailable = report(AcquisitionOutcome::Unavailable {
            reason: UnavailableReason::NoOaLocation,
        });
        unavailable.paper_id = None;
        unavailable.summary.stages = vec![];
        unavailable.summary.pdf_url = None;
        unavailable.title = None;
        unavailable.year = None;
        let output = render_acquisition_summary(&unavailable);
        assert!(output.contains("(untitled)"), "{output}");
        assert!(
            output.contains("Outcome    unavailable (no_oa_location)"),
            "{output}"
        );
        assert!(!output.contains("Stages"), "{output}");
        assert!(!output.contains("Paper"), "{output}");
        assert!(!output.contains("PDF"), "{output}");
    }

    #[test]
    fn should_render_totals_by_reason_when_cited_by_finishes() {
        use lysilogy_core::UnavailableReason;

        let mut totals = CitedByTotals {
            skipped: 2,
            ..CitedByTotals::default()
        };
        for _ in 0..3 {
            totals.record(&AcquisitionOutcome::Mapped);
        }
        for _ in 0..5 {
            totals.record(&AcquisitionOutcome::AlreadyLocal);
        }
        for _ in 0..3 {
            totals.record(&AcquisitionOutcome::Unavailable {
                reason: UnavailableReason::NoOaLocation,
            });
        }
        totals.record(&AcquisitionOutcome::Unavailable {
            reason: UnavailableReason::AllLinksFailed,
        });
        totals.record(&AcquisitionOutcome::Pending {
            error: "http_503".to_owned(),
        });
        assert_eq!(
            render_cited_by_totals(&totals),
            "Fetched 3 \u{b7} already local 5 \u{b7} unavailable 4 (all_links_failed 1, no_oa_location 3) \u{b7} pending 1 \u{b7} skipped 2"
        );
    }

    #[test]
    fn should_render_zero_totals_when_nothing_was_attempted() {
        assert_eq!(
            render_cited_by_totals(&CitedByTotals::default()),
            "Fetched 0 \u{b7} already local 0 \u{b7} unavailable 0 \u{b7} pending 0 \u{b7} skipped 0"
        );
    }

    #[test]
    fn should_render_one_line_per_target_when_cited_by_runs() {
        let line = render_cited_by_line(&report(AcquisitionOutcome::Mapped));
        assert!(
            line.starts_with("mapped Cooperative Inverse Reinforcement Learning (2016) Fetched/"),
            "{line}"
        );
        let pending = render_cited_by_line(&report(AcquisitionOutcome::Pending {
            error: "http_503".to_owned(),
        }));
        assert!(pending.starts_with("pending "), "{pending}");
        assert!(pending.contains(" http_503"), "{pending}");
        let mut warned = report(AcquisitionOutcome::Mapped);
        warned
            .summary
            .warnings
            .push("title_not_in_front_matter".to_owned());
        let line = render_cited_by_line(&warned);
        assert!(line.contains("title_not_in_front_matter]"), "{line}");
    }

    #[test]
    fn should_reject_both_or_neither_target_when_parsing_fetch_arguments() {
        for arguments in [
            vec!["lysilogy", "fetch"],
            vec!["lysilogy", "fetch", "doi:10.1/x", "--cited-by", "Paper"],
        ] {
            assert!(Cli::try_parse_from(arguments).is_err());
        }
        assert!(Cli::try_parse_from(["lysilogy", "fetch", "doi:10.1/x"]).is_ok());
        assert!(
            Cli::try_parse_from(["lysilogy", "fetch", "--cited-by", "Paper", "--limit", "5"])
                .is_ok()
        );
    }

    #[test]
    fn should_accept_publisher_flag_when_parsing_lit() {
        let plain = Cli::try_parse_from(["lysilogy", "lit", "Paper"]).expect("plain");
        assert!(matches!(
            plain.command,
            Some(Command::Lit {
                publisher: false,
                ..
            })
        ));
        let flagged = Cli::try_parse_from(["lysilogy", "lit", "Paper", "--local", "--publisher"])
            .expect("flagged");
        assert!(matches!(
            flagged.command,
            Some(Command::Lit {
                local: true,
                publisher: true,
                ..
            })
        ));
    }

    #[test]
    fn should_run_fields_by_default_when_parsing_lit() {
        let plain = Cli::try_parse_from(["lysilogy", "lit", "Paper"]).expect("plain");
        assert!(matches!(
            plain.command,
            Some(Command::Lit {
                no_fields: false,
                fields_limit: 10,
                ..
            })
        ));
        let flagged = Cli::try_parse_from([
            "lysilogy",
            "lit",
            "Paper",
            "--no-fields",
            "--sidecar",
            "codex",
            "--fields-limit",
            "3",
        ])
        .expect("flagged");
        assert!(matches!(
            flagged.command,
            Some(Command::Lit {
                no_fields: true,
                fields_limit: 3,
                sidecar: LabelSidecar::Codex,
                ..
            })
        ));
        assert!(Cli::try_parse_from(["lysilogy", "lit", "Paper", "--fields-limit", "x"]).is_err());
    }

    #[test]
    fn should_parse_provenance_subcommands_when_arguments_valid() {
        let show =
            Cli::try_parse_from(["lysilogy", "provenance", "show", "w1", "--paper"]).expect("show");
        assert!(matches!(
            show.command,
            Some(Command::Provenance {
                command: ProvenanceCommand::Show { paper: true, .. }
            })
        ));
        let set = Cli::try_parse_from([
            "lysilogy",
            "provenance",
            "set",
            "w1",
            "authors",
            "A. One; B. Two",
        ])
        .expect("set");
        match set.command {
            Some(Command::Provenance {
                command:
                    ProvenanceCommand::Set {
                        work,
                        paper,
                        field,
                        value,
                    },
            }) => {
                assert_eq!(work, "w1");
                assert!(!paper);
                assert_eq!(field, "authors");
                assert_eq!(value, "A. One; B. Two");
            }
            other => panic!("unexpected {other:?}"),
        }
        let clear =
            Cli::try_parse_from(["lysilogy", "provenance", "clear", "w1", "--paper", "year"])
                .expect("clear");
        assert!(matches!(
            clear.command,
            Some(Command::Provenance {
                command: ProvenanceCommand::Clear { paper: true, .. }
            })
        ));
        assert!(Cli::try_parse_from(["lysilogy", "provenance", "set", "w1", "year"]).is_err());
        assert!(Cli::try_parse_from(["lysilogy", "provenance", "show"]).is_err());
    }

    #[test]
    fn should_render_fields_line_when_fields_present() {
        let mut fields = FieldHarvestSummary {
            entries_considered: 31,
            model_calls: 2,
            cache_hits: 4,
            claims_written: 57,
            failed: 1,
            ..FieldHarvestSummary::default()
        };
        fields.dropped.insert("year_not_in_entry".to_owned(), 3);
        fields.dropped.insert("author_not_in_entry".to_owned(), 2);
        let line = render_fields("Fields    ", &fields);
        assert_eq!(
            line,
            "Fields     considered 31 \u{b7} model calls 2 \u{b7} cached 4 \u{b7} claims 57 \u{b7} dropped 5 (author_not_in_entry 2, year_not_in_entry 3) \u{b7} failed 1\n"
        );
    }

    #[test]
    fn should_reject_reselect_without_work_or_all() {
        assert!(Cli::try_parse_from(["lysilogy", "provenance", "reselect"]).is_err());
        assert!(
            Cli::try_parse_from(["lysilogy", "provenance", "reselect", "w1", "--all"]).is_err()
        );
        assert!(Cli::try_parse_from(["lysilogy", "provenance", "reselect", "--paper"]).is_err());
        let one = Cli::try_parse_from(["lysilogy", "provenance", "reselect", "w1", "--paper"])
            .expect("work");
        assert!(matches!(
            one.command,
            Some(Command::Provenance {
                command: ProvenanceCommand::Reselect {
                    work: Some(_),
                    paper: true,
                    all: false
                }
            })
        ));
        let all =
            Cli::try_parse_from(["lysilogy", "provenance", "reselect", "--all"]).expect("all");
        assert!(matches!(
            all.command,
            Some(Command::Provenance {
                command: ProvenanceCommand::Reselect {
                    work: None,
                    paper: false,
                    all: true
                }
            })
        ));
    }

    #[test]
    fn should_require_exactly_one_scope_when_parsing_prominence_command() {
        assert!(Cli::try_parse_from(["lysilogy", "prominence"]).is_err());
        assert!(Cli::try_parse_from(["lysilogy", "prominence", "Paper", "--all"]).is_err());
        assert!(Cli::try_parse_from(["lysilogy", "prominence", "--fetch-metrics"]).is_err());
        let paper = Cli::try_parse_from(["lysilogy", "prominence", "Paper"]).expect("paper");
        assert!(matches!(
            paper.command,
            Some(Command::Prominence {
                paper: Some(_),
                all: false,
                fetch_metrics: false
            })
        ));
        let all = Cli::try_parse_from(["lysilogy", "prominence", "--all", "--fetch-metrics"])
            .expect("all");
        assert!(matches!(
            all.command,
            Some(Command::Prominence {
                paper: None,
                all: true,
                fetch_metrics: true
            })
        ));
    }

    fn ramp_command_of(arguments: &[&str]) -> Option<RampCommand> {
        let mut full = vec!["lysilogy", "ramp"];
        full.extend_from_slice(arguments);
        match Cli::try_parse_from(full).ok()?.command {
            Some(Command::Ramp { command }) => Some(command),
            _ => None,
        }
    }

    #[test]
    fn should_parse_ramp_subcommands_when_arguments_valid() {
        assert!(matches!(
            ramp_command_of(&["new", "doi:10.1/x", "--title", "T"]),
            Some(RampCommand::New {
                paper: false,
                title: Some(_),
                ..
            })
        ));
        assert!(matches!(
            ramp_command_of(&["new", "Some title", "--paper"]),
            Some(RampCommand::New {
                paper: true,
                title: None,
                ..
            })
        ));
        assert!(matches!(
            ramp_command_of(&["list"]),
            Some(RampCommand::List)
        ));
        assert!(matches!(
            ramp_command_of(&["show", "Learning"]),
            Some(RampCommand::Show { .. })
        ));
        assert!(matches!(
            ramp_command_of(&["add", "p1", "w1", "--via", "w2", "--note", "n"]),
            Some(RampCommand::Add {
                via: Some(_),
                note: Some(_),
                ..
            })
        ));
        assert!(matches!(
            ramp_command_of(&["set", "p1", "w1", "--status", "reading"]),
            Some(RampCommand::Set {
                status: Some(RampItemStatus::Reading),
                clear_note: false,
                ..
            })
        ));
        assert!(matches!(
            ramp_command_of(&["set", "p1", "w1", "--clear-note"]),
            Some(RampCommand::Set {
                status: None,
                clear_note: true,
                ..
            })
        ));
        assert!(matches!(
            ramp_command_of(&["remove", "p1", "w1"]),
            Some(RampCommand::Remove { .. })
        ));
        assert!(matches!(
            ramp_command_of(&["order", "p1", "--suggest"]),
            Some(RampCommand::Order {
                suggest: true,
                works,
                ..
            }) if works.is_empty()
        ));
        assert!(matches!(
            ramp_command_of(&["order", "p1", "w1", "w2", "w3"]),
            Some(RampCommand::Order {
                suggest: false,
                works,
                ..
            }) if works.len() == 3
        ));
        assert!(matches!(
            ramp_command_of(&["delete", "p1"]),
            Some(RampCommand::Delete { .. })
        ));
        for arguments in [
            vec!["lysilogy", "ramp"],
            vec!["lysilogy", "ramp", "show"],
            vec!["lysilogy", "ramp", "add", "p1"],
            vec!["lysilogy", "ramp", "set", "p1", "w1", "--status", "done"],
            vec!["lysilogy", "ramp", "frobnicate"],
        ] {
            assert!(
                Cli::try_parse_from(arguments.clone()).is_err(),
                "{arguments:?}"
            );
        }
    }

    #[test]
    fn should_reject_order_without_suggest_or_works_when_parsing() {
        assert!(ramp_command_of(&["order", "p1"]).is_none());
        assert!(ramp_command_of(&["order", "p1", "--suggest", "w1"]).is_none());
    }

    #[test]
    fn should_reject_note_with_clear_note_when_parsing_set() {
        assert!(ramp_command_of(&["set", "p1", "w1", "--note", "n", "--clear-note"]).is_none());
        assert!(
            ramp_command_of(&["set", "p1", "w1"]).is_none(),
            "set needs something to change"
        );
        assert!(ramp_command_of(&["set", "p1", "w1", "--note", "n"]).is_some());
    }

    #[test]
    fn should_render_ramp_item_line_with_via_and_local_state() {
        use lysilogy::ramps::PathStep;
        use lysilogy_core::{FieldState, Visibility, WorkKind};
        let (seed, child) = (WorkId::new(), WorkId::new());
        let now = chrono::Utc::now();
        let item = RampItemView {
            work: lysilogy_core::Work {
                id: child,
                kind: WorkKind::Article,
                visibility: Visibility::Private,
                display_title: Some("Child".to_owned()),
                title_state: FieldState::Known,
                display_year: Some(2019),
                year_state: FieldState::Known,
                display_authors: None,
                authors_state: FieldState::Unknown,
                created_at: now,
                updated_at: now,
            },
            ordinal: 2,
            depth: 1,
            via_work_id: Some(seed),
            path: vec![
                PathStep {
                    work_id: seed,
                    title: Some("Seed".to_owned()),
                },
                PathStep {
                    work_id: child,
                    title: Some("Child".to_owned()),
                },
            ],
            status: RampItemStatus::Reading,
            note: None,
            added_at: now,
            updated_at: now,
            status_changed_at: None,
        };
        assert_eq!(
            render_ramp_item_line(&item, Some("held")),
            "2. [reading] Child (2019) · depth 1 · via Seed · held"
        );
        assert_eq!(
            render_ramp_item_line(&item, None),
            "2. [reading] Child (2019) · depth 1 · via Seed"
        );
    }

    #[test]
    fn should_reject_depth_and_fanout_out_of_range_when_parsing_expand_command() {
        for arguments in [
            vec!["lysilogy", "expand", "w1", "--depth", "0"],
            vec!["lysilogy", "expand", "w1", "--depth", "4"],
            vec!["lysilogy", "expand", "w1", "--depth", "-1"],
            vec!["lysilogy", "expand", "w1", "--depth", "256"],
            vec!["lysilogy", "expand", "w1", "--fanout", "0"],
            vec!["lysilogy", "expand", "w1", "--fanout", "11"],
            vec!["lysilogy", "expand", "w1", "--fanout", "many"],
            vec!["lysilogy", "expand"],
        ] {
            assert!(
                Cli::try_parse_from(arguments.clone()).is_err(),
                "{arguments:?}"
            );
        }
        let defaults = Cli::try_parse_from(["lysilogy", "expand", "doi:10.1/x"]).expect("defaults");
        assert!(matches!(
            defaults.command,
            Some(Command::Expand {
                paper: false,
                depth: 1,
                fanout: 5,
                refresh: false,
                no_local: false,
                ..
            })
        ));
        let bounded = Cli::try_parse_from([
            "lysilogy",
            "expand",
            "Title",
            "--paper",
            "--depth",
            "3",
            "--fanout",
            "10",
            "--refresh",
            "--no-local",
        ])
        .expect("bounded");
        assert!(matches!(
            bounded.command,
            Some(Command::Expand {
                paper: true,
                depth: 3,
                fanout: 10,
                refresh: true,
                no_local: true,
                ..
            })
        ));
    }

    fn expansion_report(work_id: WorkId, state: ExpansionState) -> ExpansionReport {
        ExpansionReport {
            work_id,
            cached: false,
            state,
            method: None,
            unavailable_reason: None,
            references: 0,
            new_works: 0,
            paper_id: None,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn should_render_one_line_per_expansion_and_a_walk_summary_when_walking() {
        let (first, second, third) = (WorkId::new(), WorkId::new(), WorkId::new());
        let mut expanded = expansion_report(first, ExpansionState::Expanded);
        expanded.method = Some(lysilogy_core::ExpansionMethod::Provider);
        expanded.references = 12;
        expanded.new_works = 4;
        expanded.warnings.push("local_flow_unmatched".to_owned());
        let mut cached = expansion_report(second, ExpansionState::Expanded);
        cached.cached = true;
        cached.method = Some(lysilogy_core::ExpansionMethod::Existing);
        cached.references = 7;
        let mut unavailable = expansion_report(third, ExpansionState::Unavailable);
        unavailable.unavailable_reason =
            Some(lysilogy_core::ExpansionUnavailableReason::ProviderNotFound);
        let report = WalkReport {
            seed: first,
            expansions: vec![expanded, cached, unavailable],
            levels: 2,
            truncated: true,
        };
        let titles = BTreeMap::from([(first, "First Paper".to_owned())]);
        let text = render_walk_report(&report, &titles, 1.26);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "First Paper: expanded (provider, 12 references, 4 new)"
        );
        assert_eq!(lines[1], "  warning: local_flow_unmatched");
        assert_eq!(
            lines[2],
            format!("{second}: cached (existing, 7 references)")
        );
        assert_eq!(
            lines[3],
            format!("{third}: unavailable (provider_not_found)")
        );
        assert_eq!(
            lines[4],
            "Walked 2 levels, 3 expansions, truncated at 40 expansions in 1.3 s"
        );
    }

    #[test]
    fn should_render_counts_and_warnings_when_summarizing_prominence() {
        let summary = RefreshSummary {
            works: 12,
            with_global_count: 9,
            metrics_fetched: 9,
            metric_requests: 1,
            warnings: vec!["prominence_metrics_failed:boom".to_owned()],
        };
        let text = render_prominence_summary(&summary, true, 1.26);
        assert!(
            text.starts_with("Recomputed 12 works (9 with global counts) in 1.3 s\n"),
            "{text}"
        );
        assert!(
            text.contains("Fetched cited-by counts for 9 works (1 requests)"),
            "{text}"
        );
        assert!(
            text.contains("Warnings   prominence_metrics_failed:boom"),
            "{text}"
        );
        let quiet = render_prominence_summary(&summary, false, 0.0);
        assert!(!quiet.contains("Fetched"), "{quiet}");
    }
}
