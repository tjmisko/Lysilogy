//! A recording planner for tests: it answers from a script, never calls a model, and keeps
//! every request so a test can check what the pipeline offered.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::{
    FillDraft, FillRequest, PlanDraft, PlanRequest, PlannerError, PlannerFuture, PlannerReply,
    ReadingListPlanner,
};

type FillScript = dyn Fn(&FillRequest) -> FillDraft + Send + Sync;

#[derive(Default)]
struct Recorded {
    plans: Vec<PlanRequest>,
    fills: Vec<FillRequest>,
}

/// A scripted [`ReadingListPlanner`].
#[derive(Clone)]
pub struct FakePlanner {
    plan: Result<PlanDraft, PlannerError>,
    fill: Arc<FillScript>,
    plan_cost: f64,
    fill_cost: f64,
    delay: Duration,
    panics: bool,
    recorded: Arc<Mutex<Recorded>>,
}

impl std::fmt::Debug for FakePlanner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FakePlanner")
    }
}

impl FakePlanner {
    /// A planner whose plan pass answers `plan` and whose fill pass picks nothing.
    #[must_use]
    pub fn new(plan: PlanDraft) -> Self {
        Self {
            plan: Ok(plan),
            fill: Arc::new(|_| FillDraft::default()),
            plan_cost: 0.0,
            fill_cost: 0.0,
            delay: Duration::ZERO,
            panics: false,
            recorded: Arc::default(),
        }
    }

    /// A planner whose plan pass fails with `error`.
    #[must_use]
    pub fn failing(error: PlannerError) -> Self {
        Self {
            plan: Err(error),
            ..Self::new(PlanDraft::default())
        }
    }

    /// The fill pass answers `draft`.
    #[must_use]
    pub fn with_fill(self, draft: FillDraft) -> Self {
        self.with_fill_fn(move |_| draft.clone())
    }

    /// The fill pass answers from the request it receives.
    #[must_use]
    pub fn with_fill_fn(
        mut self,
        script: impl Fn(&FillRequest) -> FillDraft + Send + Sync + 'static,
    ) -> Self {
        self.fill = Arc::new(script);
        self
    }

    /// What each pass costs, in US dollars.
    #[must_use]
    pub const fn with_costs(mut self, plan: f64, fill: f64) -> Self {
        self.plan_cost = plan;
        self.fill_cost = fill;
        self
    }

    /// The plan pass panics, as a bug in a real planner would.
    #[must_use]
    pub const fn panicking(mut self) -> Self {
        self.panics = true;
        self
    }

    /// The plan pass takes this long to answer.
    #[must_use]
    pub const fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    fn recorded(&self) -> MutexGuard<'_, Recorded> {
        self.recorded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Every plan request received.
    #[must_use]
    pub fn plan_requests(&self) -> Vec<PlanRequest> {
        self.recorded().plans.clone()
    }

    /// Every fill request received.
    #[must_use]
    pub fn fill_requests(&self) -> Vec<FillRequest> {
        self.recorded().fills.clone()
    }
}

impl ReadingListPlanner for FakePlanner {
    fn model_key(&self) -> &'static str {
        "fake:planner"
    }

    fn funding(&self) -> &'static str {
        "plan"
    }

    fn plan<'a>(&'a self, request: &'a PlanRequest) -> PlannerFuture<'a, PlanDraft> {
        Box::pin(async move {
            self.recorded().plans.push(request.clone());
            tokio::time::sleep(self.delay).await;
            assert!(!self.panics, "the fake planner was told to panic");
            self.plan.clone().map(|draft| PlannerReply {
                draft,
                cost_usd: self.plan_cost,
            })
        })
    }

    fn fill<'a>(&'a self, request: &'a FillRequest) -> PlannerFuture<'a, FillDraft> {
        Box::pin(async move {
            self.recorded().fills.push(request.clone());
            Ok(PlannerReply {
                draft: (self.fill)(request),
                cost_usd: self.fill_cost,
            })
        })
    }
}
