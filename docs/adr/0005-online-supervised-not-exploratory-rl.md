# 0005. Online supervised learning, not exploratory RL

> **Status:** accepted (partially superseded by [0007](0007-neural-action-model.md): hard rules removed) · **Date:** 2026-11-08

## Context

The repo name suggests RL, but the real signal is an implicit label: a hotkey means "wrong, the other mode was right", and silence means "probably right".
Exploration would mean deliberately typing the wrong mode in front of the user, which directly hurts the main metric.
There are only two actions and the right label is known right after the correction window, so there is no delayed credit assignment problem.

## Decision

- Model the task as binary classification `p(VI | context)`, learned online from `Sample { features, label, weight }`.
- No exploration (epsilon = 0). Prevent flicker with a hysteresis `switch_margin`.
- Labels settle after the correction window (2 words or 10 s): no correction gives weight 0.3; a hotkey correction gives weight 1.0.
- A correction retrains on the last 20 samples in 5 steps. A replay buffer of 10 000 samples prevents forgetting.
- `Policy` (read, hot path) is separate from `Learner` (write, trainer thread).

## Consequences

- The IME never types the wrong mode on purpose; users only see errors when the model is really wrong.
- Implicit labels are biased by the old policy: replay is an estimate, and final conclusions rely on real use ([05](../05-storage-and-log.md)).
- Regions where the policy never chooses the other mode are learned only when the user corrects.
- A contextual bandit could be adopted later, but only through a new ADR.
- Note: `switch_margin` is now 0.02 (was 0.2) so mid-word display follows the model; see `crates/nimk-engine/config/default.toml`.
