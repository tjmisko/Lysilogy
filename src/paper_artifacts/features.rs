//! Which optional feature passes run after the core pass.
//!
//! The core fields (identity, abstract, context, sections, references,
//! citations, relations) are always extracted. Everything here is off by
//! default and can be requested on its own, so an existing core result is
//! never invalidated by adding a feature.

use crate::{Error, Result};

/// One optional post-processing feature.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Feature {
    Figures,
    Tables,
    Algorithms,
    Equations,
    Supercut,
    Analysis,
    Glossary,
}

impl Feature {
    const fn object_kind(self) -> Option<&'static str> {
        match self {
            Self::Figures => Some("figure"),
            Self::Tables => Some("table"),
            Self::Algorithms => Some("algorithm"),
            Self::Equations => Some("equation"),
            Self::Supercut | Self::Analysis | Self::Glossary => None,
        }
    }

    fn parse_one(name: &str) -> Result<Vec<Self>> {
        Ok(match name {
            "figures" => vec![Self::Figures],
            "tables" => vec![Self::Tables],
            "algorithms" => vec![Self::Algorithms],
            "equations" => vec![Self::Equations],
            "objects" => vec![
                Self::Figures,
                Self::Tables,
                Self::Algorithms,
                Self::Equations,
            ],
            "supercut" => vec![Self::Supercut],
            "analysis" => vec![Self::Analysis],
            "glossary" => vec![Self::Glossary],
            other => {
                return Err(Error::InvalidRequest(format!(
                    "unknown feature '{other}'; expected figures, tables, algorithms, equations, objects, supercut, analysis, glossary, all, or none"
                )));
            }
        })
    }
}

/// The optional features an extraction asks for.
// Clippy prefers an enum, but the toggles are genuinely independent flags.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Features {
    pub figures: bool,
    pub tables: bool,
    pub algorithms: bool,
    pub equations: bool,
    pub supercut: bool,
    pub analysis: bool,
    pub glossary: bool,
}

impl Features {
    /// Parse a comma-separated list. `all` turns everything on, `none` and an
    /// empty list turn everything off.
    ///
    /// # Errors
    /// Fails on a name that is not a known feature.
    pub fn parse(value: &str) -> Result<Self> {
        let mut features = Self::default();
        for name in value
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            if name == "all" {
                return Ok(Self::all());
            }
            if name == "none" {
                continue;
            }
            for feature in Feature::parse_one(name)? {
                features.enable(feature);
            }
        }
        Ok(features)
    }

    #[must_use]
    pub const fn all() -> Self {
        Self {
            figures: true,
            tables: true,
            algorithms: true,
            equations: true,
            supercut: true,
            analysis: true,
            glossary: true,
        }
    }

    const fn enable(&mut self, feature: Feature) {
        match feature {
            Feature::Figures => self.figures = true,
            Feature::Tables => self.tables = true,
            Feature::Algorithms => self.algorithms = true,
            Feature::Equations => self.equations = true,
            Feature::Supercut => self.supercut = true,
            Feature::Analysis => self.analysis = true,
            Feature::Glossary => self.glossary = true,
        }
    }

    #[must_use]
    pub const fn any_objects(&self) -> bool {
        self.figures || self.tables || self.algorithms || self.equations
    }

    /// The object kinds to request, e.g. `["figure", "equations"]`.
    #[must_use]
    pub fn object_kinds(&self) -> Vec<&'static str> {
        [
            (self.figures, Feature::Figures),
            (self.tables, Feature::Tables),
            (self.algorithms, Feature::Algorithms),
            (self.equations, Feature::Equations),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .filter_map(|(_, feature)| feature.object_kind())
        .collect()
    }

    /// The enabled feature names, in a stable order, for the record.
    #[must_use]
    pub fn label(&self) -> String {
        let mut names = Vec::new();
        for (on, name) in [
            (self.figures, "figures"),
            (self.tables, "tables"),
            (self.algorithms, "algorithms"),
            (self.equations, "equations"),
            (self.supercut, "supercut"),
            (self.analysis, "analysis"),
            (self.glossary, "glossary"),
        ] {
            if on {
                names.push(name);
            }
        }
        names.join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_turn_everything_on_for_all_and_off_for_none() {
        assert_eq!(Features::parse("").expect("empty"), Features::default());
        assert_eq!(Features::parse("none").expect("none"), Features::default());
        assert_eq!(Features::parse("all").expect("all"), Features::all());
    }

    #[test]
    fn should_expand_objects_into_the_four_kinds() {
        let features = Features::parse("objects").expect("objects");
        assert_eq!(
            features.object_kinds(),
            ["figure", "table", "algorithm", "equation"]
        );
        assert!(features.any_objects());
    }

    #[test]
    fn should_keep_features_independent_in_order() {
        let features = Features::parse("equations, supercut").expect("parse");
        assert_eq!(features.object_kinds(), ["equation"]);
        assert!(features.supercut && features.equations);
        assert!(!features.figures && !features.analysis);
        assert_eq!(features.label(), "equations,supercut");
    }

    #[test]
    fn should_reject_an_unknown_feature() {
        assert!(Features::parse("objects,telepathy").is_err());
    }
}
