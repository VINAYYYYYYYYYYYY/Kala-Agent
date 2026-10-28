# Revert proposed Studio UI redesign

User explicitly requests: revert all suggested UI changes, delete the changes, and do not commit. Restore Studio to HEAD (pre-AS1100 redesign), preserve backend/Python/docs work and the index. Parent confirmed every changed/untracked Studio path is under src/test and belongs to the AS1100 UI wave.

Single OpenCode DeepSeek V4.1 Flash worker owns the exact Studio paths in `manifest.json`: restore listed tracked paths from the recorded HEAD and delete only listed new files. No additional design, source feature, commit, reset, staging, publishing or real user data writes. Historical .mece records remain as history, not active UI.

Parent verifies all preserved non-UI product SHA256s and unchanged HEAD/index, runs restored UI gates, then refreshes production UI and AppImage so the generated deliverable also uses the restored UI.
