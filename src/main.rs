use std::{net::SocketAddr, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use lysilogos::{
    AppState, Error, Result, build_router,
    domain::{AnalysisProvider, PaperId, ProcessingStatus},
};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Directory containing the PDF library.
    #[arg(
        long,
        env = "LYSILOGOS_LIBRARY",
        default_value = "local-articles/Articles",
        global = true
    )]
    library: PathBuf,

    /// Directory for portable text, Markdown, and JSON artifacts.
    #[arg(
        long,
        env = "LYSILOGOS_DATA",
        default_value = ".lysilogos",
        global = true
    )]
    data: PathBuf,

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

async fn run(cli: Cli) -> Result<()> {
    let state = AppState::new(&cli.library, &cli.data).await?;
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
        Command::Convert { query } => {
            let id = resolve_paper(&state, &query).await?;
            print!("{}", state.markdown(&id).await?);
            Ok(())
        }
        Command::Ingest {
            provider,
            force,
            limit,
        } => ingest(&state, provider, force, limit).await,
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
    tracing::info!(address = %bind, "Lysilogos is listening");
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
        .unwrap_or_else(|_| EnvFilter::new("lysilogos=info,tower_http=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
