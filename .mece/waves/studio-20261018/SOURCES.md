# Integration research notes

Checked October 18, 2026 against primary project/vendor documentation. These are API integration directions, not a claim that a native connector has been implemented or tested.

- Official authentication documentation: https://developers.openai.com/codex/auth (redirects to https://learn.chatgpt.com/docs/auth). Codex supports ChatGPT sign-in and separately billed API credentials; use the official CLI flow rather than extracting browser/session credentials.
- Noninteractive Codex: https://developers.openai.com/codex/noninteractive . `codex exec` defaults to read-only; never broaden its permissions implicitly for Studio chat.
- OpenCascade.js project: https://github.com/donalffons/opencascade.js . OpenCascade compiled to WebAssembly is a possible browser kernel. Current first wave uses optional native OCP for STEP and deterministic analytic triangulations for preview, not an already-integrated browser B-rep kernel.
- Autodesk Fusion: https://help.autodesk.com/cloudhelp/ENU/Fusion-360-API/files/WelcomeToSDK_UM.htm . Native operations need the host's supported API/add-in environment; standalone Python must not pretend its imports are Fusion runtime APIs.
- SOLIDWORKS: https://help.solidworks.com/2026/english/api/sldworksapiprogguide/GettingStarted/SolidWorks_API_Getting_Started_Overview.htm . Native add-ins/automation require the appropriate installed product and SDK. Neutral STEP interchange is a separate capability.
- Solid Edge: https://support.industrysoftware.automation.siemens.com/trainings/se/106/api/ConnectingToSolidEdge.html . Siemens's publicly indexed SDK documents `SolidEdge.Application` automation. This older SDK is architectural evidence, not compatibility evidence for the user's current installed version.
- AutoCAD: https://help.autodesk.com/cloudhelp/2024/ENU/OARX-DevGuide-Managed/files/GUID-390A47DB-77AF-433A-994C-2AFBBE9996AE.htm . Host .NET/ObjectARX APIs and neutral-format support are not interchangeable; edition and format restrictions need verification before enabling handoff claims.
- Ansys Mechanical: https://mechanical.docs.pyansys.com/version/stable/architecture.html . PyMechanical is a supported automation direction, but requires a real installed/licensed Mechanical runtime.
- COMSOL: https://doc.comsol.com/6.3/doc/com.comsol.help.comsol/comsol_api_intro.46.01.html . Official Java API and LiveLink for MATLAB are documented; do not invent an official Python API or universal standalone solver access.
- OptiStruct: https://help.altair.com/hwsolvers/os/topics/solvers/os/run_options_os_r.htm . Native execution consumes solver input decks with appropriate controls and licensing, not STEP geometry alone.
- Simcenter Femap/Nastran: https://www.siemens.com/en-us/products/simcenter/mechanical-simulation/femap/femap-nastran/ . CAD import/preprocessing and analysis decks are distinct from the solver. STEP is not a ready-to-run Nastran deck.
- OpenRouter: https://openrouter.ai/docs/api/api-reference/chat/send-chat-completion-request . Chat completions use the fixed HTTPS endpoint; keys stay in the backend environment and remote prompting requires explicit consent.

No market-share ranking was established. The five analysis targets are a representative set requested for interoperability, not a verified global top-five list.
