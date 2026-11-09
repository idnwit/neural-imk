# 0007. Neural action model replaces hard rules

> **Status:** accepted; the edit head is partially superseded by [0012](0012-unified-boundary-commit.md) · **Date:** 2026-11-08 · **Supersedes:** part of [0005](0005-online-supervised-not-exploratory-rl.md) (hard rules)

## Context

Before this ADR the engine used hard rules (`NoTelex`, `InvalidVi`), and the composer decided by itself when to append, pop or commit. The model only returned `p(VI)` for the rest.
The new requirement: every decision comes from a neural network. We only fix the contract: events come in, go into a buffer, and the model predicts and edits the buffer or emits output.

## Decision

The model is an **action-choosing policy**. The engine executes actions mechanically.

```text
 KeyEvent --+
            v
 +---------------------+   Observation    +-----------------+
 | Engine (mechanical) | ---------------> | ImeModel (NN)   |
 |  buffer: raw keys   |                  |  edit head  (4) |
 |  telex tool: vi     | <--------------- |  render head(1) |
 +---------------------+      Action      +-----------------+
            |
            v
 EngineOutput: marked text / commit text / pass key to app
```

**Observation:** the current event and the last W events; the buffer (`raw`, Telex `vi`, `valid_vi` / `viable_vi` flags as features, not rules); the shown text and previous mode; `prev_langs`, app context, time since the last switch.

**Action** has two heads:

| Head | Class | Engine does |
| --- | --- | --- |
| edit | `Append` | add the key to `raw` |
| edit | `Pop` | drop the last key of `raw` |
| edit | `Commit` | commit the shown text, give the key to the app |
| edit | `Pass` | give the key to the app, keep the buffer |
| render | `Vi` / `En` | show/commit `vi` or `raw` |

- **Action mask** (structural only): `Append` on a non-printable key and `Pop` on an empty buffer are blocked and fall back to `Pass`. The model is never asked about the hotkey, which is a teaching channel in the I/O layer.
- **Telex is a tool, not a rule.** `nimk_telex::transform` deterministically produces the `vi` text and flags; the model decides whether to use them.
- **Learning.** The render head learns online from labels (correction weight 1.0, implicit weight small); a word's label applies to every step of that word. The edit head is pretrained on synthetic data (behavior cloning) and its online learning is off by default.
- **Manual switch mid-word** is a user command: rendering is locked to the user's choice until the word ends.
- **NN backend:** `burn` 0.22.1 `flex` for `GruAction` ([0009](0009-burn-nn-backend.md)); a hand-written `LinearAction` is the baseline. Both sit behind `ImeModel`/`ImeLearner`.
- **Hysteresis** only stabilizes the display mid-word. At `Commit`, the rendering is a fresh model decision.

## Consequences

- The engine is simpler: no `DecisionRule` in the decision path. The `rule` field of `Decision` events is kept for old logs; new logs write `null`.
- Day-one quality depends on synthetic pretraining; the IME must ship a pretrained snapshot.
- Risk: a wrong edit head breaks typing (for example a letter gets `Pass`). Mitigated by the mask, pretraining and an "edit accuracy" replay metric.
- The per-key model must keep p99 < 1 ms; the backend benchmark was an acceptance gate and passed.
- Note: [0012](0012-unified-boundary-commit.md) made edits structural (decided by the key); the model now only picks the rendering, and the edit head is unused (`edit_online_weight = 0`).

## Results

Tuning on 2026-11-08, seed 42; details in [04](../04-engine-ml.md).

| Model | render@commit (held-out) | amb EN | Online replay (synthetic log, seed 7) | `--frozen` replay |
| --- | --- | --- | --- | --- |
| `LinearAction` 200 x 500 x 2 | 0.975 | 0.944 | 97.5% | - |
| `GruAction` 200 x 500 x 2 (102 s) | 0.975 | 0.909 | 97.3% | 97.4% |

- Edit accuracy 1.000; `score` p99 0.5 us (`LinearAction`) and about 170 us (`GruAction`), well under 1 ms.
- Label leak lesson: with teacher forcing, the `shown` feature equaled the previous key's target rendering, so the model copied it (held-out render about 0.65). The `SHOWN_VI` scalar is now reserved and always 0; old snapshots need retraining. Every observation feature must be computable at inference time without target information.
- Render-only updates drift the GRU trunk, so `GruAction` learns online only from corrections (`learns_from_confirmations() = false`).
- Remaining errors are keys that are identical in both languages (`in`, `ten`, `an`, `me`); they cannot be separated without context.
