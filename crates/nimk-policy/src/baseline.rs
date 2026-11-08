use std::collections::BTreeMap;
use std::sync::Arc;

use nimk_types::{Features, Lang, Sample};

use crate::codec::{Reader, Writer, log_loss, weighted_mean};
use crate::{Learner, Policy, PolicyError};

const ID: &str = "baseline@v1";
const KIND: &str = "baseline";
const VERSION: u32 = 1;

/// Probability returned for the remembered (or current) language.
const CONFIDENCE: f32 = 0.9;

fn p_vi_for(lang: Lang) -> f32 {
    match lang {
        Lang::Vi => CONFIDENCE,
        Lang::En => 1.0 - CONFIDENCE,
    }
}

/// Remembers the last label per app (by `Features::app`); falls back to `Features::current`.
///
/// Samples with non-positive weight are ignored. Unlike `Linear`/`TinyMlp`,
/// an untrained baseline is intentionally NOT neutral: it predicts 0.9 for the remembered/current language and 0.1 for the other.
/// Snapshot payload: `count: u32`, then `count x (app: u32, lang: u8)` sorted
/// by app (0 = EN, 1 = VI).
#[derive(Debug, Clone, Default)]
pub struct Baseline {
    last: BTreeMap<u32, Lang>,
}

impl Baseline {
    /// Creates an untrained model.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn p_vi(&self, f: &Features) -> f32 {
        p_vi_for(match self.last.get(&f.app) {
            Some(&lang) => lang,
            None => f.current,
        })
    }
}

/// Immutable snapshot of a [`Baseline`].
#[derive(Debug)]
struct BaselinePolicy(Baseline);

impl Policy for BaselinePolicy {
    fn id(&self) -> &str {
        ID
    }

    fn predict(&self, features: &Features) -> f32 {
        self.0.p_vi(features)
    }
}

impl Learner for Baseline {
    fn id(&self) -> &str {
        ID
    }

    fn update(&mut self, batch: &[Sample]) -> f32 {
        let (mut num, mut den) = (0.0, 0.0);
        for s in batch {
            num += s.weight * log_loss(self.p_vi(&s.features), s.label.as_target());
            den += s.weight;
        }
        for s in batch.iter().filter(|s| s.weight > 0.0) {
            self.last.insert(s.features.app, s.label);
        }
        weighted_mean(num, den)
    }

    fn snapshot(&self) -> Arc<dyn Policy> {
        Arc::new(BaselinePolicy(self.clone()))
    }

    fn save(&self) -> Vec<u8> {
        let mut w = Writer::new(KIND, VERSION, 4 + self.last.len() * 5);
        w.len(self.last.len());
        for (&app, &lang) in self.last.iter().take(u32::MAX as usize) {
            w.u32(app);
            w.u8(u8::from(lang == Lang::Vi));
        }
        w.finish()
    }

    fn load(&mut self, bytes: &[u8]) -> Result<(), PolicyError> {
        let mut r = Reader::open(bytes, KIND, VERSION)?;
        let n = r.u32()?;
        let mut last = BTreeMap::new();
        for _ in 0..n {
            let app = r.u32()?;
            let lang = match r.u8()? {
                0 => Lang::En,
                1 => Lang::Vi,
                _ => return Err(PolicyError::Corrupt("bad lang byte".to_owned())),
            };
            last.insert(app, lang);
        }
        r.finish()?;
        self.last = last;
        Ok(())
    }
}
