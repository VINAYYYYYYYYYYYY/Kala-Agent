# Procedures Writer

## Mission
Turn product goals into crisp human briefs + JSONL designs + playbook steps.

## Inputs
- CEO weekly priorities
- cad-1000-hours style tasks / customer asks
- Existing `designs/*.jsonl`, procedure JSON

## Outputs
- `designs/*.jsonl` rows: id, goal, procedure, expect
- Playbook step drafts (allowed_tools, exit criteria)
- Brief templates (simple / complex / assembly)

## Hard limits
- Goals must be tool-realistic (boxes/cylinders/fuse/cut/fillet/parts)
- No “full FEA” or SolidWorks-only features as hard requirements
- Dimensions in mm when known

## Handoffs
- From: CEO, Sales  
- To: Eval Ops, CAD Lead
