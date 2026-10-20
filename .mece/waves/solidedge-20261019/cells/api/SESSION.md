# SDK research session

Transport: T3 OpenCode delegation
Model: opencode/deepseek-v4.1-flash-free
Task ID: node:delegated-task:command%3Amcp%3A9ee55377-1277-48f7-9912-f1380e8319ed%3Adelegate-task%3Ase-impl-20261019-api-r1
Child thread: thread:delegated-task:command%3Amcp%3A9ee55377-1277-48f7-9912-f1380e8319ed%3Adelegate-task%3Ase-impl-20261019-api-r1
OpenCode session ID: not reported.

## Work log

- Firecrawl CLI/MCP unavailable (no binary, no `FIRECRAWL_*`, no MCP resources);
  used `webfetch`/`curl` against the Siemens ST6 SDK tree instead.
- Read the Siemens ST6 type-library pages for `Application`, `Documents.Add`,
  `PartDocument` (`SaveAs`, `SaveCopyAs`, `Close`, `ModelingMode`), `ProfileSets`,
  `Profiles`, `Profile` (`Lines2d`, `Relations2d`, `End`, `Plane`), `Lines2d.AddBy2Points`,
  `Relations2d.AddKeypoint/AddHorizontal/AddVertical`, `Models.AddFiniteExtrudedProtrusion`,
  `Model.Get/ComputePhysicalProperties`, `Model.Body`, `Body.GetRange`,
  `RefPlane.GetNormal/GetRootPoint`, and the enum pages (`FeaturePropertyConstants`,
  `ProfileValidationType`, `ModelingModeConstants`, `KeypointIndexConstants`,
  `DocumentTypeConstants`, `PhysicalPropertiesStatusConstants`).
- Cross-checked pywin32 `dynamic.py`/`build.py`/`__init__.py` and the VARIANT and
  generated-support docs for keyword binding, by-ref and SAFEARRAY behaviour.
- Wrote `.mece/waves/solidedge-20261019/cells/api/REPORT.md` (1198 words).
- Not run: any Windows COM call, typelib dump, or installed-version verification.
  No files outside this cell were modified.
