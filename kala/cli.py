"""CLI entry point for Kala CAD design agent."""

import argparse
import json
import os
import sys

from kala.agent.loop import run_agent


def main() -> None:
    """CLI entry point supporting KALA_CONTEXT_MODEL env var."""
    parser = argparse.ArgumentParser(
        description="Kala CAD design agent",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Environment Variables:
  KALA_CONTEXT_MODEL  Context model selection (default: stub)
                      Options: stub, learned
                      
                      stub   - No-op fallback (default)
                      learned - Rule-based enrichment from artifacts
        """,
    )
    parser.add_argument(
        "--backend",
        default="mock",
        choices=["mock", "cadquery"],
        help="Backend for CAD operations (default: mock)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Output agent state as JSON",
    )
    
    args = parser.parse_args()
    
    context_model_type = os.environ.get("KALA_CONTEXT_MODEL", "stub")
    
    print(f"Kala Agent starting...")
    print(f"  Backend: {args.backend}")
    print(f"  Context Model: {context_model_type}")
    
    try:
        agent = run_agent(backend=args.backend)
        
        if args.json:
            output = {
                "status": "initialized",
                "backend": args.backend,
                "context_model": context_model_type,
                "state": agent.state,
            }
            print(json.dumps(output, indent=2))
        else:
            print("Agent ready. Use agent.step() to process commands.")
            
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
