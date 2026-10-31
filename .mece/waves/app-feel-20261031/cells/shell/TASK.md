# Cell shell: app frame (top bar, collapsible chat, Details panel, status bar, single tool rail)

Read `../../WAVE.md` first. It holds the decisions, the contract and the rules. Look at `../../before/01-main.png` and `05-design.png`.

## Build
1. **`components/shell/TopBar.tsx` (new), 44px, `role="banner"`.**
   - **Left:** a chat toggle (`aria-label="Toggle chat"`, `aria-pressed`; dispatches `chrome/chat-toggle`) and the wordmark `KALA // STUDIO`, using the existing brand styling from the chat header.
   - **Centre:** the render-mode segmented control (2D Pencil / 3D Clay), the ONLY copy. Move the logic from the sheet pill. `aria-label="Render mode"`; its buttons are labelled `2D Pencil` and `3D Clay`.
   - **Right, three buttons:**
     - `Details`: a toggle with `aria-pressed`; dispatches `chrome/details-toggle`.
     - `Export`: dispatches `chrome/export-open`.
     - A gear: `aria-label="Settings"`; dispatches `options/open`.
2. **`AppShell.tsx`.** The grid is: top bar / [chat | sheet | details?] / status bar.
   - The chat column is 360px and is removed from layout (not just hidden) when `chatOpen` is false.
   - The Details column is 360px and is rendered only when `detailsOpen` is true. It sits in the grid and does not overlay the sheet.
   - Mount `<OptionsDrawer>` (unchanged props) and `<ExportDialog open={exportOpen} onClose={() => dispatch({type:'chrome/export-close'})} />` from `../export/ExportDialog`. Cell settings creates that file. If it does not exist yet, write a 5-line placeholder with exactly that signature, and say so in your REPORT. Cell settings will overwrite it.
   - Keep the mesh-stability memo and the single `designController` exactly as they are.
3. **`components/shell/DetailsPanel.tsx` (new).**
   - `role="complementary"`, `aria-label="Details"`.
   - Tabs (`role="tablist"`): `Properties`, `Views`, `Activity`. They bind to `detailsTab` and `chrome/details-open {tab}`.
   - A close button (`aria-label="Close details"`).
   - **Properties.** First, the active-geometry summary (rows DIMS / ENVELOPE / VOLUME / SOURCE, with the same data and the same "measured by …" honesty). Move that logic out of ChatPanel: copy it from `git show HEAD:studio/src/components/chat/ChatPanel.tsx`, the `chat-panel-context` section and its helpers, into a new `components/shell/ActiveGeometrySummary.tsx`. Cell composer deletes it from the chat. Below that, when no design is active, `<Inspector showHeading={false} />` (the primitive sliders plus surface material).
   - **Views.** The existing `<AuxView …>`, with the same props AppShell passes today.
   - **Activity.** The existing `<JournalPanel …>`, with the same props.
   - Adapt AuxView/JournalPanel CSS only as much as needed to fit a 360px column.
4. **Status bar (`BottomBar.tsx`).**
   - Make it read-only, 28px.
   - Remove the 2D/3D buttons, the icon toggles (grid/axes/dims/section/reset), Export and ⚙ Options.
   - Keep the readout (ENV/VOL/AREA/SCALE + source badge, unchanged) and the FreeCAD status dot.
   - Add on the left a connection label (`Connected · Desktop` or `Connected · Browser fallback`, otherwise the real state), as a button that dispatches `options/open {card:'connection'}`.
   - Add on the right the backend version from the store.
   - Change the component's props as you need, since AppShell is yours.
5. **`DraftingSheet.tsx`.**
   - Remove the render-mode pill (it moved to the top bar).
   - Add an `Axes` toggle to the tool rail, using the same icon-button pattern; it dispatches `viewport/set {showAxes}`.
   - The rail is now the single home of reset, dims, measure, section, grid and axes.
   - Keep the design tabs, the title block and everything else.
6. **Keyboard.**
   - Esc closes the Details panel when focus is inside it.
   - `Ctrl/Cmd+,` opens Settings.
   - `Ctrl/Cmd+B` toggles the chat.
   - Do not capture keys while a text field is focused.

## Tests you own
- Update `AppShellMeshStability`, `BottomBar`, `BottomBarDesignReadout`, `DraftingSheet`, `MeasureTool`, `AuxView`, `journalLedger`, `previewStatus`, `SurfaceMaterial` and `VernierSlider` to the new homes.
- Add `test/TopBar.dom.test.tsx` covering: the toggles dispatch the right actions, `aria-pressed` reflects the state, and the render mode switches.
- Add `test/DetailsPanel.dom.test.tsx` covering: the tabs, close, Properties with and without a design, and that Views/Activity render AuxView/JournalPanel.

## Done when
- No type errors in your files, and your tests are green.
- `shots/idle-1440.png`: idle, with Details closed.
- `shots/details-1440.png`: Details open on Properties.
- `shots/slate-1440.png`: the slate theme. Set `localStorage['kala-studio.theme']='slate'`; shot.mjs takes a theme arg.
- REPORT.md is written.
