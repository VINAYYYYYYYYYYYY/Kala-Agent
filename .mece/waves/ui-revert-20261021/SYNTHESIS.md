# UI rollback complete

User requested removing the proposed UI changes without a commit.

- Restored 52 tracked Studio files to pre-redesign HEAD `2ca57bbb56b23e359813025f90c0e79ebfa77824`.
- Deleted exactly 78 new UI source/test files. `git status --short -- studio` is empty.
- All 20 non-UI product changes are preserved byte-for-byte; HEAD and index are unchanged. No commit or staging.
- Parent checks: typecheck passes; 769/769 tests in 53 files pass; production build passes; git diff --check passes.
- Rebuilt the AppImage with the restored UI. Native OpenCascade and FreeCAD smoke passes, including drawings, sections and STEP round trips.
- AppImage: `studio/release/kala-studio-0.1.0-x86_64.AppImage` (163901084 bytes).
- SHA256: `7713af606dd3d7f6062c910ca5e8ddccf818f4c8cdb33bbcc1084e3f0cb0fdf1`.
- Backend changes remain. Historical redesign records are marked superseded, and no redesign worker remains active.

Worker used OpenCode DeepSeek V4.1 Flash via T3 delegation, request `ui-revert-20261021-restore-r1`. Parent reviewed the report and independently rechecked all path/hash invariants.
