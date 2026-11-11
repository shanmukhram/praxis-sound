//! The draft of one assistant turn: the sound as it was when the request came in, the score the
//! assistant is changing, the patches that got it there, and the rules every change must keep
//! (locks, enforced by `apply`; the request's scope, enforced here).
//!
//! Tools that only change the draft (`apply_patch`, `set_params`, `compose_edit`,
//! `compose_sound`) touch no file. When the turn ends, [`Draft::commit`] makes the draft one new
//! take through the same `make_take` as `ask`, so the take history stays shared. Tools that act on
//! the folder at once (`keep_variation`, `restore_take`, `undo_redo`) need a clean draft and
//! [`Draft::reload`] afterwards.

use super::measure::LayerMeasure;
use crate::error::{ErrorCode, OpError};
use crate::ops::changes::{bars_label, changed_layers};
use crate::ops::sound::{Sound, SoundLock, TakeRecord, TakeScope, ViewName};
use crate::ops::takes::{Words, make_take, project_error, render_settings};
use praxis_score::Score;
use praxis_score::apply::apply;
use praxis_score::patch::{Origin, PatchError, Scope, ScorePatch};
use praxis_score::project::{Project, patch_between};
use serde_json::Value;
use std::path::PathBuf;

/// A new sound composed in this turn (the sound had no takes).
#[derive(Clone, Debug)]
pub(crate) struct NewSound {
    pub summary: String,
    pub repairs: Value,
}

#[derive(Debug)]
pub(crate) struct Draft {
    pub dir: PathBuf,
    pub skip_missing: bool,
    pub scope: Option<TakeScope>,
    /// Seed for plans and edits composed in this turn.
    pub seed: u64,
    /// `takes.json` revision and current take the draft starts from.
    pub base_revision: u64,
    pub base_take: u32,
    /// The live score the draft starts from (`None`: the sound has no takes yet).
    pub base: Option<Score>,
    /// The score being changed.
    pub score: Option<Score>,
    /// Applied to `base` in order, they give `score`.
    pub patches: Vec<ScorePatch>,
    pub new_sound: Option<NewSound>,
    /// The take this turn already made (`keep_variation`); then nothing else may change.
    pub take_made: Option<u32>,
    /// `ask_user`'s question: the turn ends without a take.
    pub question: Option<String>,
    /// A view to open once the take exists.
    pub opens: Option<ViewName>,
    /// The base score's per-layer measurements (rendered once, on the first `render_check`).
    pub base_measure: Option<Vec<LayerMeasure>>,
}

impl Draft {
    pub fn open(
        dir: PathBuf,
        scope: Option<TakeScope>,
        seed: u64,
        skip_missing: bool,
    ) -> Result<Draft, OpError> {
        let mut d = Draft {
            dir,
            skip_missing,
            scope,
            seed,
            base_revision: 0,
            base_take: 0,
            base: None,
            score: None,
            patches: Vec::new(),
            new_sound: None,
            take_made: None,
            question: None,
            opens: None,
            base_measure: None,
        };
        d.reload()?;
        Ok(d)
    }

    /// Starts again from the folder as it is now (drops any change).
    pub fn reload(&mut self) -> Result<(), OpError> {
        let sound = Sound::open(&self.dir)?;
        self.base_revision = sound.takes.revision;
        self.base_take = sound.takes.current;
        self.base = sound.project.map(|p| p.score);
        self.score = self.base.clone();
        self.patches.clear();
        self.new_sound = None;
        self.base_measure = None;
        Ok(())
    }

    /// True when the draft differs from the folder.
    pub fn dirty(&self) -> bool {
        !self.patches.is_empty() || self.new_sound.is_some()
    }

    /// Why nothing more may change in this turn, if so.
    pub fn frozen(&self) -> Option<String> {
        self.take_made.map(|n| {
            format!(
                "this request already made take {n}; one request makes one take. Reply to the \
                 person now and leave further changes for their next request"
            )
        })
    }

    /// The request's scope, when this change would leave it.
    pub fn check_scope(&self, after: &Score) -> Result<(), String> {
        let (Some(scope), Some(base)) = (&self.scope, &self.base) else {
            return Ok(());
        };
        let b = scope.bars;
        for (layer, r) in changed_layers(base, after) {
            let existing = base.layer(&layer).is_some();
            if (existing && layer != scope.layer) || r[0] < b.start || r[1] > b.end {
                return Err(format!(
                    "this would change {} of layer `{layer}`, outside the request's scope ({}). \
                     Change only bars {}-{} of `{}`: notes in those bars (write_bars, clear_bars, \
                     compose_edit with those bars), or a new layer that plays only there. A \
                     change to a layer's fixed settings (instrument parameters, gain, pan, an \
                     existing effect's parameters) counts as a change to every bar it plays. To \
                     hold a change of sound to the bars, automate it in an unscoped \
                     update_layer: an automation lane that keeps the old value outside the bars \
                     (a `step` curve with points at the first bar and at the bar after the \
                     last), or a new effect whose `mix` is 0 outside the bars (and 0 as its \
                     fixed value). If that does not fit, say so to the person and offer to \
                     apply the change to the whole layer",
                    bars_label(r),
                    scope_label(base, scope),
                    b.start,
                    b.end,
                    scope.layer
                ));
            }
        }
        Ok(())
    }

    /// Applies `patch` to the draft (as the model: locked layers stay locked) and keeps it when
    /// the result stays inside the scope. Returns the new draft score.
    pub fn apply(&mut self, mut patch: ScorePatch) -> Result<&Score, String> {
        if let Some(why) = self.frozen() {
            return Err(why);
        }
        let Some(score) = &self.score else {
            return Err(
                "the sound has no score yet: compose one with compose_sound first".to_string(),
            );
        };
        patch.origin = Origin::Model;
        let applied = apply(score, &patch).map_err(|e| explain(&e, &patch))?;
        self.check_scope(&applied.score)?;
        if self.new_sound.is_some() {
            // A new sound's take is its whole score; later changes just refine it.
            self.score = Some(applied.score);
        } else {
            self.patches.push(patch);
            self.score = Some(applied.score);
        }
        Ok(self.score.as_ref().expect("set above"))
    }

    /// Replaces the draft with a whole new score (`compose_sound`).
    pub fn replace(&mut self, score: Score, new: NewSound) -> Result<&Score, String> {
        if let Some(why) = self.frozen() {
            return Err(why);
        }
        match &self.base {
            None => {
                self.patches.clear();
                self.new_sound = Some(new);
                self.score = Some(score);
                Ok(self.score.as_ref().expect("set above"))
            }
            Some(_) => {
                let current = self.score.clone().expect("a base means a score");
                let patch = patch_between(&current, &score, format!("new sound: {}", score.title))?;
                self.apply(patch)
            }
        }
    }

    /// The draft's changes as one patch on `base`.
    pub fn merged_patch(&self) -> ScorePatch {
        merge(&self.patches)
    }

    /// Makes the draft the sound's next take. `Ok(None)` when the draft is clean.
    pub fn commit(&mut self, words: Words) -> Result<Option<(TakeRecord, Value)>, OpError> {
        if !self.dirty() {
            return Ok(None);
        }
        let _lock = SoundLock::acquire(&self.dir)?;
        let mut sound = Sound::open(&self.dir)?;
        let settings = render_settings(self.skip_missing);
        let score = self.score.clone().expect("a dirty draft has a score");
        if let Some(new) = &self.new_sound {
            if sound.project.is_some() || !sound.takes.takes.is_empty() {
                return Err(OpError::new(
                    ErrorCode::PatchRejected,
                    "the sound got its first take from someone else while I was composing; \
                     nothing was changed",
                ));
            }
            sound.project = Some(Project::new(score));
            let take = make_take(&mut sound, None, None, words, &settings)?;
            let mut payload = crate::ops::takes::take_payload(&sound, &take, false);
            payload["repairs"] = new.repairs.clone();
            self.take_made = Some(take.n);
            return Ok(Some((take, payload)));
        }
        let base = self
            .base
            .clone()
            .expect("a dirty draft without a new sound has a base");
        let project = sound
            .project
            .as_mut()
            .ok_or_else(|| OpError::new(ErrorCode::Error, "the sound's project disappeared"))?;
        let before = project.score.clone();
        let mut patch = self.merged_patch();
        project.commit(patch.clone()).map_err(|e| {
            OpError::new(
                ErrorCode::PatchRejected,
                if before == base {
                    format!("the change could not be committed: {e}")
                } else {
                    format!(
                        "the sound changed while I was working and my change no longer applies \
                         ({e}); nothing was changed"
                    )
                },
            )
        })?;
        if before == base && project.score != score {
            // The merged ops must give the draft exactly; if they ever do not, commit the exact
            // difference instead.
            project.undo().map_err(project_error)?;
            project.redo.clear();
            patch =
                patch_between(&before, &score, patch.description.clone()).map_err(project_error)?;
            patch.origin = Origin::Model;
            project.commit(patch.clone()).map_err(project_error)?;
        }
        let take = make_take(&mut sound, Some(&before), Some(patch), words, &settings)?;
        let payload = crate::ops::takes::take_payload(&sound, &take, false);
        self.take_made = Some(take.n);
        Ok(Some((take, payload)))
    }
}

/// `"bars 13–16, Felt piano"`.
pub(crate) fn scope_label(score: &Score, scope: &TakeScope) -> String {
    format!(
        "{}, {}",
        bars_label([scope.bars.start, scope.bars.end]),
        super::words::layer_name(Some(score), &scope.layer)
    )
}

/// `apply`'s refusal in words the model can act on. Under `scope.bars`, `apply` refuses every op
/// that changes a whole layer (an `update_layer` other than a rename: instrument, gain, pan,
/// effects, automation), so the fix is a patch without `scope.bars`.
pub(crate) fn explain(e: &PatchError, patch: &ScorePatch) -> String {
    let mut text = e.to_string();
    if let PatchError::OutOfScope { .. } = e
        && patch.scope.bars.is_some()
    {
        text.push_str(
            ". A patch with `scope.bars` may only change notes in those bars (write_bars, \
             clear_bars, place_clip) or add a layer that plays only there. An update_layer that \
             changes the instrument, gain, pan, effects or automation changes the whole layer: \
             send it in a patch without `scope.bars` (leave `scope` out). To make such a change \
             audible only in some bars, put automation in that unscoped update_layer that keeps \
             the old value outside the bars (e.g. `instrument.tone` on a `step` curve, or a new \
             reverb whose `mix` is 0 outside the bars); the request's scope counts it bar by bar",
        );
    }
    if let PatchError::LockedLayer(id) = e {
        text.push_str(&format!(
            ". Only the person can unlock `{id}`: tell them it is locked and change nothing on it"
        ));
    }
    text
}

/// One patch with every op of `patches`, in order, under a scope that covers all of theirs.
pub(crate) fn merge(patches: &[ScorePatch]) -> ScorePatch {
    if let [one] = patches {
        let mut p = one.clone();
        p.origin = Origin::Model;
        return p;
    }
    let unscoped_bars = patches.iter().any(|p| p.scope.bars.is_none());
    let any_layer = patches.iter().any(|p| p.scope.layers.is_empty());
    let bars = if unscoped_bars {
        None
    } else {
        let lo = patches
            .iter()
            .filter_map(|p| p.scope.bars)
            .map(|b| b.start)
            .min();
        let hi = patches
            .iter()
            .filter_map(|p| p.scope.bars)
            .map(|b| b.end)
            .max();
        lo.zip(hi)
            .map(|(start, end)| praxis_score::BarRange { start, end })
    };
    let mut layers: Vec<String> = Vec::new();
    if !any_layer {
        for p in patches {
            for l in &p.scope.layers {
                if !layers.contains(l) {
                    layers.push(l.clone());
                }
            }
        }
    }
    ScorePatch {
        description: patches
            .iter()
            .map(|p| p.description.trim())
            .filter(|d| !d.is_empty())
            .collect::<Vec<_>>()
            .join("; "),
        origin: Origin::Model,
        scope: Scope { bars, layers },
        ops: patches.iter().flat_map(|p| p.ops.clone()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::BarRange;
    use praxis_score::patch::PatchOp;

    fn p(bars: Option<(u32, u32)>, layers: &[&str], desc: &str) -> ScorePatch {
        ScorePatch {
            description: desc.into(),
            origin: Origin::User,
            scope: Scope {
                bars: bars.map(|(start, end)| BarRange { start, end }),
                layers: layers.iter().map(|l| l.to_string()).collect(),
            },
            ops: vec![PatchOp::SetTitle { title: desc.into() }],
        }
    }

    #[test]
    fn merged_scopes_cover_every_patch() {
        let m = merge(&[
            p(Some((13, 16)), &["piano"], "a"),
            p(Some((9, 12)), &["kick"], "b"),
        ]);
        assert_eq!(m.scope.bars, Some(BarRange { start: 9, end: 16 }));
        assert_eq!(m.scope.layers, ["piano", "kick"]);
        assert_eq!(m.ops.len(), 2);
        assert_eq!(m.description, "a; b");
        assert_eq!(m.origin, Origin::Model);
        let m = merge(&[p(Some((13, 16)), &["piano"], "a"), p(None, &[], "b")]);
        assert_eq!(m.scope, Scope::default());
        let one = merge(&[p(None, &[], "x")]);
        assert_eq!(one.origin, Origin::Model);
    }
}
