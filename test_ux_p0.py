"""Test script to verify UX P0 trio implementation."""
import sys
from pathlib import Path

# Add parent directory to path for imports
sys.path.insert(0, str(Path(__file__).parent))

from kala.llm.providers import looks_like_api_key
from kala.ui.desktop import STARTERS


def test_looks_like_api_key():
    """Test API key validation."""
    print("Testing looks_like_api_key...")
    
    # Valid keys
    assert looks_like_api_key("sk-proj-1234567890abcdefghij1234567890abcdefghij")
    assert looks_like_api_key("x" * 25)  # Long enough generic key
    
    # Invalid keys
    assert not looks_like_api_key(None)
    assert not looks_like_api_key("")
    assert not looks_like_api_key("   ")
    assert not looks_like_api_key("short")
    assert not looks_like_api_key("https://api.openai.com")
    assert not looks_like_api_key("http://example.com/key")
    assert not looks_like_api_key("www.example.com/api")
    
    print("✓ looks_like_api_key tests passed")


def test_starters():
    """Test starter prompts."""
    print("\nTesting STARTERS configuration...")
    
    # Check L-bracket is first
    assert len(STARTERS) >= 1, "STARTERS should have at least one entry"
    first_starter = STARTERS[0]
    
    assert first_starter["label"] == "L-bracket", "First starter label must be 'L-bracket'"
    
    expected_prompt = "L-bracket base 60x40x4 and vertical wall 60x4x50 at origin, fuse, two Ø5 base holes at (12,20) and (48,20), export L_bracket.step"
    actual_prompt = first_starter["prompt"]
    
    assert actual_prompt == expected_prompt, f"L-bracket prompt mismatch:\nExpected: {expected_prompt}\nActual: {actual_prompt}"
    
    print("✓ L-bracket is first starter with exact golden brief")
    print(f"✓ Total starters: {len(STARTERS)}")


def test_error_sanitization():
    """Test error message sanitization."""
    print("\nTesting error sanitization...")
    
    from kala.ui.desktop import KalaDesktopUI
    from PyQt6.QtWidgets import QApplication
    
    app = QApplication(sys.argv)
    ui = KalaDesktopUI()
    
    # Test sanitization of API keys
    test_cases = [
        ("Error: sk-proj-abc123def456ghi789jkl012mno345pqr678stu901vwx234", "[REDACTED]"),
        ("Authorization: Bearer sk-1234567890abcdefghij", "[REDACTED]"),
        ("api_key=sk-test1234567890abcdefghij", "[REDACTED]"),
        ("Normal error message", "Normal error message"),
    ]
    
    for input_text, expected_pattern in test_cases:
        result = ui._sanitize_error(input_text)
        if expected_pattern == "[REDACTED]":
            assert "[REDACTED]" in result, f"Failed to redact: {input_text}"
            assert "sk-" not in result or result.count("sk-") == result.count("[REDACTED]"), f"API key leaked: {result}"
        else:
            assert result == expected_pattern, f"Unexpected sanitization: {result}"
    
    print("✓ Error sanitization tests passed")


def main():
    """Run all tests."""
    print("=" * 60)
    print("Kala UI UX P0 Trio Verification")
    print("=" * 60)
    
    try:
        test_looks_like_api_key()
        test_starters()
        test_error_sanitization()
        
        print("\n" + "=" * 60)
        print("✅ ALL TESTS PASSED")
        print("=" * 60)
        print("\nVerification summary:")
        print("1. ✓ Provider gate: no API key detection works")
        print("2. ✓ L-bracket starter: first chip with exact golden brief")
        print("3. ✓ Humanized _fail: error sanitization prevents key leaks")
        
    except AssertionError as e:
        print(f"\n❌ TEST FAILED: {e}")
        sys.exit(1)
    except Exception as e:
        print(f"\n❌ ERROR: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
