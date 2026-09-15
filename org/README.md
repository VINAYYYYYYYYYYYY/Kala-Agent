# Kala Org — how to run the company from one bot

You talk to **one Orchestrator**. It routes to the full roster.

## Grok (4 slots) — paste from `org/grok/`

| Slot | File | Role |
|------|------|------|
| 1 | `grok/01_ORCHESTRATOR.txt` | Only daily chat — routes everything |
| 2 | `grok/02_CEO.txt` | Strategy / priorities |
| 3 | `grok/03_CTO.txt` | Agent, CAD, quality |
| 4 | `grok/04_CFO.txt` | Money, tokens, runway |

In Grok: **Settings → Customize → Create Agent** → paste Name + Personality + Instructions (≤4000 chars).

## Full roster — `org/roles/`

Specialists the Orchestrator delegates to (not all need a Grok face):

- Product/CAD: `cad_lead`, `design_qa`, `procedures`, `parts_librarian`, `eval_ops`, `ux`
- Growth: `cmo`, `content`, `social`, `web`, `community`, `sales`
- Ops: `bookkeeper`, `pricing`, `legal_advisor`, `support`

Each role file has: Mission · Inputs · Outputs · Hard limits · Handoffs.

## Company OS (shared artifacts)

| Path | Owner |
|------|--------|
| `designs/` | Procedures / Eval Ops |
| `outputs/eval/*/summary.json` | Eval Ops |
| `outputs/eval/*/rectify_queue.md` | CTO / CAD Lead |
| `outputs/eval/*/runs/<id>/` | Design QA |
| `org/STATUS.md` | Orchestrator updates |

## Rule

Social/Web drafts need **Founder approve** before publish.  
CFO never spends. Legal is advice only.
