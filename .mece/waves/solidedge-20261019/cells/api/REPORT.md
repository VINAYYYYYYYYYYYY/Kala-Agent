# Windows probe API evidence (Solid Edge COM)

Status: research complete, **no Windows host touched**. Everything below is
documentation-derived and marked where it is inference rather than proof.
No implementation files were changed; only this report was written.

## Tools and checks actually run

- Read `docs/studio.md`, `studio-20261018/SOURCES.md`, `WAVE.md`, sibling TASKs.
- `firecrawl` CLI and Firecrawl MCP are **not installed/configured** here
  (no binary, no `FIRECRAWL_*` env, no MCP resources). Used `webfetch`/`curl`
  instead — a documented deviation from the task's "use Firecrawl skills".
- Fetched Siemens ST6 SDK pages, stripped tags with a local Python script and
  read the syntax blocks. Fetched pywin32 sources (`win32com/client/dynamic.py`,
  `build.py`, `__init__.py`) and the `VARIANT`/generated-support docs to check
  marshalling claims.
- No typelib available (Linux), so **no constant below is confirmed against an
  installed Solid Edge**. The probe must resolve constants by name and fail closed.

## Sources

1. Connecting: <https://support.industrysoftware.automation.siemens.com/trainings/se/106/api/ConnectingToSolidEdge.html>
2. `Documents.Add`: <.../SolidEdgeFramework~Documents~Add.html>
3. `AddFiniteExtrudedProtrusion`: <.../SolidEdgePart~Models~AddFiniteExtrudedProtrusion.html>
4. `Body.GetRange`: <.../SolidEdgeGeometry~Body~GetRange.html>
5. `PartDocument.SaveAs`: <.../SolidEdgePart~PartDocument~SaveAs.html> (+ `~SaveCopyAs`, `~Close`)

(All under `https://support.industrysoftware.automation.siemens.com/trainings/se/106/api/`.)
Supplementary member pages from the same ST6 tree supplied argument order and
enum values: `PartDocument_members`, `Profile~Lines2d|Relations2d|End`,
`Profiles~Add`, `ProfileSets~Add`, `RefPlanes~Item`, `Model~Get|ComputePhysicalProperties`,
`Models~Item`, `ModelingMode`, `FrameworkSupport~Lines2d~AddBy2Points`,
`Relations2d~AddKeypoint|AddHorizontal`, `FeaturePropertyConstants`,
`ProfileValidationType`, `ModelingModeConstants`, `KeypointIndexConstants`,
`DocumentTypeConstants`. Plus `soliddna.com/SEHelp/ST5/EN/i_v/save2a.htm`
(STEP is a Save As translator; options live in `STEP.INI`, not the API).

## Verified signatures (VB declaration → Python call)

```text
Application.Documents : Documents            # property
Documents.Add(Optional ProgID As Variant, Optional TemplateDoc As Variant) -> Object
    # ProgID must be "SolidEdge.PartDocument"; omitting it with
    # DisplayAlerts=False raises a runtime error (documented).

PartDocument.Models / .ProfileSets / .RefPlanes   # properties
PartDocument.ModelingMode : ModelingModeConstants # read-write
PartDocument.SaveAs(NewName As String, Optional IsATemplate, FileFormat,
                    ReadOnlyEnforced, ReadOnlyRecommended, NewStatus,
                    CreateBackup, UpdateLinkInContainer, UpdateAllLinksInContainer)
PartDocument.SaveCopyAs(Name As String)
PartDocument.Close(Optional SaveChanges, Optional FileName, Optional RouteWorkbook)
PartDocument.FullName / .Dirty                   # properties

ProfileSets.Add() -> ProfileSet
ProfileSet.Profiles -> Profiles
Profiles.Add(pRefPlaneDisp As Object) -> Profile
Profile.Lines2d -> Lines2d ; Profile.Relations2d -> Object ; Profile.Plane (rw)
Profile.End(ValidationCriteria As ProfileValidationType) -> Integer   # 0 == OK
Lines2d.AddBy2Points(x1, y1, x2, y2 As Double) -> Line2d
Relations2d.AddHorizontal(Object As Object, Optional guaranteed_ok) -> Relation2d
Relations2d.AddVertical(Object As Object, Optional guaranteed_ok) -> Relation2d
Relations2d.AddKeypoint(Object1, Index1 As Long, Object2, Index2 As Long,
                        Optional guaranteed_ok) -> Relation2d

Models.Item(Index As Variant) -> Model
Models.AddFiniteExtrudedProtrusion(
    NumberOfProfiles As Long, ByRef ProfileArray() As Object,
    ProfilePlaneSide As FeaturePropertyConstants, ExtrusionDistance As Double,
    Optional KeyPointOrTangentFace, KeyPointFlags,
    FromSurfOrRefPlane, ToSurfOrRefPlane) -> Model
Model.Body -> Object
Model.ComputePhysicalProperties(Density As Double, Accuracy As Double,
    ByRef Volume As Double, ByRef Area As Double, ByRef Mass As Double,
    ByRef CenterOfGravity() As Double, ByRef CenterOfVolume() As Double,
    ByRef GlobalMomentsOfInteria() As Double,
    ByRef PrincipalMomentsOfInteria() As Double, ByRef PrincipalAxes() As Double,
    ByRef RadiiOfGyration() As Double, ByRef RelativeAccuracyAchieved As Double,
    ByRef Status As Integer)
Model.GetPhysicalProperties(...same out-params, Status first...)  # stored, may be stale
RefPlane.GetNormal(ByRef Normal() As Double) ; RefPlane.GetRootPoint(ByRef RootPoint() As Double)
```

Units are Solid Edge *database units*: metres. Official examples extrude `0.1`
and use `Radius:=0.05`, and `UnitsOfMeasure.ParseUnit` returns "database units".
Volume/area therefore come back as m³/m² (×1e9 / ×1e6 for mm).

## Enum names/values (ST6 docs; resolve by name at runtime)

- `FeaturePropertyConstants`: `igSymmetric`=3, `igFinite`=13, `igLeft`=1,
  `igRight`=2, `igBoth`=6, `igNone`=44, `igThroughAll`=16.
- `ProfileValidationType`: `igProfileClosed`=1, `igProfileSingle`=4,
  `igProfileNoSelfIntersect`=8, `igProfileRefAxisRequired`=16.
- `KeypointIndexConstants`: `igLineStart`=0, `igLineEnd`=1, `igLineMiddle`=2,
  `igCircleCenter`=0.
- `ModelingModeConstants`: `seModelingModeSynchronous`=1, `seModelingModeOrdered`=2.
- `DocumentTypeConstants`: `igPartDocument`=1.
- `PhysicalPropertiesStatusConstants`: `sePhysicalPropertiesStatus_None`=0,
  `_Model`=1, `_User`=2.

## pywin32 early binding, out-params, SAFEARRAY

- Prefer `win32com.client.gencache.EnsureDispatch("SolidEdge.Application")`
  (generates wrappers from the *installed* typelib). `Dispatch`/`DumbDispatch`
  also build methods from typeinfo via `build.BuildCallList`, but generated
  modules are the documented path for by-ref support.
- **Named keyword binding**: generated signatures take names from
  `ITypeInfo::GetNames`, so `NumberOfProfiles=1` works only if that is the exact
  case-sensitive typelib name. pywin32 munges Python keywords and unnamed params
  (`MakePublicAttributeName`, `argN`). Verdict: probably safe but
  version-dependent — the probe should use **positional** args and log
  `inspect.signature(...)` once as evidence. Optional args must be named when
  skipping earlier ones.
- **Scalar `ByRef`**: `VARIANT(pythoncom.VT_BYREF | pythoncom.VT_R8, 0.0)`,
  read `.value` afterwards. **`ByRef X() As Double`**:
  `VARIANT(VT_BYREF|VT_ARRAY|VT_R8, [0.0]*3)` — the list must pre-exist.
  This is the documented VARIANT pattern; whether Solid Edge declares
  `VT_ARRAY|VT_R8|VT_BYREF` or `VT_ARRAY|VT_VARIANT` is an installed-typelib
  fact the probe must confirm.
- **`ByRef ProfileArray() As Object`**: try a plain tuple of `Profile`
  dispatches first; on `DISP_E_TYPEMISMATCH`/`E_INVALIDARG` retry with
  `VARIANT(VT_BYREF|VT_ARRAY|VT_VARIANT, [profile])`. Encode both attempts and
  report which worked; do not guess in production code.
- **Constants**: `win32com.client.constants` is populated from the most
  recently loaded generated module, which may be Framework rather than Part.
  Resolve each name, record the number, fail closed if absent — never silently
  substitute a literal.
- **Bitness**: out-of-process IDispatch marshalling does not require Python and
  Solid Edge to match bitness. Assert no bitness rule; let the host probe decide.
- **STA**: `pythoncom.CoInitializeEx(pythoncom.COINIT_APARTMENTTHREADED)`
  (`== 2`) before any COM call; do **not** set `sys.coinit_flags = 0` (that is
  MTA). Treat `RPC_E_CHANGED_MODE` as a hard reported failure, keep COM on that
  one thread, and retry `RPC_E_CALL_REJECTED` (`0x80010001`) with bounded backoff.

## Minimal probe pseudocode

```python
pythoncom.CoInitializeEx(pythoncom.COINIT_APARTMENTTHREADED)
app = win32com.client.gencache.EnsureDispatch("SolidEdge.Application")  # attaches if running
C = win32com.client.constants            # verify each name; abort if missing
doc = app.Documents.Add("SolidEdge.PartDocument")   # positional ProgID only
try:
    doc.ModelingMode = C.seModelingModeOrdered      # record previous value first
    prof = doc.ProfileSets.Add().Profiles.Add(doc.RefPlanes.Item(1))
    n = doc.RefPlanes.Item(1).GetNormal(...)        # record plane identity
    L = 0.010
    p = [(-L/2,-L/2), (L/2,-L/2), (L/2,L/2), (-L/2,L/2)]
    for i in range(4):
        prof.Lines2d.AddBy2Points(*p[i], *p[(i+1) % 4])
    prof.End(C.igProfileClosed)                     # 0 == success
    model = doc.Models.AddFiniteExtrudedProtrusion(1, (prof,),
                C.igSymmetric, L)
    mn = VARIANT(VT_BYREF|VT_ARRAY|VT_R8, [0.0]*3)
    mx = VARIANT(VT_BYREF|VT_ARRAY|VT_R8, [0.0]*3)
    model.Body.GetRange(mn, mx)
    vol = VARIANT(VT_BYREF|VT_R8, 0.0)
    model.ComputePhysicalProperties(1000.0, 0.001, vol, ...)  # all out-params as VARIANTs
    doc.SaveAs(str(out / "probe.par"))
    doc.SaveCopyAs(str(out / "probe.stp"))          # verify ISO-10303-21 header
finally:
    doc.Close()                                     # after save: doc is clean, no prompt
    # never app.Quit(); never read app.ActiveDocument
```

Guard: before `Close`, compare `doc.FullName` with the path we created and only
close on match; release COM references in `finally`.

## Real uncertainties (do not paper over)

1. **RefPlanes index → plane orientation unverified.** All official examples use
   `RefPlanes(1)` without saying which plane. Record `GetNormal`/`GetRootPoint`
   and derive the mapping; do not hardcode XY=1.
2. **Ordered vs synchronous template.** `AddFiniteExtrudedProtrusion` is an
   ordered API; on a synchronous document unsupported members return
   `CO_E_NOT_SUPPORTED`. Setting `ModelingMode` may be required and may itself be
   restricted by template/edition.
3. **`SaveCopyAs` to `.stp`** is documented only as "saves a copy … without
   modifying the open document"; translation is not stated. Fall back to
   `SaveAs("….stp")` if no STEP appears, and validate the ISO-10303-21 header.
4. **Out-param VARIANT shape** for `GetRange`, `ComputePhysicalProperties` and
   `ProfileArray` is inferred from pywin32, not observed on the Solid Edge typelib.
5. **Density/accuracy units** for `ComputePhysicalProperties` are undocumented;
   metres and kg/m³ are reasonable but must be calibrated.
6. **STEP flavour** (AP203/AP214) comes from `STEP.INI`, not the API.
7. **`win32com.client.constants` coverage** may lack Part-typelib enums unless
   that typelib was generated.
8. **No truly private application instance**: Solid Edge is single-instance, so
   attaching may create the document in the user's live session. Only the
   *document* is private.
9. **Student Edition automation access and version** are unknown; ST6 evidence is
   architectural, not compatibility proof.
10. A hung call is **indeterminate**; no evidence COM cancellation stops Solid
    Edge work.

## What the bridge implementer should take from this

Use positional arguments, one STA thread, `Documents.Add("SolidEdge.PartDocument")`,
`ProfileSets.Add().Profiles.Add(RefPlanes.Item(1))`, four `Lines2d.AddBy2Points`,
`Profile.End(igProfileClosed)`, `AddFiniteExtrudedProtrusion(1, (prof,), igSymmetric, 0.010)`,
`Body.GetRange` with by-ref SAFEARRAY VARIANTs, `ComputePhysicalProperties` for
volume, `SaveAs(.par)` + `SaveCopyAs(.stp)`, then `Close()` on the held document
object only. Every item under "Real uncertainties" must be answered by the probe's
own JSON output, not assumed.
