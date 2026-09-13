//! The plan's complete scorecard. Components retain their own units and direction.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Gate,
    Objective,
    Reported,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Higher,
    Lower,
}

#[derive(Clone, Copy, Debug)]
pub struct Definition {
    pub id: &'static str,
    pub suite: &'static str,
    pub name: &'static str,
    pub truth: &'static str,
    pub kind: Kind,
    pub direction: Direction,
    pub target: Option<f64>,
    pub unit: &'static str,
    /// Absolute for proportions/counts, relative for latency; no gate tolerance.
    pub tolerance: f64,
    pub relative_tolerance: bool,
}

impl Definition {
    #[must_use]
    pub fn at_target(self, value: f64) -> bool {
        self.target.is_none_or(|target| match self.direction {
            Direction::Higher => value >= target,
            Direction::Lower => value <= target,
        })
    }

    #[must_use]
    pub fn improves(self, value: f64, baseline: f64) -> bool {
        match self.direction {
            Direction::Higher => value > baseline,
            Direction::Lower => value < baseline,
        }
    }

    #[must_use]
    pub fn regresses(self, value: f64, baseline: f64) -> bool {
        let margin = if self.relative_tolerance {
            baseline.abs() * self.tolerance
        } else {
            self.tolerance
        };
        let loss = match self.direction {
            Direction::Higher => baseline - value,
            Direction::Lower => value - baseline,
        };
        loss > (f64::EPSILON * 8.0).mul_add(baseline.abs().max(1.0), margin)
    }

    #[must_use]
    pub fn valid_value(self, value: f64) -> bool {
        value.is_finite()
            && value >= 0.0
            && match self.unit {
                "fraction" | "pass" => value <= 1.0,
                "rating / 5" => (1.0..=5.0).contains(&value),
                _ => true,
            }
    }
}

#[must_use]
#[allow(clippy::too_many_lines)] // One auditable row per scorecard component.
pub fn definitions() -> Vec<Definition> {
    use Direction::{Higher as H, Lower as L};
    use Kind::{Gate as G, Objective as O, Reported as R};
    let rows = [
        (
            "G1.works",
            "resolution",
            "Auto-merge precision: Works",
            "K3",
            G,
            H,
            Some(0.99),
            "fraction",
        ),
        (
            "G1.persons",
            "persons",
            "Auto-merge precision: Persons",
            "K3,K4",
            G,
            H,
            Some(0.99),
            "fraction",
        ),
        (
            "G2",
            "acquisition",
            "Wrong paper linked after download",
            "K5",
            G,
            L,
            Some(0.0),
            "count",
        ),
        (
            "G3",
            "objects",
            "Published enrichment quote source mismatches",
            "enrichment outputs",
            G,
            L,
            Some(0.0),
            "count",
        ),
        (
            "G4",
            "resolution",
            "Rebuild determinism: entities, IDs, aliases",
            "K2",
            G,
            H,
            Some(1.0),
            "fraction",
        ),
        (
            "G5",
            "tests",
            "Tests pass with network disabled and no model CLIs",
            "isolated Rust and web tests",
            G,
            H,
            Some(1.0),
            "pass",
        ),
        (
            "O1",
            "objects",
            "Figure and table detection F1",
            "K1",
            O,
            H,
            Some(0.90),
            "fraction",
        ),
        (
            "O2",
            "objects",
            "Figure and table region IoU, median",
            "K1",
            O,
            H,
            Some(0.75),
            "fraction",
        ),
        (
            "O3",
            "objects",
            "Numbered equation detection F1",
            "K1",
            O,
            H,
            Some(0.85),
            "fraction",
        ),
        (
            "O4",
            "objects",
            "Equation and statement mention link accuracy",
            "K1",
            O,
            H,
            Some(0.90),
            "fraction",
        ),
        (
            "O5",
            "objects",
            "Theorem-like statement detection F1",
            "K1",
            O,
            H,
            Some(0.85),
            "fraction",
        ),
        (
            "O6",
            "objects",
            "Proof to statement link accuracy",
            "K1",
            O,
            H,
            Some(0.85),
            "fraction",
        ),
        (
            "O7",
            "objects",
            "Algorithm detection F1",
            "K1",
            O,
            H,
            Some(0.80),
            "fraction",
        ),
        (
            "O8",
            "bibliography",
            "Bibliography entry segmentation F1",
            "K1",
            O,
            H,
            Some(0.95),
            "fraction",
        ),
        (
            "O9.title",
            "bibliography",
            "Bibliography title accuracy",
            "K1,K2",
            O,
            H,
            Some(0.90),
            "fraction",
        ),
        (
            "O9.first_author",
            "bibliography",
            "Bibliography first author accuracy",
            "K1,K2",
            O,
            H,
            Some(0.90),
            "fraction",
        ),
        (
            "O9.year",
            "bibliography",
            "Bibliography year accuracy",
            "K1,K2",
            O,
            H,
            Some(0.95),
            "fraction",
        ),
        (
            "O10.precision",
            "bibliography",
            "Citation marker to entry precision",
            "K1",
            O,
            H,
            Some(0.97),
            "fraction",
        ),
        (
            "O10.recall",
            "bibliography",
            "Citation marker to entry recall",
            "K1",
            O,
            H,
            Some(0.85),
            "fraction",
        ),
        (
            "O11",
            "objects",
            "Key-figure top-3 panel agreement",
            "K1",
            O,
            H,
            Some(0.70),
            "fraction",
        ),
        (
            "O12.precision",
            "resolution",
            "Reference to identifier precision",
            "K2",
            O,
            H,
            Some(0.98),
            "fraction",
        ),
        (
            "O12.recall",
            "resolution",
            "Reference to identifier recall",
            "K2",
            O,
            H,
            Some(0.80),
            "fraction",
        ),
        (
            "O13",
            "persons",
            "Person clustering B-cubed F1",
            "K4",
            O,
            H,
            Some(0.90),
            "fraction",
        ),
        (
            "O14",
            "resolution",
            "Auto-merge recall",
            "K3",
            O,
            H,
            Some(0.80),
            "fraction",
        ),
        (
            "O15",
            "resolution",
            "Duplicate Works remaining after ingest",
            "K2",
            O,
            L,
            Some(0.02),
            "fraction",
        ),
        (
            "O16",
            "acquisition",
            "References reaching identifier_found",
            "K5",
            O,
            H,
            Some(0.85),
            "fraction",
        ),
        (
            "O17",
            "acquisition",
            "OA references reaching downloaded",
            "K5",
            O,
            H,
            Some(0.80),
            "fraction",
        ),
        (
            "O18",
            "acquisition",
            "Downloads mapped under extract_heuristic",
            "K5",
            O,
            H,
            Some(0.95),
            "fraction",
        ),
        (
            "O19",
            "acquisition",
            "Model calls per acquired reference",
            "K5",
            O,
            L,
            Some(0.30),
            "calls / reference",
        ),
        (
            "O20",
            "citations",
            "Styled citation normalized CSL match",
            "K2",
            O,
            H,
            Some(0.90),
            "fraction",
        ),
        (
            "O21.bibtex",
            "citations",
            "BibTeX exports parsing without errors",
            "K2",
            O,
            H,
            Some(1.0),
            "fraction",
        ),
        (
            "O21.ris",
            "citations",
            "RIS exports parsing without errors",
            "K2",
            O,
            H,
            Some(1.0),
            "fraction",
        ),
        (
            "O22.resolved",
            "lists",
            "AI list proposals resolved",
            "K6",
            O,
            H,
            Some(0.90),
            "fraction",
        ),
        (
            "O22.fabricated",
            "lists",
            "AI list fabricated works",
            "K6",
            O,
            L,
            Some(0.05),
            "fraction",
        ),
        (
            "O23",
            "lists",
            "AI list relevance panel mean",
            "K6",
            O,
            H,
            Some(4.0),
            "rating / 5",
        ),
        (
            "O24",
            "read-next",
            "Read-next recall at 10",
            "K7",
            O,
            H,
            Some(0.30),
            "fraction",
        ),
        (
            "O25",
            "scale",
            "No-change rescan at 10k papers",
            "synthetic,K0",
            O,
            L,
            Some(2.0),
            "s",
        ),
        (
            "O26.render",
            "scale",
            "Home first render at 10k papers",
            "synthetic,K0",
            O,
            L,
            Some(500.0),
            "ms",
        ),
        (
            "O26.search",
            "scale",
            "Search p95 at 10k papers",
            "synthetic,K0",
            O,
            L,
            Some(150.0),
            "ms",
        ),
        (
            "O27",
            "scale",
            "Extraction parallel efficiency at four workers",
            "synthetic,K0",
            O,
            H,
            Some(0.70),
            "fraction",
        ),
        (
            "O28",
            "scale",
            "Two-hop query p95 at 500k Works and 3M edges",
            "synthetic",
            O,
            L,
            Some(150.0),
            "ms",
        ),
        (
            "O29",
            "scale",
            "Graph frame rate at 500 nodes",
            "synthetic",
            O,
            H,
            Some(30.0),
            "fps",
        ),
        (
            "O30",
            "scale",
            "Provider budget violations at 10k references",
            "recorded fixtures",
            O,
            L,
            Some(0.0),
            "count",
        ),
        (
            "R1",
            "acquisition",
            "Agent-fallback lift over deterministic acquisition",
            "K5",
            R,
            H,
            None,
            "fraction",
        ),
        (
            "R2",
            "lists",
            "AI list overlap with survey bibliography",
            "K6",
            R,
            H,
            None,
            "fraction",
        ),
        (
            "R3",
            "resolution",
            "Gold-set inter-labeler agreement",
            "K3",
            R,
            H,
            None,
            "fraction",
        ),
        (
            "R4.enrichment_cost",
            "objects",
            "Enrichment cost per paper",
            "enrichment outputs",
            R,
            L,
            None,
            "USD / paper",
        ),
        (
            "R4.enrichment_time",
            "objects",
            "Enrichment wall time per paper",
            "enrichment outputs",
            R,
            L,
            None,
            "s",
        ),
        (
            "R4.acquisition_cost",
            "acquisition",
            "Acquisition cost per reference",
            "K5",
            R,
            L,
            None,
            "USD / reference",
        ),
        (
            "R4.acquisition_time",
            "acquisition",
            "Acquisition wall time per reference",
            "K5",
            R,
            L,
            None,
            "s",
        ),
        (
            "R4.list_cost",
            "lists",
            "Generation cost per AI list",
            "K6",
            R,
            L,
            None,
            "USD / list",
        ),
        (
            "R4.list_time",
            "lists",
            "Generation wall time per AI list",
            "K6",
            R,
            L,
            None,
            "s",
        ),
    ];
    rows.into_iter()
        .map(|(id, suite, name, truth, kind, direction, target, unit)| {
            let relative_tolerance = matches!(unit, "s" | "ms");
            let tolerance = if kind != Kind::Objective {
                0.0
            } else if relative_tolerance {
                0.1
            } else if unit == "fraction" {
                0.01
            } else {
                0.0
            };
            Definition {
                id,
                suite,
                name,
                truth,
                kind,
                direction,
                target,
                unit,
                tolerance,
                relative_tolerance,
            }
        })
        .collect()
}
