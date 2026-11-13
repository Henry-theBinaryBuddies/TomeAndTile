# contracts/fixtures

Recorded sample data, written as Protobuf JSON of contract messages (camelCase field names, enums as strings). The mock server (`crates/mock-server`) and tests load them.

- `mock/snapshot.json`: one `Snapshot` (a campaign with one scene, two tokens and two members) that the mock server starts from.

Rule: every fixture must always parse with the generated types. A test in the consuming crate checks this; update the fixture in the same PR as any contract change that breaks it. Fixtures use invented content only.
