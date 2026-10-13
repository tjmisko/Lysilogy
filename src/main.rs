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
use lysilogy::literature::evidence::LOCAL_EVIDENCE_REQUIRES_WORK;
use lysilogy::{
    AppState, Error, Result, build_router,
    domain::{
        AnalysisProvider, ExperimentArmScore, ExperimentJudgmentRequest, ExperimentRecord,
        ExperimentStatus, LearningRamp, PaperId, ProcessingStatus, StartExperimentRequest,
    },
};
use lysilogy::{analysis::LocalCliAnalyzer, labeling::SidecarLabeler};
use lysilogy_ingest::acquire::AcquisitionOutcome;
use lysilogy_ingest::labels::{LabelOptions, LabelSummary};
use lysilogy_ingest::literature::{LiteratureStatus, LiteratureSummary};
use lysilogy_ingest::local_evidence::LocalEvidenceSummary;
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
}

impl LabelSidecar {
    const fn provider(self) -> AnalysisProvider {
        match self {
            Self::Claude => AnalysisProvider::Claude,
            Self::Codex => AnalysisProvider::Codex,
        }
    }
}

/// How `lit` should label edges, when asked to.
struct LabelRun {
    sidecar: LabelSidecar,
    limit: usize,
    scratch: PathBuf,
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
    /// Analyze one paper selected by ID or an unambiguous title fragment.
    Analyze {
        query: String,
        #[arg(long, default_value = "codex")]
        provider: AnalysisProvider,
        #[arg(long)]
        force: bool,
    },
    /// Refresh only the authored abstract, preserving the map and context.
    RefreshAbstract {
        query: String,
        #[arg(long, default_value = "codex")]
        provider: AnalysisProvider,
        #[arg(long)]
        force: bool,
    },
    /// Refresh only cited research history and subsequent influence.
    RefreshContext {
        query: String,
        #[arg(long, default_value = "codex")]
        provider: AnalysisProvider,
        #[arg(long)]
        force: bool,
    },
    /// Regenerate only the section map, claims, and glossary as coherent reading units.
    RefreshStructure {
        query: String,
        #[arg(long, default_value = "codex")]
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
        #[arg(long, value_enum, default_value = "claude")]
        sidecar: LabelSidecar,
        /// Stop after this many model calls; cached answers do not count.
        #[arg(long, default_value_t = 25)]
        label_limit: usize,
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
    /// Analyze all papers that are not ready, continuing past individual faults.
    Ingest {
        #[arg(long, default_value = "codex")]
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
        #[arg(long, default_value = "codex")]
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
        #[arg(long, default_value = "codex")]
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
        } => {
            let label_run = labels.then(|| LabelRun {
                sidecar,
                limit: label_limit,
                scratch: cli.data.join("labels"),
            });
            literature_command(&state, &cli.library, &paper, local, label_run).await
        }
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

async fn literature_command(
    state: &AppState,
    library: &std::path::Path,
    paper: &str,
    local: bool,
    label_run: Option<LabelRun>,
) -> Result<()> {
    let id = resolve_lit_target(state, library, paper).await?;
    let summary = state.run_literature_flow(&id).await?;
    print!("{}", render_literature_summary(&summary));
    if local || label_run.is_some() {
        match state.run_local_evidence(&id, &summary).await? {
            Some(mut evidence) => {
                if let Some(run) = &label_run {
                    evidence.labels = label_local_edges(state, &id, &summary, run).await?;
                }
                print!("{}", render_local_evidence(&evidence));
            }
            None => println!("Warnings   {LOCAL_EVIDENCE_REQUIRES_WORK}"),
        }
    }
    if let LiteratureStatus::Unmatched { reason } = &summary.status {
        return Err(Error::Literature(format!(
            "no Work found for {id}: {reason}"
        )));
    }
    Ok(())
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
    format!("{kind} {} {detail}", work_title_line(report))
        .trim_end()
        .to_owned()
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
    for warning in &summary.warnings {
        let _ = writeln!(output, "Warnings   {warning}");
    }
    output
}

fn render_local_evidence(summary: &LocalEvidenceSummary) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "Local      entries {} \u{b7} mentions {} (unresolved {})",
        summary.entries, summary.mentions, summary.unresolved_mentions
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
    for warning in &summary.warnings {
        let _ = writeln!(output, "Warnings   {warning}");
    }
    output
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
            ..LocalEvidenceSummary::default()
        };
        let text = render_local_evidence(&evidence);
        assert_eq!(
            text,
            "Local      entries 48 \u{b7} mentions 112 (unresolved 9)\n\
             Resolved   46 (identifier 5, title 30, stub 11, reused 0) \u{b7} ambiguous 2 \u{b7} merged 0\n\
             Edges      local 44 (also provider 31)\n"
        );
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
        assert!(pending.ends_with("http_503"), "{pending}");
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
}
