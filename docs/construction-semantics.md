# Construction semantics

> Status: **measured reference, not a solved geometry kernel.** The
> numbers here were measured on FreeCAD 1.1.4 and are the shared
> expectations the Solid Edge integration is required to match for the
> supported subset. The compiler that lowers these cases is implemented
> and checked, but they are not a claim that Solid Edge has reproduced
> them; no Windows host was available to try.

## Why this document exists

Kala already has a neutral construction language:
`kala_agent.contracts.ConstructionScript` in
`src/kala_agent/contracts/construction.py`. It is what the FreeCAD
adapter compiles and what a future CAD integration should compile too.
The Solid Edge client does not invent a second script language. It
accepts the same `ConstructionScript`, refuses everything outside a
supported subset, and lowers the rest into bridge protocol v1
operations.

That only works if the two compilers agree on what a plane, an offset, a
cut, and a bound *mean*. This document pins those meanings to measured
FreeCAD behaviour. Where prose and measurement disagree, the measured
number wins.

The corpus is intentionally narrow. It contains only cases both sides
are expected to support. It does not contain screw-jack threads,
de-threading approximations, fillets, fuses, compounds, or FEA.

## Planes and normals

Three sketch planes are defined, each with a canonical normal. A profile
is placed on the plane in local `(u, v)` coordinates; the third world
axis is fixed by the plane.

- **XY** — a point at `(u, v)` is `(u, v, 0)`, and the normal is `+Z`.
- **XZ** — a point at `(u, v)` is `(u, 0, v)`, and the normal is `-Y`.
- **YZ** — a point at `(u, v)` is `(0, u, v)`, and the normal is `+X`.

An earlier draft of the plan described the XZ normal inconsistently
with its own numbers. The numeric measurement is authoritative: **XZ is
`-Y`**, not `+Y`. Anything that assumes otherwise mirrors an extrusion
on XZ and will produce a part on the wrong side of the plane.

An extrusion runs along the plane normal. `reversed = true` flips the
direction, so the solid grows along the opposite normal. Reversal
changes where the material is, not how much of it there is: the volume
of a reversed extrusion equals the volume of the forward one.

A cut removes material along the plane's canonical normal. It has no
`reversed` field in the v1 contract: the direction is fixed by the
contract and is independent of whether the preceding extrusion used
`reversed`. For `XY` that means the cut runs along `+Z`; for `XZ`, along
`-Y`; for `YZ`, along `+X`. A native executor must calibrate that
canonical direction before it can build a through-cut.

## A worked example: offset rectangle, pad 6

Sketch a rectangle of width `4`, height `5`, at offset `(u = 2, v = 3)`,
on each plane, and extrude it `6` mm along the plane normal. The
measured world-space bounding boxes and volume are:

- **XY** — bounds `(2, 3, 0)` to `(6, 8, 6)`; volume `120` mm³.
- **XZ** — bounds `(2, -6, 3)` to `(6, 0, 8)`; volume `120` mm³.
- **YZ** — bounds `(0, 2, 3)` to `(6, 6, 8)`; volume `120` mm³.

Read the XZ row carefully. The profile occupies `x` from `2` to `6` and
`z` from `3` to `8`, exactly as the `(u, 0, v)` mapping says. The
extrusion along the `-Y` normal puts the body from `y = 0` down to
`y = -6`, which is why the minimum is `-6` and the maximum is `0`.
A `+Y` normal would have produced `y` from `0` to `+6`, and the measured
data says that is wrong.

Volume is `4 × 5 × 6 = 120` mm³ on all three planes, because a plane
change rotates the body and does not resize it.

## Cuts and pockets

A pocket is a sketch followed by a cut. The measured reference case is:

- A `20 × 20` mm rectangle on `XY`, extruded `10` mm forward (`+Z`).
  The solid occupies `(0, 0, 0)` to `(20, 20, 10)`, volume `4000` mm³.
- A circle of diameter `2` mm centred at `(5, 5)` on the same `XY`
  plane, cut to a finite depth of `4` mm.

The measured volume is **`3987.433629` mm³** (six decimals). That is
`4000 − π·1²·4 = 4000 − 12.566371`, rounded to the six digits the
compilers report.

The same hole cut through-all measures **`3968.584073` mm³**, which is
`4000 − π·1²·10`. The difference is exactly the extra `6` mm of hole
depth. Through-all in v1 means through the existing solid along the
canonical cut direction, not through an unbounded space.

In the FreeCAD run this case was driven with `Pocket.Reversed = True`,
which removes `+Z` material from an `XY` bottom-plane `+Z` pad. That is
consistent with the bridge's canonical rule: a cut on `XY` runs along
`+Z`. The bridge contract does not expose a `reversed` flag, so a native
executor must reproduce the canonical direction rather than copy the
FreeCAD flag. If it cannot, it must report that the cut is uncalibrated
instead of removing material from the wrong side.

**A no-op cut can be valid.** If the profile lies outside the solid, or
the cut runs the wrong way, the operation can complete and remove
nothing. A volume that does not change is not automatically a failure
of the script — it is a failure of the *intent*. For that reason a
future native executor must compare the measured volume before and
after a cut and report a warning or a failure when the volume did not
decrease. It must not report a successful cut that removed nothing as
if material had been removed.

The same rule applies to a cut that removes everything: an empty or
zero-volume solid is invalid, and the compiler must fail rather than
publish a `.par` or STEP file for it.

## Revolves

Version 1 supports one revolve case and treats another as an extrusion.

- **Rectangle revolve.** A rectangle revolved about the **component-local
  `Y` axis** is supported on `XY` and `YZ`. A rectangle on `XZ` is
  **rejected before any request is submitted**, because the component
  `Y` axis is not the axis the profile is drawn around and the result is
  not the shared case. The refusal happens in the compiler, not in the
  bridge.
- **Circle revolve quirk.** A circle profile with `solid.revolve` is
  **not** revolved. It is lowered to an extrusion whose length is the
  supplied `length`, or the circle diameter when no length is given.
  This matches the existing FreeCAD lowering and is preserved
  deliberately, because silently producing a torus where the shared
  corpus expects a cylinder would be a wrong part that looks valid.

`solid.revolve` takes an `angle` in `(0, 360]` degrees and a fixed
`axis` of `Y`. There is no arbitrary axis and no partial-revolve
subtlety beyond the angle.

## Bounds: local features, world assembly

Two different bounding boxes matter and they must not be confused.

- **A feature's range box is in body-local coordinates.** It describes
  the shape of the part as modelled, before any placement.
- **A body's shape includes its placement.** When a part is placed in
  an assembly, its world position changes while its local geometry does
  not.
- **A `PartResult.range_box` is local.** It is the part's own extent.
- **`BuildResult.assembly_bounds` is world.** It is the extent of the
  placed occurrences, and it is only present when an assembly was
  built.

Both the minimum and the maximum corner matter. A box that is mirrored
by a reversed extrusion has the same size and a different minimum; a
box that is placed at a negative translation has the same size and a
different minimum. A comparison that checks only extents will pass a
part that is in the wrong place. The shared corpus compares both
corners, to `0.001` mm, and volume to `0.1 %`.

An assembly with two parts that overlap has world bounds that are the
union of the two boxes. That is **not** a boolean union and it is not a
claim about the material in the overlap. Version 1 has no fuse and no
compound, so no result may present overlapping parts as one solid or
sum their volumes as if they were disjoint.

## Assembly occurrences

A positioned assembly is a set of occurrences, not a FreeCAD compound
and not a Solid Edge compound feature. Each occurrence is a reference to
a finished part plus a translation. Consequences:

- **Each component may be placed at most once.** A duplicate
  `assembly.place` for the same component is rejected by the contract
  validator and by the client compiler. The corpus has no repeated
  instances, and v1 does not pretend to support them.
- **Every part is placed exactly once** in a supported assembly, and a
  missing translation defaults to zero. A part that is not placed is
  not in the assembly.
- **Assembly save and export require at least one placement.** An empty
  assembly is refused rather than written as a file.
- **No compound, no fuse.** The assembly is occurrences; the STEP file
  is the positioned set. The integration does not approximate a boolean
  operation that was not requested.

## The shared parity corpus

The corpus is the set of cases the FreeCAD adapter and the Solid Edge
compiler are expected to reproduce. It is deliberately small:

- A single rectangle pad on `XY`, `XZ`, or `YZ`, with and without an
  offset, forward and reversed.
- A single circle pad (extrude) on each plane, with and without an
  offset.
- A rectangle revolve on `XY` and `YZ`.
- A contained, same-plane circular hole cut, finite and through-all.
- Two or three finished parts placed once each with translations.

Explicitly **not** in the corpus:

- Helix, threads, and any de-threading or thread-approximation scheme.
  A screw jack is not modelled by approximating its thread as a
  cylinder; if threads are needed, they are out of scope until a
  supported thread feature exists.
- Fillets, chamfers, drafts, shells, and patterns.
- Fuse, compound, and boolean assembly operations.
- FEA, meshing, materials, loads, and results.
- Additive features beyond a single primary solid are accepted by the
  wire contract but are **not** in the measured parity corpus. They are
  therefore unchecked against a real host until a native executor
  exists.
- Repeated assembly instances of one component.

A case outside the corpus is **unchecked**, not "probably fine". The
oracle must say so rather than compute a guessed expectation and call it
a pass.

## Units and rounding

- Lengths are millimetres everywhere on the wire. The bridge converts to
  metres only where Solid Edge requires it.
- Angles are degrees on the wire and radians inside the runtime.
- Volumes are reported in mm³ and areas in mm².
- Bounds and volumes are reported to six decimal places; comparisons
  use `0.001` mm for bounds and `0.1 %` relative for volume.
- Mass is reported only when the request supplies an explicit
  `density_kg_m3`. There is no default density and no material
  catalogue, so a missing density yields `null` mass rather than a
  guessed one.

## What this document does not claim

- It does not claim Solid Edge reproduces these numbers. No Windows
  host and no Solid Edge edition were available; the FreeCAD
  measurements are the expectation, not the observation.
- It does not claim the corpus is complete, or that a case outside it
  will fail gracefully rather than produce a wrong solid. That is why
  the compiler refuses unsupported features before submission.
- It does not claim the FreeCAD numbers are exact beyond the stated
  rounding. They are measured values, and the tolerances above are the
  comparison contract.
- It does not define a general CAD semantics. It defines the subset the
  milestone supports, so that a mismatch is detectable rather than
  argued about.
