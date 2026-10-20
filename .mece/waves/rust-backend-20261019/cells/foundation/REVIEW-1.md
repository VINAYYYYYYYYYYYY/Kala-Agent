# GPT-6.1-Sol review — accepted foundation slice

Conductor inspected parser, serializer, float formatter, code-point values, digest and chain APIs plus corpus/differential tests. Independently reran cargo test -p kala-json --locked (23 passed), fmt, workspace clippy with -D warnings and release build, all passed. Temporary out-of-tree adversarial probe on a 2 MiB thread: nested arrays/objects at49/200/500/900/1024 returned DepthExceeded,1100 returned NotJson; no abort or stack overflow.

Accepted as the canonical tools.protocol and chain foundation, not complete persistence or backend. No blocking defect found in this slice. Known rejected-input first-error differences remain documented. Parser's1024 syntactic recursion limit does not exactly reproduce process-dependent CPython decoder recursion thresholds; both reject those inputs. No Windows execution or benchmark evidence.

Consumer requirements:
- Persistence contracts.models.CanonicalJsonObject uses a DIFFERENT, effectively unbounded canonicalization surface from tools.protocol. Never apply default tool byte/string/key/array/depth limits to persisted objects. Configure Limits explicitly and add live Python tests exceeding each tool bound.
- Sha256Digest::from_hex accepts uppercase hex and normalizes to bytes. Storage readers must validate original chain strings are exactly lowercase64hex BEFORE converting; the typed digest does not prove this condition.
- kala_json::verify_chain covers only typed fields. Persistence must additionally verify actual payload bytes/digest, stored digest validity, canonical object bytes, identifiers/timestamps and complete old-row chain integrity on reads. Never use this helper alone as full database verification.
- No schema/repository, provenance, service, CLI or packaging cutover has occurred. Python is a test oracle only.
