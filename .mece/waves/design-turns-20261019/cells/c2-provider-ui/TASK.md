# C2 provider UI (after B2)
Read ADR-0011 and `../b2-providers/REPORT.md`. Own studio/src/** and studio/test/**.
- A CAD target selector in the composer (Solid Edge / FreeCAD / OpenCascade), with availability from `GET /providers/cad` (unavailable targets are disabled with the reason in a tooltip). There is NO default selection: the first design turn asks the user to pick. The choice persists for the session only.
- The agent card, bottom bar, and title block label the source: "measured by <provider> <version>". A `not_reported` check shows "—", with no substitution.
- The viewer always shows the imported provider STEP. Label it "view: OpenCascade import (view-only)" unless the provider is opencascade.
- Tests, and the gates `npm run typecheck && npm test && npm run build`. No screenshots (final QA does them).

## Added by Opus after reviewing B2 (binding): B2 CHANGED the API contract the UI uses
Read `../b2-providers/REPORT.md` (new shapes) and `../c1a-data/REPORT.md` (the current client and validators). You must update the data layer too:
- `DesignResult`: `mesh` → `view` (from the OpenCascade import of the provider STEP); `measured` is nullable per field (`not_reported`); `checks[].status` (pass/fail/not_reported); `provider`, `provider_version`, `measured_by`; native and STEP artifact digests.
- `POST /design` requires `provider`; `ChatRequest.cad_provider`; `GET /providers/cad`.
Update `studio/src/types/studio.ts`, `api/validators.ts`, `api/client.ts`, `state/designs.ts` (designToDisplayMesh reads `view`), and every component that reads them (agent card, bottom bar, title block, drawing, AuxView, export receipt). Validators must accept the new shapes and reject the old ones (no silent coercion of `null` to 0). The UI must show "—" for `not_reported`. Then do the selector work above. Tests for all of it. Also read `../d1-qa/REPORT.md` and fix any issue it lists that is still open after these changes.
