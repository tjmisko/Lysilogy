//! Offline trace collector exercising the exact HTTP admission state machine.
use lysilogy::citation_graph::{
    Provider,
    budget::{Admission, BudgetPolicy, BudgetState},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema_version: u32,
    references: u32,
    initial_time_ms: u64,
    policy: BudgetPolicy,
    providers: Vec<Provider>,
    cooldown_every: u32,
    cooldown_seconds: u64,
    restart_every: u32,
}
#[derive(Serialize)]
struct Observation {
    provider: Provider,
    reference: u32,
    at_ms: u64,
    event: &'static str,
    until_ms: Option<u64>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "eval/truth/provider-budget-batch.json".to_owned());
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(path)?)?;
    if fixture.schema_version != 1
        || fixture.references != 10_000
        || fixture.providers != Provider::ALL
        || fixture.cooldown_every == 0
        || fixture.restart_every == 0
    {
        return Err(std::io::Error::other("Expected a 10k-reference fixture with all four providers and nonzero cooldown/restart periods").into());
    }
    let policy = fixture
        .policy
        .validate()
        .map_err(|failure| std::io::Error::other(failure.message))?;
    let mut states: [BudgetState; 4] = std::array::from_fn(|_| BudgetState::default());
    let mut times = [fixture.initial_time_ms; 4];
    let mut counts = [0_u32; 4];
    let mut events = Vec::new();
    for reference in 0..fixture.references {
        let provider = fixture.providers[usize::try_from(reference)? % fixture.providers.len()];
        let index = provider.index();
        loop {
            let admission = states[index].reserve(times[index], policy);
            let (event, until_ms) = match admission {
                Admission::Granted => ("granted", None),
                Admission::Wait { until_ms } => ("wait", Some(until_ms)),
                Admission::Deferred { until_ms } => ("deferred", Some(until_ms)),
            };
            events.push(Observation {
                provider,
                reference,
                at_ms: times[index],
                event,
                until_ms,
            });
            if let Some(until_ms) = until_ms {
                times[index] = until_ms;
            } else {
                break;
            }
        }
        counts[index] += 1;
        if counts[index].is_multiple_of(fixture.cooldown_every) {
            states[index].cooldown(times[index], fixture.cooldown_seconds);
            events.push(Observation {
                provider,
                reference,
                at_ms: times[index],
                event: "cooldown",
                until_ms: Some(states[index].cooldown_until_ms),
            });
        }
        if counts[index].is_multiple_of(fixture.restart_every) {
            states[index] = serde_json::from_slice(&serde_json::to_vec(&states[index])?)?;
            events.push(Observation {
                provider,
                reference,
                at_ms: times[index],
                event: "restart",
                until_ms: None,
            });
        }
    }
    serde_json::to_writer(std::io::stdout().lock(), &events)?;
    Ok(())
}
