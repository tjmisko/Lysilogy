use std::{collections::BTreeMap, sync::Arc};

use chrono::Utc;
use tokio::sync::RwLock;

use crate::{
    Result,
    domain::{
        AnalysisJob, AnalysisJobKind, AnalysisJobStatus, AnalysisProvider, AnalysisTask,
        AnalysisTaskStatus, PaperId, ProcessingQueue, ProcessingStage,
    },
    error::Error,
    store::ArtifactStore,
};

#[derive(Clone, Debug)]
pub struct JobTracker {
    jobs: Arc<RwLock<BTreeMap<PaperId, AnalysisJob>>>,
    store: ArtifactStore,
}

impl JobTracker {
    pub async fn load(store: ArtifactStore) -> Result<Self> {
        let mut jobs = BTreeMap::new();
        for mut job in store.load_jobs().await? {
            if job.status.is_active() {
                if let Some(markdown) = store.load_tasklist(&job.paper_id).await? {
                    let mut tasks = parse_tasklist(&markdown);
                    if !tasks.is_empty() {
                        for fallback in &job.tasks {
                            if !tasks.iter().any(|task| task.id == fallback.id) {
                                tasks.push(fallback.clone());
                            }
                        }
                        job.tasks = tasks;
                    }
                }
                job.status = AnalysisJobStatus::Failed {
                    stage: ProcessingStage::Analysis,
                    message:
                        "The server stopped while this run was active. Retry the paper to continue."
                            .to_owned(),
                    retryable: true,
                };
                job.updated_at = Utc::now();
                for task in &mut job.tasks {
                    if task.status == AnalysisTaskStatus::Active {
                        task.status = AnalysisTaskStatus::Failed;
                        task.detail = Some("Interrupted by a server restart".to_owned());
                    }
                }
                job.progress = progress(&job.tasks, &job.status);
                store
                    .save_tasklist(&job.paper_id, &render_tasklist(&job))
                    .await?;
                store.save_job(&job).await?;
            }
            jobs.insert(job.paper_id.clone(), job);
        }
        Ok(Self {
            jobs: Arc::new(RwLock::new(jobs)),
            store,
        })
    }

    pub async fn begin(
        &self,
        paper_id: PaperId,
        paper_title: String,
        provider: AnalysisProvider,
        kind: AnalysisJobKind,
        feedback: Option<String>,
    ) -> Result<AnalysisJob> {
        let now = Utc::now();
        let tasks = tasks_for(kind);
        let resumable = if kind == AnalysisJobKind::Revision {
            match self.store.load_agent_session(&paper_id).await {
                Ok(Some(session)) => session.provider == provider,
                Ok(None) => false,
                Err(error) => {
                    tracing::warn!(paper_id = %paper_id, %error, "could not inspect saved agent session");
                    false
                }
            }
        } else {
            false
        };
        let job = AnalysisJob {
            paper_id: paper_id.clone(),
            paper_title: compact_line(&paper_title),
            provider,
            kind,
            status: AnalysisJobStatus::Queued,
            progress: 0,
            tasks,
            resumable,
            feedback,
            created_at: now,
            updated_at: now,
        };
        self.store
            .save_tasklist(&paper_id, &render_tasklist(&job))
            .await?;
        self.store.save_job(&job).await?;
        self.jobs.write().await.insert(paper_id, job.clone());
        Ok(job)
    }

    pub async fn transition(
        &self,
        paper_id: &PaperId,
        stage: ProcessingStage,
        active_task: &str,
    ) -> Result<()> {
        self.update_task(paper_id, active_task, AnalysisTaskStatus::Active, None)
            .await?;
        self.update_job(paper_id, |job| {
            job.status = AnalysisJobStatus::Running { stage };
        })
        .await
    }

    pub async fn task_completed(&self, paper_id: &PaperId, task_id: &str) -> Result<()> {
        self.update_task(paper_id, task_id, AnalysisTaskStatus::Completed, None)
            .await
    }

    pub async fn task_active(
        &self,
        paper_id: &PaperId,
        task_id: &str,
        detail: Option<String>,
    ) -> Result<()> {
        self.update_task(paper_id, task_id, AnalysisTaskStatus::Active, detail)
            .await
    }

    pub async fn complete(&self, paper_id: &PaperId, resumable: bool) -> Result<()> {
        let mut tasklist = self.tasks_from_disk_or_job(paper_id).await?;
        for task in &mut tasklist {
            task.status = AnalysisTaskStatus::Completed;
            task.detail = None;
        }
        self.save_tasks(paper_id, tasklist).await?;
        self.update_job(paper_id, |job| {
            job.status = AnalysisJobStatus::Completed;
            job.resumable = resumable;
        })
        .await
    }

    pub async fn fail(
        &self,
        paper_id: &PaperId,
        stage: ProcessingStage,
        error: &Error,
    ) -> Result<()> {
        let mut tasklist = self.tasks_from_disk_or_job(paper_id).await?;
        if let Some(task) = tasklist
            .iter_mut()
            .find(|task| task.status == AnalysisTaskStatus::Active)
        {
            task.status = AnalysisTaskStatus::Failed;
            task.detail = Some(compact_line(&error.to_string()));
        }
        self.save_tasks(paper_id, tasklist).await?;
        self.update_job(paper_id, |job| {
            job.status = AnalysisJobStatus::Failed {
                stage,
                message: error.to_string(),
                retryable: error.retryable(),
            };
        })
        .await
    }

    pub async fn queue(&self) -> Result<ProcessingQueue> {
        let mut jobs = self.jobs.read().await.values().cloned().collect::<Vec<_>>();
        for job in &mut jobs {
            if let Some(markdown) = self.store.load_tasklist(&job.paper_id).await? {
                let mut tasks = parse_tasklist(&markdown);
                if !tasks.is_empty() {
                    for fallback in &job.tasks {
                        if !tasks.iter().any(|task| task.id == fallback.id) {
                            tasks.push(fallback.clone());
                        }
                    }
                    job.tasks = tasks;
                    job.progress = progress(&job.tasks, &job.status);
                }
            }
        }
        jobs.sort_by(|left, right| {
            let left_active = left.status.is_active();
            let right_active = right.status.is_active();
            right_active
                .cmp(&left_active)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        Ok(ProcessingQueue { jobs })
    }

    async fn update_task(
        &self,
        paper_id: &PaperId,
        task_id: &str,
        status: AnalysisTaskStatus,
        detail: Option<String>,
    ) -> Result<()> {
        let mut tasks = self.tasks_from_disk_or_job(paper_id).await?;
        let task = tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .ok_or_else(|| Error::Task(format!("analysis task `{task_id}` was not found")))?;
        task.status = status;
        task.detail = detail.map(|value| compact_line(&value));
        self.save_tasks(paper_id, tasks).await
    }

    async fn tasks_from_disk_or_job(&self, paper_id: &PaperId) -> Result<Vec<AnalysisTask>> {
        let fallback = self
            .jobs
            .read()
            .await
            .get(paper_id)
            .map(|job| job.tasks.clone())
            .ok_or_else(|| Error::Task(format!("analysis job for `{paper_id}` was not found")))?;
        if let Some(markdown) = self.store.load_tasklist(paper_id).await? {
            let mut parsed = parse_tasklist(&markdown);
            if !parsed.is_empty() {
                for task in fallback {
                    if !parsed.iter().any(|candidate| candidate.id == task.id) {
                        parsed.push(task);
                    }
                }
                return Ok(parsed);
            }
        }
        Ok(fallback)
    }

    async fn save_tasks(&self, paper_id: &PaperId, tasks: Vec<AnalysisTask>) -> Result<()> {
        let job = {
            let mut jobs = self.jobs.write().await;
            let job = jobs.get_mut(paper_id).ok_or_else(|| {
                Error::Task(format!("analysis job for `{paper_id}` was not found"))
            })?;
            job.tasks = tasks;
            job.updated_at = Utc::now();
            job.progress = progress(&job.tasks, &job.status);
            let job = job.clone();
            drop(jobs);
            job
        };
        self.store
            .save_tasklist(paper_id, &render_tasklist(&job))
            .await?;
        self.store.save_job(&job).await
    }

    async fn update_job<F>(&self, paper_id: &PaperId, update: F) -> Result<()>
    where
        F: FnOnce(&mut AnalysisJob),
    {
        let job = {
            let mut jobs = self.jobs.write().await;
            let job = jobs.get_mut(paper_id).ok_or_else(|| {
                Error::Task(format!("analysis job for `{paper_id}` was not found"))
            })?;
            update(job);
            job.updated_at = Utc::now();
            job.progress = progress(&job.tasks, &job.status);
            let job = job.clone();
            drop(jobs);
            job
        };
        self.store.save_job(&job).await
    }
}

fn tasks_for(kind: AnalysisJobKind) -> Vec<AnalysisTask> {
    let tasks = match kind {
        AnalysisJobKind::Initial => [
            ("extract", "Extract text and exact PDF page coordinates"),
            (
                "read",
                "Read the complete target paper and locate its boundaries",
            ),
            ("structure", "Map the paper's argumentative sections"),
            (
                "evidence",
                "Collect exact quotations and check their page evidence",
            ),
            (
                "explain",
                "Write the outsider digest, reading path, and Gloss",
            ),
            ("persist", "Validate and save the finished paper atlas"),
        ]
        .as_slice(),
        AnalysisJobKind::Revision => [
            (
                "context",
                "Load the source, current atlas, and prior agent context",
            ),
            (
                "feedback",
                "Interpret the reader's feedback against the current state",
            ),
            (
                "revise",
                "Revise the structural map and contextual explanations",
            ),
            (
                "evidence",
                "Re-check every changed quotation and source boundary",
            ),
            ("persist", "Validate and save the revised paper atlas"),
        ]
        .as_slice(),
    };
    tasks
        .iter()
        .map(|(id, label)| AnalysisTask {
            id: (*id).to_owned(),
            label: (*label).to_owned(),
            status: AnalysisTaskStatus::Pending,
            detail: None,
        })
        .collect()
}

#[must_use]
pub fn render_tasklist(job: &AnalysisJob) -> String {
    use std::fmt::Write;

    let kind = match job.kind {
        AnalysisJobKind::Initial => "initial analysis",
        AnalysisJobKind::Revision => "feedback revision",
    };
    let mut markdown = format!(
        "# Analysis tasklist\n\n- Paper: {}\n- Provider: `{}`\n- Run: {kind}\n\n<!-- Lysilogos watches these checkboxes. Keep each backticked task ID unchanged. -->\n",
        compact_line(&job.paper_title),
        job.provider,
    );
    for task in &job.tasks {
        let marker = match task.status {
            AnalysisTaskStatus::Pending => ' ',
            AnalysisTaskStatus::Active => '~',
            AnalysisTaskStatus::Completed => 'x',
            AnalysisTaskStatus::Failed => '!',
        };
        let _ = write!(
            markdown,
            "- [{marker}] `{}` {}",
            compact_line(&task.id),
            compact_line(&task.label)
        );
        if let Some(detail) = &task.detail {
            let _ = write!(markdown, " — {}", compact_line(detail));
        }
        markdown.push('\n');
    }
    markdown
}

#[must_use]
pub fn parse_tasklist(markdown: &str) -> Vec<AnalysisTask> {
    markdown.lines().filter_map(parse_task).take(64).collect()
}

fn parse_task(line: &str) -> Option<AnalysisTask> {
    let line = line.trim();
    let (status, rest) = [
        (AnalysisTaskStatus::Pending, "- [ ] "),
        (AnalysisTaskStatus::Active, "- [~] "),
        (AnalysisTaskStatus::Completed, "- [x] "),
        (AnalysisTaskStatus::Completed, "- [X] "),
        (AnalysisTaskStatus::Failed, "- [!] "),
    ]
    .into_iter()
    .find_map(|(status, prefix)| line.strip_prefix(prefix).map(|rest| (status, rest)))?;
    let rest = rest.strip_prefix('`')?;
    let (id, rest) = rest.split_once("` ")?;
    if id.is_empty()
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return None;
    }
    let (label, detail) = rest
        .split_once(" — ")
        .map_or((rest, None), |(label, detail)| (label, Some(detail)));
    let label = compact_line(label);
    if label.is_empty() {
        return None;
    }
    Some(AnalysisTask {
        id: id.to_owned(),
        label,
        status,
        detail: detail.map(compact_line).filter(|detail| !detail.is_empty()),
    })
}

fn progress(tasks: &[AnalysisTask], status: &AnalysisJobStatus) -> u8 {
    if matches!(status, AnalysisJobStatus::Completed) {
        return 100;
    }
    let denominator = u32::try_from(tasks.len()).unwrap_or(u32::MAX).max(1) * 100;
    let units = tasks
        .iter()
        .map(|task| match task.status {
            AnalysisTaskStatus::Pending => 0,
            AnalysisTaskStatus::Active | AnalysisTaskStatus::Failed => 50,
            AnalysisTaskStatus::Completed => 100,
        })
        .sum::<u32>();
    u8::try_from((units * 100) / denominator).unwrap_or(100)
}

fn compact_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn tasklist_round_trips_progress_and_details() {
        let now = Utc::now();
        let mut job = AnalysisJob {
            paper_id: PaperId::from_relative_path(std::path::Path::new("paper.pdf")),
            paper_title: "A paper".to_owned(),
            provider: AnalysisProvider::Codex,
            kind: AnalysisJobKind::Initial,
            status: AnalysisJobStatus::Running {
                stage: ProcessingStage::Analysis,
            },
            progress: 0,
            tasks: tasks_for(AnalysisJobKind::Initial),
            resumable: false,
            feedback: None,
            created_at: now,
            updated_at: now,
        };
        job.tasks[0].status = AnalysisTaskStatus::Completed;
        job.tasks[1].status = AnalysisTaskStatus::Active;
        job.tasks[1].detail = Some("page 4 of 12".to_owned());
        let parsed = parse_tasklist(&render_tasklist(&job));
        assert_eq!(parsed, job.tasks);
        assert_eq!(progress(&parsed, &job.status), 25);
    }

    #[tokio::test]
    async fn queue_reads_progress_edits_from_the_agent_tasklist() -> Result<()> {
        let directory = tempdir().map_err(|error| Error::io("tempdir", error))?;
        let store = ArtifactStore::new(directory.path());
        store.initialize().await?;
        let tracker = JobTracker::load(store.clone()).await?;
        let id = PaperId::from_relative_path(std::path::Path::new("paper.pdf"));
        tracker
            .begin(
                id.clone(),
                "A paper".to_owned(),
                AnalysisProvider::Codex,
                AnalysisJobKind::Initial,
                None,
            )
            .await?;
        let tasklist = store
            .load_tasklist(&id)
            .await?
            .ok_or_else(|| Error::Task("tasklist was not written".to_owned()))?
            .replace("- [ ] `extract`", "- [x] `extract`")
            .replace(
                "- [ ] `read` Read the complete target paper and locate its boundaries",
                "- [~] `read` Read the complete target paper and locate its boundaries — page 4 of 12",
            );
        store.save_tasklist(&id, &tasklist).await?;
        let queue = tracker.queue().await?;
        let job = queue
            .jobs
            .first()
            .ok_or_else(|| Error::Task("queue was empty".to_owned()))?;
        assert_eq!(job.progress, 25);
        assert_eq!(job.tasks[0].status, AnalysisTaskStatus::Completed);
        assert_eq!(job.tasks[1].status, AnalysisTaskStatus::Active);
        assert_eq!(job.tasks[1].detail.as_deref(), Some("page 4 of 12"));
        Ok(())
    }

    #[tokio::test]
    async fn active_jobs_become_retryable_failures_after_restart() -> Result<()> {
        let directory = tempdir().map_err(|error| Error::io("tempdir", error))?;
        let store = ArtifactStore::new(directory.path());
        store.initialize().await?;
        let tracker = JobTracker::load(store.clone()).await?;
        let id = PaperId::from_relative_path(std::path::Path::new("paper.pdf"));
        tracker
            .begin(
                id.clone(),
                "A paper".to_owned(),
                AnalysisProvider::Codex,
                AnalysisJobKind::Initial,
                None,
            )
            .await?;
        tracker
            .transition(&id, ProcessingStage::Analysis, "read")
            .await?;
        let restarted = JobTracker::load(store).await?;
        let queue = restarted.queue().await?;
        assert!(matches!(
            queue.jobs[0].status,
            AnalysisJobStatus::Failed {
                retryable: true,
                ..
            }
        ));
        assert_eq!(queue.jobs[0].tasks[1].status, AnalysisTaskStatus::Failed);
        Ok(())
    }
}
