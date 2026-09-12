use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    net::SocketAddr,
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
};

use clap::{Parser, Subcommand};
use lysilogy::{
    AppState, Error, Result, build_router,
    domain::{
        AnalysisProvider, ExperimentArmScore, ExperimentJudgmentRequest, ExperimentRecord,
        ExperimentStatus, LearningRamp, PaperId, ProcessingStatus, StartExperimentRequest,
    },
};
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

    #[command(subcommand)]
    command: Option<Command>,
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
    let state = AppState::new(&cli.library, &cli.data)
        .await?
        .with_notes_root(&cli.notes);
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
