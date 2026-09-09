"""Test parts catalog alias resolution."""

import pytest

from kala.parts.catalog import PartsCatalog


class TestCatalogAliases:
    """Test that common search misses resolve to real catalog part_ids."""

    @pytest.fixture
    def catalog(self) -> PartsCatalog:
        return PartsCatalog.default()

    def test_nema17_motor_aliases(self, catalog: PartsCatalog) -> None:
        """Test NEMA17 motor aliases resolve correctly."""
        aliases = [
            "nema17",
            "nema_17",
            "nema-17",
            "stepper",
            "stepper_motor",
            "motor",
            "steppermotor",
            "nema17motor",
            "nema17_motor",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "nema17_body", f"Failed to resolve '{alias}' to nema17_body"

    def test_bearing_608_aliases(self, catalog: PartsCatalog) -> None:
        """Test bearing 608 aliases (common skate bearing)."""
        aliases = [
            "608",
            "bearing_608",
            "608_2rs",
            "608zz",
            "skate_bearing",
            "skatebearing",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "bearing_608", f"Failed to resolve '{alias}' to bearing_608"

    def test_bearing_mr128_aliases(self, catalog: PartsCatalog) -> None:
        """Test bearing MR128 aliases (planet bearing)."""
        aliases = [
            "mr128",
            "bearing_mr128",
            "mr_128",
            "mr128zz",
            "planet_bearing",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "bearing_mr128", f"Failed to resolve '{alias}' to bearing_mr128"

    def test_bearing_6709_aliases(self, catalog: PartsCatalog) -> None:
        """Test bearing 6709 aliases (thin section output bearing)."""
        aliases = [
            "6709",
            "bearing_6709",
            "6709zz",
            "output_bearing",
            "thin_bearing",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "bearing_6709", f"Failed to resolve '{alias}' to bearing_6709"

    def test_bearing_6704_aliases(self, catalog: PartsCatalog) -> None:
        """Test bearing 6704 aliases."""
        aliases = [
            "6704",
            "bearing_6704",
            "6704zz",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "bearing_6704", f"Failed to resolve '{alias}' to bearing_6704"

    def test_m3_bolt_8mm_aliases(self, catalog: PartsCatalog) -> None:
        """Test M3x8 bolt aliases."""
        aliases = [
            "m3x8",
            "m3_x_8",
            "m3x8mm",
            "hex_m3_x_8",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "hex_m3x8", f"Failed to resolve '{alias}' to hex_m3x8"

    def test_m3_bolt_12mm_aliases(self, catalog: PartsCatalog) -> None:
        """Test M3x12 bolt aliases."""
        aliases = [
            "m3x10",
            "m3x12",
            "m3_x_12",
            "m3x12mm",
            "hex_m3_x_12",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "hex_m3x12", f"Failed to resolve '{alias}' to hex_m3x12"

    def test_m3_bolt_16mm_aliases(self, catalog: PartsCatalog) -> None:
        """Test M3x16 bolt aliases."""
        aliases = [
            "m3x16",
            "m3_x_16",
            "m3x16mm",
            "hex_m3_x_16",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "hex_m3x16", f"Failed to resolve '{alias}' to hex_m3x16"

    def test_generic_m3_fastener_aliases(self, catalog: PartsCatalog) -> None:
        """Test generic M3 fastener aliases default to 8mm."""
        aliases = [
            "m3",
            "m3bolt",
            "m3_bolt",
            "m3screw",
            "m3_screw",
            "bolt_m3",
            "screw_m3",
            "hexbolt_m3",
            "hex_bolt_m3",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "hex_m3x8", f"Failed to resolve '{alias}' to hex_m3x8"

    def test_m3_nut_aliases(self, catalog: PartsCatalog) -> None:
        """Test M3 nut aliases."""
        aliases = [
            "nut",
            "m3nut",
            "m3_nut",
            "nut_m3",
            "hex_nut_m3",
            "hexnut_m3",
            "hexnut",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "nut_m3", f"Failed to resolve '{alias}' to nut_m3"

    def test_m6_bolt_aliases(self, catalog: PartsCatalog) -> None:
        """Test M6 bolt aliases."""
        aliases = [
            "m6",
            "m6x20",
            "m6_x_20",
            "m6x20mm",
            "hex_m6_x_20",
            "m6bolt",
            "m6_bolt",
            "bolt_m6",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "hex_m6x20", f"Failed to resolve '{alias}' to hex_m6x20"

    def test_m8_bolt_aliases(self, catalog: PartsCatalog) -> None:
        """Test M8 bolt aliases."""
        aliases = [
            "m8",
            "m8x25",
            "m8_x_25",
            "m8x25mm",
            "hex_m8_x_25",
            "m8bolt",
            "m8_bolt",
            "bolt_m8",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "hex_m8x25", f"Failed to resolve '{alias}' to hex_m8x25"

    def test_generic_fastener_aliases(self, catalog: PartsCatalog) -> None:
        """Test generic fastener aliases default to M3x8."""
        aliases = [
            "bolt",
            "screw",
            "fastener",
            "hex_bolt",
            "hexbolt",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "hex_m3x8", f"Failed to resolve '{alias}' to hex_m3x8"

    def test_sun_gear_aliases(self, catalog: PartsCatalog) -> None:
        """Test sun gear aliases."""
        aliases = [
            "sun",
            "sun_gear",
            "sungear",
            "gear_sun",
            "gear_blank_sun",
            "pinion",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "gear_sun_nema17", f"Failed to resolve '{alias}' to gear_sun_nema17"

    def test_planet_gear_aliases(self, catalog: PartsCatalog) -> None:
        """Test planet gear aliases."""
        aliases = [
            "planet",
            "planet_gear",
            "planetgear",
            "gear_planet",
            "gear_blank_planet",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "gear_planet", f"Failed to resolve '{alias}' to gear_planet"

    def test_ring_gear_aliases(self, catalog: PartsCatalog) -> None:
        """Test ring gear aliases."""
        aliases = [
            "ring",
            "ring_gear",
            "ringgear",
            "gear_ring",
            "gear_blank_ring",
            "annulus",
            "annulus_gear",
            "internal_gear",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "gear_ring_internal", f"Failed to resolve '{alias}' to gear_ring_internal"

    def test_generic_gear_aliases(self, catalog: PartsCatalog) -> None:
        """Test generic gear aliases default to planet gear."""
        aliases = [
            "gear",
            "gear_blank",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "gear_planet", f"Failed to resolve '{alias}' to gear_planet"

    def test_2020_profile_aliases(self, catalog: PartsCatalog) -> None:
        """Test 2020 profile aliases."""
        aliases = [
            "2020",
            "profile_2020",
            "2020_profile",
            "extrusion_2020",
            "2020_extrusion",
            "alu_profile_2020",
            "aluminum_2020",
            "vslot",
            "v_slot",
            "tslot",
            "t_slot",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "profile_2020", f"Failed to resolve '{alias}' to profile_2020"

    def test_2020_profile_300_aliases(self, catalog: PartsCatalog) -> None:
        """Test 2020 profile 300mm variant aliases."""
        aliases = [
            "profile_2020_300",
            "2020x300",
            "2020_x_300",
            "extrusion_2020_300",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "profile_2020_300", f"Failed to resolve '{alias}' to profile_2020_300"

    def test_generic_profile_aliases(self, catalog: PartsCatalog) -> None:
        """Test generic profile/extrusion aliases default to 2020."""
        aliases = [
            "profile",
            "extrusion",
            "aluminum_extrusion",
            "alu_extrusion",
            "frame",
            "rail",
        ]
        for alias in aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved == "profile_2020", f"Failed to resolve '{alias}' to profile_2020"

    def test_case_insensitive_resolution(self, catalog: PartsCatalog) -> None:
        """Test that alias resolution is case-insensitive."""
        test_cases = [
            ("NEMA17", "nema17_body"),
            ("Stepper", "nema17_body"),
            ("MOTOR", "nema17_body"),
            ("M3X8", "hex_m3x8"),
            ("Bolt", "hex_m3x8"),
            ("NUT", "nut_m3"),
            ("BEARING_608", "bearing_608"),
            ("Sun_Gear", "gear_sun_nema17"),
            ("PLANET", "gear_planet"),
            ("Ring", "gear_ring_internal"),
            ("PROFILE", "profile_2020"),
        ]
        for alias, expected in test_cases:
            resolved = catalog.resolve_id(alias)
            assert resolved == expected, f"Failed to resolve '{alias}' to {expected}"

    def test_spacing_normalization(self, catalog: PartsCatalog) -> None:
        """Test that spacing and dash normalization works."""
        test_cases = [
            ("nema 17", "nema17_body"),
            ("nema-17", "nema17_body"),
            ("m3 bolt", "hex_m3x8"),
            ("hex-bolt", "hex_m3x8"),
            ("sun gear", "gear_sun_nema17"),
            ("planet-gear", "gear_planet"),
            ("v slot", "profile_2020"),
        ]
        for alias, expected in test_cases:
            resolved = catalog.resolve_id(alias)
            assert resolved == expected, f"Failed to resolve '{alias}' to {expected}"

    def test_unknown_alias_returns_none(self, catalog: PartsCatalog) -> None:
        """Test that unknown aliases return None."""
        unknown_aliases = [
            "unknown_part",
            "nema23",
            "m5x10",
            "bearing_9999",
            "invalid",
        ]
        for alias in unknown_aliases:
            resolved = catalog.resolve_id(alias)
            assert resolved is None, f"Expected None for unknown alias '{alias}', got {resolved}"

    def test_exact_part_id_match(self, catalog: PartsCatalog) -> None:
        """Test that exact part_ids resolve correctly."""
        exact_ids = [
            "nema17_body",
            "bearing_608",
            "bearing_mr128",
            "bearing_6709",
            "bearing_6704",
            "hex_m3x8",
            "hex_m3x12",
            "hex_m3x16",
            "nut_m3",
            "hex_m6x20",
            "hex_m8x25",
            "gear_sun_nema17",
            "gear_planet",
            "gear_ring_internal",
            "profile_2020",
            "profile_2020_300",
        ]
        for part_id in exact_ids:
            resolved = catalog.resolve_id(part_id)
            assert resolved == part_id, f"Exact part_id '{part_id}' should resolve to itself"

    def test_all_aliases_map_to_existing_parts(self, catalog: PartsCatalog) -> None:
        """Verify that all ALIASES map to parts that actually exist in the catalog."""
        from kala.parts.catalog import PartsCatalog as PC

        for alias, target_id in PC.ALIASES.items():
            resolved = catalog.resolve_id(target_id)
            assert resolved == target_id, (
                f"Alias '{alias}' maps to '{target_id}', "
                f"but '{target_id}' does not exist in catalog"
            )
