"""Tests for softened unknown_part_id failure and search fixtures."""

import pytest

from kala.parts.catalog import PartsCatalog


class MockBackend:
    name = "mock"

    def create_cylinder(self, *a, **k):
        from kala.cad.protocol import ToolResult
        return ToolResult(ok=True, message="", data={"body_id": "cyl"})

    def create_box(self, *a, **k):
        from kala.cad.protocol import ToolResult
        return ToolResult(ok=True, message="", data={"body_id": "box"})

    def boolean_cut(self, *a, **k):
        from kala.cad.protocol import ToolResult
        return ToolResult(ok=True, message="", data={"body_id": "cut"})

    def translate(self, *a, **k):
        from kala.cad.protocol import ToolResult
        return ToolResult(ok=True, message="")

    def rotate(self, *a, **k):
        from kala.cad.protocol import ToolResult
        return ToolResult(ok=True, message="")


class TestUnknownPartSoftened:
    @pytest.fixture
    def catalog(self) -> PartsCatalog:
        return PartsCatalog.default()

    def test_unknown_part_message_is_short_and_has_suggestions(self, catalog: PartsCatalog) -> None:
        result = catalog.insert(MockBackend(), "no_such_part_xyz")
        assert result.ok is False
        msg = result.message
        assert "Unknown part_id" in msg
        assert len(msg) < 250
        assert "search_parts" in msg.lower()
        assert "Known:" not in msg
        data = result.data
        assert "suggestions" in data
        assert isinstance(data["suggestions"], list)



    def test_resolve_rejects_false_substring_hits(self, catalog: PartsCatalog) -> None:
        """Kill over-eager substring resolve (e.g. 'ring' inside 'bearing_9999')."""
        assert catalog.resolve_id("bearing_9999") is None
        assert catalog.resolve_id("nema23") is None

    def test_insert_suggestions_are_existing_ids_only(self, catalog: PartsCatalog) -> None:
        result = catalog.insert(MockBackend(), "bearing_9999")
        assert result.ok is False
        for sid in result.data["suggestions"]:
            assert sid in catalog._parts


class TestSearchFixtures:
    @pytest.fixture
    def catalog(self) -> PartsCatalog:
        return PartsCatalog.default()

    def test_search_by_keyword(self, catalog: PartsCatalog) -> None:
        result = catalog.search("bolt")
        assert result.ok is True
        ids = [p["part_id"] for p in result.data["parts"]]
        assert any("hex_m3" in i for i in ids)

    def test_search_by_category(self, catalog: PartsCatalog) -> None:
        result = catalog.search("bearing")
        assert result.ok is True
        ids = [p["part_id"] for p in result.data["parts"]]
        assert "bearing_608" in ids

    def test_search_by_part_id(self, catalog: PartsCatalog) -> None:
        result = catalog.search("hex_m3x8")
        assert result.ok is True
        ids = [p["part_id"] for p in result.data["parts"]]
        assert "hex_m3x8" in ids

    def test_search_empty_query(self, catalog: PartsCatalog) -> None:
        result = catalog.search("")
        assert result.ok is True
        assert len(result.data["parts"]) == len(catalog._parts)

    def test_search_no_match(self, catalog: PartsCatalog) -> None:
        result = catalog.search("xyznonexistent")
        assert result.ok is True
        assert result.data["parts"] == []

    def test_search_only_real_ids(self, catalog: PartsCatalog) -> None:
        result = catalog.search("gear")
        for p in result.data["parts"]:
            assert p["part_id"] in catalog._parts
