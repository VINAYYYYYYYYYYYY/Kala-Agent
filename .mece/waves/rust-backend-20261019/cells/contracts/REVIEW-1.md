# GPT-6.1-Sol initial contracts review — repairs required before acceptance

Independently ran all38 locked offline release tests including434live Python differential cases, all passed. Read actual ContractModel/WireValue, typed models, URI and construction profile code. No missing Python contract model class found. Current T3 task still reports running despite the ready report; do not launch a duplicate repair until its terminal result is consumed and no live turns remain.

The typed/canonical foundation is useful. Two documented behavior changes are not accepted as silent hardening/determinism changes for this migration. Both reproduced with an independent scratch Rust binary in /tmp consuming the actual crate through a read-only path dependency, and live Python Pydantic on the same host:

## P2 — ordinary model JSON must preserve Python duplicate-key behavior

`Requirement` JSON `{"id":"req-a","id":"req-b","title":"probe","kind":"functional"}`: Python model_validate_json accepts and gives id=req-b; Rust Requirement::from_json_str fails duplicate-key strict parsing. Public from_json_str/from_json_bytes explicitly describe refusal as a security choice, but migration requires existing model/HTTP contracts. Provide a correct last-wins model JSON entry point and make the default consumer path source-compatible. Strict canonical/hash JSON remains strict and must continue rejecting duplicate keys (especially CanonicalJsonObject supplied as a JSON string). Do not weaken kala-json or silently route HTTP through a changed strict boundary.

## P2 — construction alias interpretation must preserve input order

`ComponentStep` JSON with op=sketch and profile `{"rectangle":{"width":10,"height":4},"circle":{"diameter":8}}`: Python produces rectangle,width10,height4; Rust produces circle,diameter8. Reversing alias key order produces circle in both, proving the first Rust result silently changed geometry. Python _flatten_profile iterates insertion order; parsing into sorted CanonicalValue loses it before validation.

Add an ordered input representation/path or normalize ordered input before canonical conversion, preserving alias-first behavior for ComponentStep nested within ComponentScript/ConstructionScript and loader entry points. Already-parsed CanonicalValue has no insertion metadata, so document that API's interpretation honestly and give real HTTP/JSON consumers an order-preserving route. Do not pretend BTree iteration recovers original order. Hashing still sorts keys after validation. Test both alias orders and duplicate aliases/fields across nested scripts against actual Python, including JSON string/bytes and loader consumers. Ordinary model input accepted/rejected sets and canonical wire/digest outputs need regression coverage.

Independent URI probes for snowman, emoji, zero-width joiner and capital-sharp-S hosts matched Python; no additional confirmed IDNA bug is asserted. Retain explicit exotic-IDNA limitation until evidence justifies more work. Ordinary error text details are not yet core422 acceptance; do not claim full HTTP parity based only on model output tests.

Proposed repair ownership remains ONLY kala-contracts and this cell's reports, standalone manifest/lock; root workspace consolidation owns parent and six accepted crate manifests. No source/sharedcrate/corpus/Python/frontend/packaging/user edits. No dependent domain/repository/kernel writer begins before repaired contracts acceptance. Original TASK.md/REPORT.md remain the full brief for the eventual fresh Go/max repair round.
