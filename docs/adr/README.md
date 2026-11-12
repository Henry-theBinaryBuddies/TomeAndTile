# Architecture decision records

One file per group of related decisions. Each records what was chosen, why, and what was rejected, so nobody (human or Claude) has to re-argue it.

- Status is one of **Accepted**, **Proposed** or **Superseded by NNNN**.
- Accepted ADRs are not edited to change the decision. Write a new ADR that supersedes the old one.
- To challenge a decision, open an issue. Do not work around it in code.

| # | Title | Status |
|---|---|---|
| [0001](0001-platform-and-language.md) | Platform and language | Accepted |
| [0002](0002-repository-and-ownership.md) | Repository, ownership and boundaries | Accepted |
| [0003](0003-contracts.md) | Schema-first Protobuf contracts | Accepted |
| [0004](0004-state-model.md) | Server-authoritative, event-sourced state | Accepted |
| [0005](0005-rules-engine.md) | 5e rules engine and automation | Accepted |
| [0006](0006-dice-and-visibility.md) | Dice, chat and hidden information | Accepted |
| [0007](0007-video-and-voice.md) | Video and voice | Proposed |
| [0008](0008-data-assets-accounts.md) | Persistence, assets, accounts and campaigns | Accepted |
| [0009](0009-extensions-and-license.md) | Extensions, content and license | Accepted |
| [0010](0010-integrations.md) | Integration with the other artifacts | Accepted |
| [0011](0011-client-ui.md) | Client UI, map and token features | Accepted |
| [0012](0012-workflow.md) | Agentic workflow, review and records | Accepted |
| [0013](0013-scope-and-schedule.md) | Class scope and schedule | Accepted |
