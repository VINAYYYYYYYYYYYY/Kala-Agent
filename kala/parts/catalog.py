"""Standard parts catalog — approximated with CAD primitives for agent use."""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from kala.cad.protocol import CadBackend, ToolResult


@dataclass(frozen=True)
class StandardPart:
    part_id: str
    name: str
    category: str
    dims_mm: dict[str, float]
    keywords: tuple[str, ...]


def _default_parts() -> list[StandardPart]:
    return [
        # Fasteners
        StandardPart(
            "hex_m3x8",
            "Hex Bolt M3x8",
            "fastener",
            {"diameter": 3.0, "length": 8.0},
            ("bolt", "hex", "m3", "fastener", "screw"),
        ),
        StandardPart(
            "hex_m3x12",
            "Hex Bolt M3x12",
            "fastener",
            {"diameter": 3.0, "length": 12.0},
            ("bolt", "hex", "m3", "fastener", "screw"),
        ),
        StandardPart(
            "hex_m3x16",
            "Hex Bolt M3x16",
            "fastener",
            {"diameter": 3.0, "length": 16.0},
            ("bolt", "hex", "m3", "fastener", "screw"),
        ),
        StandardPart(
            "nut_m3",
            "Hex Nut M3",
            "fastener",
            {"diameter": 5.5, "length": 2.4},
            ("nut", "m3", "hex", "fastener"),
        ),
        StandardPart(
            "hex_m6x20",
            "Hex Bolt M6x20",
            "fastener",
            {"diameter": 6.0, "length": 20.0},
            ("bolt", "hex", "m6", "fastener"),
        ),
        StandardPart(
            "hex_m8x25",
            "Hex Bolt M8x25",
            "fastener",
            {"diameter": 8.0, "length": 25.0},
            ("bolt", "hex", "m8", "fastener"),
        ),
        # Bearings (Hackaday robot gearbox + common)
        StandardPart(
            "bearing_608",
            "Ball Bearing 608-2RS",
            "bearing",
            {"inner": 8.0, "outer": 22.0, "width": 7.0},
            ("bearing", "608", "ball", "skate"),
        ),
        StandardPart(
            "bearing_mr128",
            "Ball Bearing MR128",
            "bearing",
            {"inner": 8.0, "outer": 12.0, "width": 3.5},
            ("bearing", "mr128", "ball", "planet"),
        ),
        StandardPart(
            "bearing_6709",
            "Thin-section Bearing 6709",
            "bearing",
            {"inner": 45.0, "outer": 55.0, "width": 6.0},
            ("bearing", "6709", "output", "thin"),
        ),
        StandardPart(
            "bearing_6704",
            "Thin-section Bearing 6704",
            "bearing",
            {"inner": 20.0, "outer": 27.0, "width": 4.0},
            ("bearing", "6704", "ball"),
        ),
        # Motion / structure
        StandardPart(
            "nema17_body",
            "NEMA17 Stepper Body",
            "motor",
            {"width": 42.3, "height": 42.3, "length": 40.0, "shaft_d": 5.0, "shaft_l": 22.0},
            ("nema17", "stepper", "motor", "nema"),
        ),
        StandardPart(
            "profile_2020",
            "Aluminum Extrusion 20x20",
            "profile",
            {"width": 20.0, "height": 20.0, "length": 100.0},
            ("extrusion", "2020", "profile", "aluminum", "vslot"),
        ),
        StandardPart(
            "profile_2020_300",
            "Aluminum Extrusion 20x20 x300",
            "profile",
            {"width": 20.0, "height": 20.0, "length": 300.0},
            ("extrusion", "2020", "profile", "aluminum", "frame"),
        ),
        # Gear blanks (no teeth yet — OD/ID/width envelopes for planetary layout)
        StandardPart(
            "gear_sun_nema17",
            "Sun Gear Blank (NEMA17 shaft)",
            "gear",
            {"outer": 20.0, "inner": 5.0, "width": 8.0},
            ("gear", "sun", "pinion", "planetary"),
        ),
        StandardPart(
            "gear_planet",
            "Planet Gear Blank",
            "gear",
            {"outer": 24.0, "inner": 8.0, "width": 8.0},
            ("gear", "planet", "planetary"),
        ),
        StandardPart(
            "gear_ring_internal",
            "Ring Gear Blank (internal)",
            "gear",
            {"outer": 70.0, "inner": 56.0, "width": 10.0},
            ("gear", "ring", "annulus", "internal", "planetary"),
        ),
    ]


class PartsCatalog:
    # Common LLM / human aliases → catalog ids
    ALIASES: dict[str, str] = {
        # NEMA17 motor / stepper aliases
        "nema17": "nema17_body",
        "nema_17": "nema17_body",
        "nema-17": "nema17_body",
        "stepper": "nema17_body",
        "stepper_motor": "nema17_body",
        "motor": "nema17_body",
        "steppermotor": "nema17_body",
        "nema17motor": "nema17_body",
        "nema17_motor": "nema17_body",
        
        # Bearing 608 aliases (common skate bearing)
        "608": "bearing_608",
        "bearing_608": "bearing_608",
        "608_2rs": "bearing_608",
        "608zz": "bearing_608",
        "skate_bearing": "bearing_608",
        "skatebearing": "bearing_608",
        
        # Bearing MR128 aliases (planet bearing)
        "mr128": "bearing_mr128",
        "bearing_mr128": "bearing_mr128",
        "mr_128": "bearing_mr128",
        "mr128zz": "bearing_mr128",
        "planet_bearing": "bearing_mr128",
        
        # Bearing 6709 aliases (thin section output bearing)
        "6709": "bearing_6709",
        "bearing_6709": "bearing_6709",
        "6709zz": "bearing_6709",
        "output_bearing": "bearing_6709",
        "thin_bearing": "bearing_6709",
        
        # Bearing 6704 aliases
        "6704": "bearing_6704",
        "bearing_6704": "bearing_6704",
        "6704zz": "bearing_6704",
        
        # M3 bolt variants (8mm length)
        "m3x8": "hex_m3x8",
        "m3_x_8": "hex_m3x8",
        "m3x8mm": "hex_m3x8",
        "hex_m3_x_8": "hex_m3x8",
        
        # M3 bolt variants (12mm length)
        "m3x10": "hex_m3x12",
        "m3x12": "hex_m3x12",
        "m3_x_12": "hex_m3x12",
        "m3x12mm": "hex_m3x12",
        "hex_m3_x_12": "hex_m3x12",
        
        # M3 bolt variants (16mm length)
        "m3x16": "hex_m3x16",
        "m3_x_16": "hex_m3x16",
        "m3x16mm": "hex_m3x16",
        "hex_m3_x_16": "hex_m3x16",
        
        # Generic M3 fasteners (default to 8mm)
        "m3": "hex_m3x8",
        "m3bolt": "hex_m3x8",
        "m3_bolt": "hex_m3x8",
        "m3screw": "hex_m3x8",
        "m3_screw": "hex_m3x8",
        "bolt_m3": "hex_m3x8",
        "screw_m3": "hex_m3x8",
        "hexbolt_m3": "hex_m3x8",
        "hex_bolt_m3": "hex_m3x8",
        
        # M3 nut aliases
        "nut": "nut_m3",
        "m3nut": "nut_m3",
        "m3_nut": "nut_m3",
        "nut_m3": "nut_m3",
        "hex_nut_m3": "nut_m3",
        "hexnut_m3": "nut_m3",
        "hexnut": "nut_m3",
        
        # M6 bolt aliases
        "m6": "hex_m6x20",
        "m6x20": "hex_m6x20",
        "m6_x_20": "hex_m6x20",
        "m6x20mm": "hex_m6x20",
        "hex_m6_x_20": "hex_m6x20",
        "m6bolt": "hex_m6x20",
        "m6_bolt": "hex_m6x20",
        "bolt_m6": "hex_m6x20",
        
        # M8 bolt aliases
        "m8": "hex_m8x25",
        "m8x25": "hex_m8x25",
        "m8_x_25": "hex_m8x25",
        "m8x25mm": "hex_m8x25",
        "hex_m8_x_25": "hex_m8x25",
        "m8bolt": "hex_m8x25",
        "m8_bolt": "hex_m8x25",
        "bolt_m8": "hex_m8x25",
        
        # Generic fastener aliases
        "bolt": "hex_m3x8",
        "screw": "hex_m3x8",
        "fastener": "hex_m3x8",
        "hex_bolt": "hex_m3x8",
        "hexbolt": "hex_m3x8",
        
        # Sun gear aliases
        "sun": "gear_sun_nema17",
        "sun_gear": "gear_sun_nema17",
        "sungear": "gear_sun_nema17",
        "gear_sun": "gear_sun_nema17",
        "gear_blank_sun": "gear_sun_nema17",
        "pinion": "gear_sun_nema17",
        
        # Planet gear aliases
        "planet": "gear_planet",
        "planet_gear": "gear_planet",
        "planetgear": "gear_planet",
        "gear_planet": "gear_planet",
        "gear_blank_planet": "gear_planet",
        
        # Ring gear aliases
        "ring": "gear_ring_internal",
        "ring_gear": "gear_ring_internal",
        "ringgear": "gear_ring_internal",
        "gear_ring": "gear_ring_internal",
        "gear_blank_ring": "gear_ring_internal",
        "annulus": "gear_ring_internal",
        "annulus_gear": "gear_ring_internal",
        "internal_gear": "gear_ring_internal",
        
        # Generic gear aliases (default to planet)
        "gear": "gear_planet",
        "gear_blank": "gear_planet",
        
        # 2020 profile aliases
        "2020": "profile_2020",
        "profile_2020": "profile_2020",
        "2020_profile": "profile_2020",
        "extrusion_2020": "profile_2020",
        "2020_extrusion": "profile_2020",
        "alu_profile_2020": "profile_2020",
        "aluminum_2020": "profile_2020",
        "vslot": "profile_2020",
        "v_slot": "profile_2020",
        "tslot": "profile_2020",
        "t_slot": "profile_2020",
        
        # 2020 profile 300mm variant aliases
        "profile_2020_300": "profile_2020_300",
        "2020x300": "profile_2020_300",
        "2020_x_300": "profile_2020_300",
        "extrusion_2020_300": "profile_2020_300",
        
        # Generic profile/extrusion aliases (default to 2020)
        "profile": "profile_2020",
        "extrusion": "profile_2020",
        "aluminum_extrusion": "profile_2020",
        "alu_extrusion": "profile_2020",
        "frame": "profile_2020",
        "rail": "profile_2020",

        # Additional common aliases mapping only to existing ids
        "m3_bolt_8": "hex_m3x8",
        "m3_bolt_12": "hex_m3x12",
        "m3_bolt_16": "hex_m3x16",
        "nema_17_motor": "nema17_body",
        "sun_gear_nema17": "gear_sun_nema17",
        "planet_gear_blank": "gear_planet",
        "ring_gear_internal": "gear_ring_internal",
        "profile_2020_100": "profile_2020",
    }

    def __init__(self, parts: list[StandardPart]) -> None:
        self._parts = {p.part_id: p for p in parts}

    @classmethod
    def default(cls) -> PartsCatalog:
        return cls(_default_parts())

    def _learned_aliases(self) -> dict[str, str]:
        """Optional overnight-learned aliases from batch_eval."""
        path = Path(__file__).with_name("learned_aliases.json")
        if not path.is_file():
            return {}
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
            raw = data.get("aliases") or {}
            return {str(k).lower(): str(v) for k, v in raw.items() if k and v}
        except Exception:
            return {}

    def resolve_id(self, part_id: str) -> str | None:
        raw = part_id.strip()
        if raw in self._parts:
            return raw
        key = raw.lower().replace(" ", "_").replace("-", "_")
        if key in self._parts:
            return key
        if key in self.ALIASES:
            return self.ALIASES[key]
        learned = self._learned_aliases()
        if key in learned and learned[key] in self._parts:
            return learned[key]
        return None

    def search(self, query: str) -> ToolResult:
        from kala.cad.protocol import ToolResult

        q = query.lower().strip()
        hits = []
        for part in self._parts.values():
            hay = " ".join([part.part_id, part.name, part.category, *part.keywords]).lower()
            if not q or q in hay or any(tok in hay for tok in q.split()):
                hits.append(
                    {
                        "part_id": part.part_id,
                        "name": part.name,
                        "category": part.category,
                        "dims_mm": part.dims_mm,
                    }
                )
        return ToolResult(
            ok=True,
            message=f"{len(hits)} parts matched",
            data={"parts": hits, "query": query},
        )

    def _suggest_ids(self, part_id: str, *, limit: int = 5) -> list[str]:
        """Suggest existing catalog ids for an unresolved part_id (never invent ids)."""
        needle = part_id.strip().lower().replace(" ", "_").replace("-", "_")
        hits: list[str] = []
        # Strong contains against real ids only (avoid short-alias false positives like "ring" in "bearing")
        for pid in self._parts:
            pl = pid.lower()
            if len(needle) >= 3 and (needle in pl or pl in needle):
                hits.append(pid)
        for alias, pid in self.ALIASES.items():
            if pid not in self._parts:
                continue
            if len(alias) < 4:
                continue
            if needle in alias or alias in needle:
                hits.append(pid)
        if not hits and needle:
            search_hits = self.search(part_id)
            for row in (search_hits.data.get("parts") or [])[:limit]:
                pid = row.get("part_id")
                if isinstance(pid, str) and pid in self._parts:
                    hits.append(pid)
        seen: set[str] = set()
        out: list[str] = []
        for pid in hits:
            if pid not in seen:
                seen.add(pid)
                out.append(pid)
            if len(out) >= limit:
                break
        return out

    def insert(
        self,
        backend: CadBackend,
        part_id: str,
        *,
        x: float = 0.0,
        y: float = 0.0,
        z: float = 0.0,
    ) -> ToolResult:
        from kala.cad.protocol import ToolResult

        resolved = self.resolve_id(part_id)
        if resolved is None:
            suggestions = self._suggest_ids(part_id)
            hint = ", ".join(suggestions) if suggestions else "none — call search_parts"
            msg = (
                f"Unknown part_id '{part_id}'. "
                f"Try search_parts for catalog ids. Suggestions: {hint}"
            )
            return ToolResult(
                ok=False,
                message=msg,
                data={"suggestions": suggestions, "query": part_id},
            )
        part = self._parts[resolved]

        result = self._build(backend, part)
        if not result.ok:
            return result
        body_id = result.data.get("body_id")
        if body_id and (x or y or z):
            backend.translate(str(body_id), x, y, z)
        note = f"Inserted standard part {part.name} → {body_id}"
        if resolved != part_id:
            note += f" (resolved '{part_id}' → '{resolved}')"
        return ToolResult(
            ok=True,
            message=note,
            data={
                "part_id": part.part_id,
                "requested_part_id": part_id,
                "body_id": body_id,
                "approx": True,
                "category": part.category,
                "dims_mm": part.dims_mm,
            },
        )

    def _build(self, backend: CadBackend, part: StandardPart) -> ToolResult:
        from kala.cad.protocol import ToolResult

        d = part.dims_mm
        if part.category == "bearing":
            # Tube: outer cylinder minus inner bore
            outer = backend.create_cylinder(
                d["outer"] / 2.0, d["width"], label=part.part_id
            )
            if not outer.ok:
                return outer
            oid = str(outer.data["body_id"])
            inner = backend.create_cylinder(
                d["inner"] / 2.0, d["width"] + 2.0, label=f"{part.part_id}_bore"
            )
            if not inner.ok:
                return inner
            iid = str(inner.data["body_id"])
            backend.translate(iid, 0.0, 0.0, -1.0)
            cut = backend.boolean_cut(oid, iid)
            if cut.ok:
                cut.data["part_id"] = part.part_id
            return cut

        if part.category == "gear":
            outer = backend.create_cylinder(
                d["outer"] / 2.0, d["width"], label=part.part_id
            )
            if not outer.ok:
                return outer
            oid = str(outer.data["body_id"])
            if d.get("inner", 0) > 0:
                inner = backend.create_cylinder(
                    d["inner"] / 2.0, d["width"] + 2.0, label=f"{part.part_id}_bore"
                )
                if not inner.ok:
                    return inner
                iid = str(inner.data["body_id"])
                backend.translate(iid, 0.0, 0.0, -1.0)
                cut = backend.boolean_cut(oid, iid)
                if cut.ok:
                    cut.data["part_id"] = part.part_id
                return cut
            return outer

        if part.category == "motor":
            body = backend.create_box(
                d["length"], d["width"], d["height"], label=part.part_id
            )
            if not body.ok:
                return body
            bid = str(body.data["body_id"])
            shaft = backend.create_cylinder(
                d.get("shaft_d", 5.0) / 2.0,
                d.get("shaft_l", 22.0),
                label=f"{part.part_id}_shaft",
            )
            if not shaft.ok:
                return body
            sid = str(shaft.data["body_id"])
            # Default cylinder is +Z; rotate about Y so axis is +X, then place on front face center
            backend.rotate(sid, "y", 90.0, cx=0.0, cy=0.0, cz=0.0)
            backend.translate(
                sid,
                d["length"],
                d["width"] / 2.0,
                d["height"] / 2.0,
            )
            return ToolResult(
                ok=True,
                message=f"Inserted motor body {bid} + shaft {sid}",
                data={"body_id": bid, "shaft_id": sid, "part_id": part.part_id},
            )

        if part.category == "profile":
            return backend.create_box(
                d.get("length", 100.0),
                d["width"],
                d["height"],
                label=part.part_id,
            )

        # fasteners
        return backend.create_cylinder(
            d.get("diameter", 6.0) / 2.0,
            d.get("length", 20.0),
            label=part.part_id,
        )
