# Mixer: Project Specification

Mixer was called Low-End Modeler until 2026-11-08, when the owner renamed it. Older entries below keep the old name where they describe what happened then.

Read this whole file before writing code. It records the decisions made so far, the scope, and how to work with the project owner.

Status: **Stage 10 done (0.15.0): small fixes, the Mixer Launcher with updates, the settings page, Blender's bevel options, rendering engines (Vulkan, DirectX 12, OpenGL 3.3 beside OpenGL 2.1), dockable panels, themes, and feature search. Next stage not yet chosen by the owner. The owner tries the app on the laptop and reports problems as they find them; the Roblox import test is on their own time. See section 11.**
Next step: **Stage 10 step 4: rendering engines (Vulkan, DirectX 12, OpenGL 3.3 through wgpu, beside OpenGL 2.1). Then step 5, feature search.**

## 1. Goal

Build a small, Blender-inspired desktop 3D modeler for very low-end Windows laptops. Its job is to create models and export them for use in games and other programs. It started out aimed at Roblox Studio, but on 2026-11-08 the owner said Mixer is not specialized to Roblox: the interface must not present it as a Roblox tool. Roblox stays one of the places exports should work (section 7).

It is not a Blender replacement. If a feature does not help someone model and export a game asset, it is out of scope.

## 2. Target hardware

| Item | Value |
|---|---|
| OS | Windows 10 Home 64-bit (build 19045) |
| Laptop | ASUS X55C |
| CPU | Intel Core i3-2328M, 2.2 GHz, 2 cores, 4 threads |
| RAM | 4 GB DDR3 (3.8 GB usable). 87% was already in use with a browser open |
| Disk | Hard disk (HDD), seen at 100% activity. Keep startup and disk access light |
| GPU | Intel HD Graphics 3000, OpenGL 3.1.0, driver 9.17.10.4459 |
| Screen | 1920 x 1080 |

Do not assume a graphics feature newer than OpenGL 2.1 is available.

## 3. Technical decisions

| Area | Decision | Reason |
|---|---|---|
| Language | Rust | Prevents many memory bugs, has fast native performance, and has simpler dependency management than C++ |
| Graphics | OpenGL 2.1 through `glow` | Suits old integrated graphics |
| Window and input | SDL2 | Small and widely used |
| UI | Dear ImGui through Rust bindings | Low overhead |
| App type | Native desktop app | Best performance, with no dependence on browser WebGL support |
| First export | OBJ | Very simple to write, and suits basic single meshes |
| Second export | glTF | Supports multiple meshes, hierarchies, and textures |
| FBX | Not planned | Roblox accepts OBJ and glTF, and FBX support is weaker in Rust |
| Build | GitHub Actions produces the Windows .exe | Compiling Rust on 4 GB of RAM may be slow |

Rejected options:
- **wgpu:** not suited to very old integrated graphics.
- **Rune (github.com/anomalyco/rune), suggested by the owner on 2026-11-07:** its Windows build draws only through Vulkan or DirectX 12 (via wgpu), and the laptop's HD Graphics 3000 supports neither: dxdiag shows feature level 10_1 and driver model WDDM 1.2. It would not open. It is also pre-release.
- **C++:** the main remaining reason to choose it was FBX support, which is not needed.
- **Python or Electron:** too slow or too memory-hungry for this hardware.
- **Browser app:** slower and dependent on browser WebGL support.

Before adding a dependency, check that it is still maintained and works with OpenGL 2.1. If the ImGui bindings or SDL2 setup cause problems, raise it with the owner before replacing them.

## 4. Performance rules

These are requirements, not optional polish.

1. Render only when something changes. The app should sit near 0% CPU when idle.
2. Use simple flat shading, with no shadows or anti-aliasing.
3. Update only the part of the mesh being edited, rather than rebuilding everything.
4. Keep memory use low, and cap undo history by memory used, not only by step count.
5. Avoid heavy dependencies. Keep the .exe small, ideally under 10 MB.

## 5. Mesh architecture

Vertices, edges, and faces refer to each other, which conflicts with Rust's ownership rules when written using direct references.

- Store mesh elements in arrays and link them by typed index, such as `VertexId`, `EdgeId`, and `FaceId`.
- Use a half-edge structure, or a similar one, that supports fast neighbor lookups. Bevel, loop cut, and loop selection depend on this.
- Support n-gons in the editing mesh. Triangulate only for rendering and export.
- Make sure deleted elements cannot leave stale indices that silently point at the wrong element. Generational indices are one option.
- Keep mesh code independent of graphics and UI so it can be unit tested without a window.
- Add a validation function that checks mesh consistency, and run it in tests after every operation.
- Design undo/redo at this stage, because every editing tool depends on it.

### Decisions made in Stage 1

The code is in `crates/mesh`. The top of `crates/mesh/src/lib.rs` explains the design in full.

- **Structure:** BMesh-style (Blender's layout), rather than a classic half-edge with twin pointers. It can store wire edges, loose vertices, three or more faces on one edge, and neighbors with mismatched winding. A twin pointer cannot, and normal modeling creates all of these.
- **Naming:** the half-edge is called a `Corner`, so it is not confused with edge loops and loop cuts.
- **Rules every mesh keeps:** an edge joins two different vertices; at most one edge joins any pair; a face has at least 3 corners and never repeats a vertex; no two faces use exactly the same vertices. Edits that would break these return an error and change nothing.
- **Stale IDs:** generation numbers are issued once and never reused, even after undo, so an old ID can never match a newer element.
- **Undo/redo:** storage saves an element's old value the first time an edit changes it. Tools therefore get undo without any undo code of their own. Memory use follows the size of the edit, not the size of the mesh.
- **History limits:** 64 MiB and 256 steps by default, dropping oldest steps first. The newest step is always kept, so it alone can exceed the byte limit.
- **Using it:** `Document` owns the mesh and gives out `&mut Mesh` only during an edit, so a change cannot bypass history. Use `Document::edit` for one-shot tools, and `begin_edit`, `commit_edit`, and `cancel_edit` for drags.
- **Built only as low-level edits so far:** add and remove, split edge, split face, join faces, flip face. User-facing tools and merge belong to their own stages.
- **Still to add when its stage arrives:** selection fields on elements (Stage 4), and a list of changed elements for partial redraws (Stage 3). History holds one mesh for now; a step must grow to hold several when multiple objects arrive.

### Decisions made in Stage 2

The app is in `crates/app`. The code is on GitHub: https://github.com/Smogi67/mixer (called low-end-modeler until 2026-11-08; the old address forwards). Each push to `main` builds the .exe with GitHub Actions.

- **Rendering:** GLSL 1.20 shaders and vertex buffers only, which are both part of OpenGL 2.1. All OpenGL calls are in `renderer.rs`.
- **Idle use:** the main loop sleeps until an event arrives, and draws only if that event changed something.
- **Axes:** Z is up, as in Blender. Roblox uses Y up, so Stage 3's export must convert this.
- **Camera controls (owner's choice, replaces Blender's here):** right drag turns the view and middle drag (holding the wheel) slides it, with no keys held. The wheel zooms, and Ctrl + middle drag also zooms. Plain left stays free for selecting in Stage 4. Alt + left drag turns the view on touchpads, and Alt + Shift + left drag slides it.
- **View keys:** F1, F3, and F7 or numpad 1, 3, and 7 give front, right, and top views (add Ctrl for the opposite side). F5 or numpad 5 switches to orthographic. Numpad 2, 4, 6, and 8 turn the view, and Home resets it.
- **Linking:** on Windows, the C runtime is built into the .exe. SDL2 was too, until Stage 8 moved it into SDL2.dll, which the Mixer Launcher installs (section 6).
- **Grid:** it lies on the ground, except in level views along an axis (such as front and right), where it stands upright, as in Blender.
- **Dear ImGui:** not added yet, because Stage 2 needs no UI. Check the bindings work with OpenGL 2.1 when the first UI is added.
- **Log file:** `mixer.log` (`low-end-modeler.log` before the rename) beside the .exe records the OpenGL details and any errors. The title bar shows the OpenGL version too.

### Decisions made in Stage 3

- **UI library:** the `imgui` crate (0.12, last released 2024) with `imgui_gl.rs`, a small OpenGL 2.1 renderer written here, and direct SDL event handling. The ready-made `imgui-glow-renderer` and `imgui-sdl2-support` crates need older `glow` and `sdl2` versions than this app uses.
- **Primitives and triangulation:** in `crates/mesh`, with faces wound outward. Triangulation uses ear clipping, which handles concave n-gons.
- **One mesh for now:** added shapes all go into one mesh, and undo is the only way to remove one until Stage 4. Separate objects are not built yet, so OBJ export writes one mesh.
- **OBJ export:** triangulated, one normal per face, and Z up becomes Y up by a rotation, so winding stays correct. Files go in an `exports` folder beside the .exe. A native save dialog can come in Stage 8.
- **Roblox limits:** export warns above 20,000 triangles per mesh and on open edges, but does not block. Back faces draw red, so inside-out faces are easy to see.
- **Adjusting the newest shape (owner's request):** the panel's values change the shape just added, like Blender's Adjust Last Operation, without adding undo steps. This stops after undo or redo.
- **ImGui labels:** every label in a window must be unique. A shared ID crashes the app when clicked, and `clicking_and_dragging_everywhere_on_the_panel_is_safe` in `ui.rs` guards against it.
- **Not done yet:** the whole mesh is re-sent to the graphics card after each change. This is fine for adding shapes, but Stage 4's transforms must send only what changed (section 4, rule 3), and must also use the list of changed elements noted in section 5.

### Decisions made in Stage 4

- **Selection:** stored as flags on elements, so undo restores it. A selected face always has its edges selected, and a selected edge its vertices. `validate` checks this.
- **Mouse:** left click selects, left drag can select with a chosen tool (box, circle, or lasso), Shift adds, and Ctrl removes. A click on nothing clears the selection. This follows the owner's wish to avoid extra keys for common actions.
- **Hidden parts:** depth is read from the graphics card once per selection, so only visible parts get selected. There is no select-through mode yet.
- **Transforms:** G, R, and S or panel buttons start a drag around the selection's center. X, Y, or Z locks an axis, Ctrl snaps (1 unit, 15 degrees, or 0.1 scale), a left click confirms, and a right click or Esc cancels. Duplicate is one undo step with its move, so cancelling removes the copy.
- **Mirror:** across the selection's center. Fully selected faces get their winding reversed, so they keep facing outward.
- **Partial updates:** during a transform only the affected faces, edges, and points are rewritten and sent (section 4, rule 3). Other changes, including selection, still rebuild everything.
- **Scheduling gaps:** proportional editing, the 3D cursor, other pivot points, and typing exact values have no stage yet. Add them to Stage 8 or 9 unless the owner says otherwise. Loop selection stays in Stage 6.

### Decisions made in Stage 5

- **Sliders follow the selection (owner's request):** the app remembers each added shape. If exactly one shape is touched by the selection and it has not been reshaped, the panel shows its values and changing them rebuilds it. A shape moved as a whole keeps this. A reshaped one, or one with anything attached, does not, because a rebuild would destroy that work. Replaces "only the newest shape" from Stage 3. Slider changes in a row share one undo step.
- **Selecting (owner's request):** the new default is Part mode (key 4), where a click selects a whole connected part. Left drag is now off by default, and Box, Circle, or Lasso can be turned on. A press that moves under 8 pixels counts as a click.
- **Tools:** in `crates/mesh/src/tools.rs`. Extrude keeps the selected faces as a region and moves along their average normal. Inset treats each face separately. Merge removes a face left with too few corners or crossing itself. Merging two faces onto the same vertices keeps one. Subdivide turns triangles into four, and larger faces into one quad per corner. Unselected neighbors gain a corner on each cut edge, so they stay whole.
- **Holes show red:** the inside of an open shape shows its back faces, which draw red by design.

### Interface redesign (owner's request, after Stage 5)

The owner found the default Dear ImGui look ugly. The UI library stays: Rune was checked and needs Vulkan or DirectX 12, which the HD 3000 lacks. Instead:

- **Layout:** a top bar (mode, drag tool, undo, redo, Export), a tool strip on the left, a settings panel on the right that is only as tall as its contents, a view gizmo, and a status bar. All are floating rounded panels over the 3D view, in `crates/app/src/ui.rs`.
- **Look:** dark grey with one orange accent, in `crates/app/src/theme.rs`. All colors are named there; change them there only.
- **Fonts:** Inter (text) and Lucide (icons), both under free licenses whose texts ship next to the fonts in `crates/app/assets/fonts`. They are cut down to the characters used and built into the .exe (about 95 KB). `scripts/make-fonts.sh` remakes them and explains how to add an icon. The font texture stays within 1024 × 1024, which a test checks.
- **View gizmo:** click an axis ball for that view, plus Home and orthographic buttons.
- **3D view:** a gradient background, a grid that fades out with distance instead of flickering, light from just above and left of the camera plus soft light from above, and points pulled slightly toward the camera so sloped faces do not cut them.
- **Settings for new shapes** are folded away while nothing is selected, since they then only affect the next shape added.

### Decisions made in Stage 6

The mesh code is in `crates/mesh/src/loops.rs`.

- **Edge loops follow Blender's rules:** straight through vertices with four edges; along a hole's border; along the outline of a face with more than four sides, such as a cylinder cap; and along loose edges. Elsewhere a loop stops, so a cube edge's loop is just that edge.
- **Selecting loops (no extra keys, owner's wish):** double-click an edge for its loop, and Shift + double-click to add one. In face mode this selects the band of faces across the edge. Blender's Alt + click is not used, because Alt + left drag already turns the view on touchpads. The Loop and Ring buttons grow selected edges in vertex and edge mode.
- **Loop cut (Ctrl + R or the tool button):** point at an edge to see the cut in yellow, scroll or press + and - for 1 to 32 cuts, and click to cut. The new loops then slide with the mouse. Esc or right-click only cancels the slide, keeping the cut in the middle, as in Blender. The number of cuts is remembered for next time. The cut needs a four-sided face beside the edge, and it stops at faces that are not four-sided, which gain a corner instead.
- **Edge slide (G twice, or the tool button):** selected edges slide along their neighboring edges, to either side, without leaving them. They must form loops or lines that do not branch. Ctrl snaps in tenths. Vertex slide is not built.
- **Bridge (tool button):** joins two loops with four-sided faces. With faces selected, their outlines are bridged and the faces removed, so two facing caps become a tube. Both loops must have the same number of edges and both be closed or both open. The faces wind like their neighbors, and the loops are lined up to avoid a twist. Loops with different edge counts are refused rather than guessed at.
- **Input during a drag:** the interface now also hears mouse button releases during drags, so no button stays stuck down when a drag starts from a button.

### Decisions made in Stage 7

The mesh code is in `crates/mesh/src/bevel.rs`, whose top comment explains the method.

- **Edge bevel only.** Selected edges are beveled; in face mode that means every edge of the selected faces, as in Blender. Vertex-only bevel and Blender's profile shape slider were added later at the owner's request, with a Bevel panel; see the progress log.
- **Using it (Ctrl + B or the tool button):** move the mouse away from where the bevel started to widen it, scroll or press + and - for 1 to 16 segments, Ctrl snaps the width to tenths, click to confirm, and Esc or right-click to cancel. The number of segments is remembered.
- **Shape:** more than one segment gives a round profile: an exact quarter circle on a right-angled edge. Where three beveled edges meet, as at a cube corner, the corner becomes a triangle of a sphere divided into small triangles, so a fully rounded cube stays convex. Other corners with three or more points get a flat face (one segment) or a fan of triangles around a raised center.
- **Width limit:** the width stops before any new point passes the middle of an edge it moves along, so the shape cannot fold over itself. This replaces Blender's "clamp overlap" option.
- **Refused:** edges without a face on each side (borders and loose edges), and vertices whose faces do not form one simple fan facing the same way. The message says why.
- **Drag speed:** every new point is its original vertex plus the width times a fixed direction, so dragging only moves points. Changing segments rebuilds the bevel.

### Decisions made in Stage 9, part 1 (owner's choices)

The owner asked for a free cut, for joining shapes ("merging"), and for a mode that hides the lines. Booleans were moved up from last on the later list at the owner's request.

- **Knife (K or the tool button):** click points across the model, Enter or a double click to cut, Backspace removes the last point, Z cuts hidden faces too, Esc or right-click cancels. The cut goes where the path crosses edges; clicks near a corner go through the corner, and other clicks only steer the path. Code: `crates/app/src/knife.rs` finds the crossings on screen, `crates/mesh/src/cut.rs` cuts.
- **Bisect (tool button):** drag a line across the selection; the selected faces are cut where the flat plane through that line and the view passes. Bisect's options to delete one side or fill the cut are not built.
- **Join and Separate (Ctrl+J, P, or the Combine buttons in Part mode):** joined shapes keep their geometry but count as one part: clicking one selects all, and glTF export makes them one MeshPart. Each face stores a group number (0 for none), saved in project files as `g` lines, which older versions skip. Tools that rebuild a joined shape's faces keep it in its group. Code: `crates/mesh/src/groups.rs`.
- **Union, Subtract, Intersect (Combine buttons in Part mode):** need closed shapes. Subtract cuts the shape clicked last out of each other selected shape. The method is csg.js's BSP trees, kept in flat arrays with no recursion so Windows' 1 MB stack cannot overflow; afterwards points are welded, edges get the points other faces split them at, so the result is closed, and faces in one plane are joined again. Code: `crates/mesh/src/boolean.rs`. In tests, 2,600 random combinations of cubes, spheres, cylinders and cones gave closed surfaces; while developing, 2 in 3,000 left a speck smaller than a thousandth of a unit where shapes only just touched.
- **M in Part mode** no longer squashes whole shapes: it explains how to merge points or join shapes instead.
- **View mode (Tab or the eye button at the top):** hides edges, points, selection color and handles; clicking does not select. Any editing action switches back to Edit mode.
- **Tool strip:** buttons shrink on short screens so all tools fit, down to 24 pixels.

### Decisions made in Stage 9, part 3: smooth shading (owner's request)

- **Flat, Smooth, Auto smooth** buttons under Selection set the shading of the selected faces. Each face stores 0 (flat) or an angle up to 180: it is lit smooth across edges gentler than that. Smooth is 180; Auto smooth is 30, as in Blender. Saved in project files as `s` lines; an OBJ file's smoothing groups (`s 1`) open as smooth.
- **Normals:** a smooth corner averages the faces around its point that are within the angle, weighted by face area and by the angle at that corner, so a big flat face next to a bevel stays nearly flat and the bevel looks round. Blender's Harden Normals and custom normals are not built.
- **Keeping it:** faces a tool makes take the shading of the faces they replace or touch, so bevelling, extruding or combining a smooth shape keeps it smooth.
- **Exports:** OBJ writes each different normal once (`vn`) and glTF one normal per corner, so Roblox shows the same shading.
- **Anti-aliasing check:** confirmed on Linux with an off-screen display limited to OpenGL 2.1: 4 samples, visibly smoother edges, and no OpenGL errors from Mixer's own calls.

### Stage 10 plan (owner's choices, 2026-11-08)

Asked to consult first, the owner chose this order:

1. **Small fixes:** every Quick Project gets its own folder (Quick Projects > name > name.mix, exports inside it); old quick projects stay where they are. The start window's Recent list gets Open folder and Delete; Delete asks first, then moves the project to the Recycle Bin. The "Start with a shape" text in the empty 3D view is removed completely.
2. **Mixer Launcher:** the setup program is renamed Mixer Launcher (`mixer-launcher.exe`) and becomes the way to start Mixer. It checks GitHub Releases for a newer version and **asks first** ("Update now? / Not now", with what is new); then downloads, verifies, replaces Mixer and starts it. Without internet it just starts Mixer. Each finished stage becomes a numbered GitHub Release, because Actions downloads need a login and Releases do not. New look, dark in Mixer's colors, one page at a time (check, advanced, progress, done). It must still need nothing that it installs. **A picture of the design goes to the owner for approval before building it.**
3. **Settings page** in Mixer with all four groups: Display (rendering engine, anti-aliasing, interface size 100/125/150 %, grid on or off), Mouse (touchpad mode with Alt+drag to turn and Shift+Alt+drag to slide, zoom toward the pointer, invert zoom, turn speed), Saving (auto-save every few minutes, undo steps, recent list length, start window on or off), Export and updates (default format and scale, open folder after export, check for updates on start).
4. **Rendering engine choice:** the owner wants it to work on PCs generally, not only the laptop, including Vulkan. Plan: keep today's OpenGL 2.1 renderer (the laptop supports nothing newer that wgpu can use) and add a second renderer built on wgpu offering Vulkan, DirectX 12 and OpenGL 3.3. Every engine is listed, even ones the PC seems not to support; those say "may be unsupported" beside them, and people may still pick them (owner's choice: freedom over safety). The launcher recommends one, and an advanced view lets people pick. If a picked engine fails to start, Mixer must fall back to OpenGL 2.1 and say so, never just fail to open. The same choice is in Mixer's settings. The owner accepted that the download grows by several MB. No software (Mesa) fallback.
5. **Feature search.**

Step 4 is done (version 0.13.0); see its decisions below.

### Decisions made in Stage 10, step 4: rendering engines (owner's choices)

- **Two renderers:** OpenGL 2.1 stays as it was (`renderer.rs`, `imgui_gl.rs`). Vulkan, DirectX 12 and OpenGL 3.3 come through wgpu 30 (`wgpu_renderer.rs`, `wgpu_shaders.wgsl`), drawing the same picture: same colors, lighting, grid fading, 5-pixel points, picking depth (drawn into a color image so every engine can read it back), anti-aliasing with 4 samples, and screenshots. `gpu.rs` lets the rest of Mixer use either. On Windows, DirectX 12 compiles shaders with Windows' own compiler, so no extra files are shipped.
- **Picking an engine:** Settings > Display and the launcher's Advanced page list all four, and any may be picked. Each is marked from what this PC offers: "Recommended", "In use", "May be unsupported" (missing, or not checked yet in the launcher) or "Software only, slow" (only Windows' or Mesa's software stand-in). It takes effect when Mixer next starts.
- **Finding out what a PC offers:** `mixer --check-engines` asks the drivers and writes Documents › Mixer › engines.txt. It runs as its own process, because a bad driver could crash it: Mixer starts it in the background when a new version first runs, and the launcher runs it (30-second limit) after an install, so the launcher can recommend before Mixer has run. OpenGL 3.3 counts only if the driver reports OpenGL 3.3, or OpenGL ES 3.0 on Linux.
- **Recommendation:** DirectX 12 on a graphics card that offers it (Windows' own engine, with the best-kept drivers there), else Vulkan, else OpenGL 2.1. OpenGL 3.3 is never first: where it works, OpenGL 2.1 does too, and it has been tried for longer. On the owner's laptop (Intel HD Graphics 3000) that is OpenGL 2.1.
- **Falling back:** if the chosen engine cannot start (no driver, refused, or the driver crashes while starting), Mixer opens with OpenGL 2.1; if that fails too, with DirectX 12, Vulkan or OpenGL 3.3, whichever starts first. It says so in the start window and status bar, writes the reasons to mixer.log, and Settings shows it. The setting is kept, so a fixed driver is used next time. Only if nothing starts does Mixer show OpenGL's error.
- **A software stand-in is allowed:** if only a software adapter offers the engine, Mixer still uses it (marked "software, slow" in the title bar and log), since the owner wants the freedom to choose.
- **Grid lines** are cut into 2-unit pieces for the newer engines: Mesa's drivers drew long lines reaching behind the camera as stray triangles.
- **Size:** mixer.exe grows by several MB (accepted by the owner when choosing this).

### Decisions made in Stage 10, step 5: feature search

- **Ctrl+F** (and File > Search features) opens a search box at the top of the 3D view, like Blender's F3 menu. F3 stays the right view, as Mixer has used it for views on laptops without a number pad since Stage 3; changing it would break the owner's habit.
- About 70 commands: every tool, shape, selection command, mode, shading, view, file command and the settings. Each has extra words people might type ("grab" finds Move, "theme" finds Settings) and shows its shortcut. Typed words must all appear; names starting with the text come first.
- Enter or a click runs the chosen command exactly as its button or key would; the arrow keys choose; Esc or a click outside closes it.

### Docked panels, Blender-style bevel drag, themes (owner's requests, 2026-11-08)

- **Bevel again:** a Cutoff bevel with a stepped custom profile could leave cut faces with no area (all points in a line), and the next bevel refused the shape. Such a face now borrows the facing of the faces around it. Tests bevel a fully beveled cube again for every preset, segment count, intersection and width.
- **Two side panels** replace the fixed right panel and the floating Bevel window: *Add Shapes* (shapes, selection, combining, modifiers) and *Properties* (the last bevel's options, Blender's "Adjust Last Operation", and the shape settings). Both start docked on the right, stacked; the column's left edge and the gap between them can be dragged. Dragging a panel's title takes it out as a floating, resizable window; dropping it near the right edge (a highlighted outline shows where) docks it again; double-clicking the title switches. Their layout is not saved between runs yet. Because Properties always shows, the bevel options appear the moment the bevel is confirmed.
- **Bevel drag like Blender:** while the left button is held, the cursor is hidden and held still (SDL's relative mouse mode), so a drag never stops at the screen edge; it comes back where it was when the button is let go. Xvfb's simulated mouse cannot show real drag distances in this mode, so this is checked by hand on the laptop.
- **Themes:** Settings > Theme lists all 30 colors (interface, 3D view, shapes) with a color picker each, a Reset per color and Reset to the default theme. Changes show at once, including the grid, background, faces, edges and selection, in all engines, and are kept in Documents › Mixer › theme.txt (`name #rrggbbaa` lines; the default theme removes the file). The selected-face color's opacity is how strong the tint is.

### Bevel by dragging, and one Edit/View button (owner's request, 2026-11-08)

- Ctrl+B (or the Bevel button) no longer makes the bevel follow the mouse straight away. It waits, with "Hold left click and drag" in the hint; the width follows the drag from where the button went down, and letting go confirms. Scrolling still changes segments; Esc or right-click cancels; Enter before any drag cancels too, as a bevel of no width does nothing.
- The two Edit and View buttons in the top bar are now one button showing the current look ("Edit" or "View"); clicking it switches. Tab still does the same.

### Materials (owner's choices, 2026-11-09)

- **What a material is:** a name, a color with opacity, Metal and Roughness (Blender's Principled BSDF basics). Each face uses one material or none ("Default look", the theme's face color).
- **One list per project**, not Blender's slots per object: Mixer's parts are pieces of one mesh, so a per-part list would only add steps. Up to 256 materials.
- **Materials panel**, a third dockable panel (docked between Add Shapes and Properties): the list with a swatch and face counts, New and Remove, Name, Color, Metal and Roughness, and Assign, Select and Deselect as in Blender. Editing shows at once; letting go makes one undo step.
- **Tools keep materials:** new faces take a neighbor's material (extrude, inset, loop cut, bevel, subdivide), and duplicate, mirror, array, join, separate and booleans keep each face's own. The bevel's **Material Index** option (under More options) gives the bevel's new faces a chosen material.
- **Saved in projects** as `mat` and `u` lines; the file says `mixer 2` only when it has materials, so files without them still open in older versions.
- **Exports:** OBJ gets a `.mtl` file beside it (Kd, d, and the PBR extension's Pm and Pr). glTF/GLB gets one primitive per material with metallic-roughness materials (colors turned from sRGB to linear), plus the same colors as vertex colors (COLOR_0).
- **Roblox (checked 2026-11-09):** the Importer makes one MeshPart per glTF primitive, so each material becomes its own MeshPart, and it has an "Ignore Vertex Colors" option, so it reads vertex colors. Its docs do not say whether it reads glTF material colors or OBJ `.mtl` colors; the owner's import test will tell. Sources: https://create.roblox.com/docs/en-us/studio/importer.md

### Not a Roblox tool (owner's request, 2026-11-08)

- Every "for Roblox" wording is gone from the interface and README: the start window says "3D modeling", the export button and popup just say Export, and scale is explained as "sizes are exported as they are".
- The triangle counter and export warnings stay, worded for game engines in general. The limit is a setting (Settings > Export & updates > Triangle limit): on by default at 20,000, any number from 100 to 10,000,000, or off, which leaves a plain count with no bar or warning. Stored as `trianglelimit` in settings.txt.

### Edge glow (owner's request, from a Blender screenshot)

- Selected and sharp edges are drawn twice more on top: a faint glow 7 pixels wide, then a bright 2.5-pixel core, like Blender's. Where glowing edges meet the glow adds up, so corners stand out (most visibly with the newer engines, which draw the glow as bands). Dark, unselected edges get none. Selected faces get a slightly lighter tint so the edges stand out on them. Both renderers share the colors and widths.

### Decisions made in Stage 10: Blender's bevel options (owner's request, before step 4)

The owner sent the Blender 5.2 manual page for Bevel and asked for what is on it. This replaces the Stage 7 decision that left these options out.

- **Width type:** Offset, Width, Depth, Percent and Absolute, as in Blender. Inside, every bevel is still worked out in Offset; each type gives a factor from its own unit to offset (Percent: the mean length of the edges the points slide along, over 100; Width and Depth: from the bevel's chord). Switching to or from Percent converts the number so the bevel stays about the same size.
- **Custom profile:** an editor in the panel. Drag points, click the line to add one (up to 24), right-click to remove, double-click for sharp or smooth. Curves through smooth points are Catmull-Rom; between two sharp points the line is straight. Small rings on the line show where the new edges go. Presets: Round, Chamfer, Support Loops, Steps, Cornice Molding, Crown Molding. Sample straight edges and Sample even lengths work as in Blender.
- **Miter outer** (Sharp, Patch, Arc), **Miter inner** (Sharp, Arc) and **Spread**, where two beveled edges meet at a corner whose two sides differ in angle. **Intersection:** Grid Fill (the rounded patch, as before) or Cutoff (each end cut off flat, with a flat face in the middle).
- **Loop slide** (on by default): off gives an even width instead of sliding along the edges.
- **Sharp edges:** edges can be marked sharp (Mark sharp and Clear sharp in Edge mode, or the bevel's Mark sharp option, which marks the bevel's borders). Smooth shading stops at them; they are saved in project files (`sharp a b` lines) and carried into exports through the normals. Sharp edges are drawn blue.
- The less common options are behind "More options" in the Bevel panel, so it stays short on the laptop's screen; the panel scrolls when it is taller than the 3D view.
- **Left for later**, because Mixer has nothing for them yet: Mark Seams (needs UVs), Harden Normals and Face Strength (need custom normals).
- **Shadows setting** (Settings > Display > Shadows on shapes, on by default): off lights every side the same. Asked whether that looks too flat in View mode, the owner chose to offer both: with shadows off, "Edge lines in View mode" (off by default, `flatedges` in settings.txt) draws thin plain edge lines in View mode, so the shape still shows.
- **Leftover launcher:** after an update, Mixer deletes `mixer-launcher.old.exe` when it starts (trying for up to 10 seconds while the old launcher closes), so it no longer waits for the launcher's next start. Owner's request.

### Decisions made in Stage 10, step 3: the settings page (owner chose all four groups)

- **Where:** a Settings window over everything, from the gear button in the top bar, File > Settings, Ctrl+comma, or the Settings button in the start window (which replaces the anti-aliasing checkbox there). Four pages on the left. Every change is saved to settings.txt straight away; Esc or Close closes it.
- **Display:** rendering engine (OpenGL 2.1 only until step 4; the others listed as coming), anti-aliasing (next start), show the grid, and interface size 100, 125 or 150 %. The interface is drawn bigger as a whole: Dear ImGui works in pixels divided by the size, and its letters are made that much bigger so they stay sharp. On a narrow window or a big size, the top bar drops the mode buttons' words and a shorter file name, and the Settings and Keyboard buttons give way rather than cover other buttons.
- **Mouse:** touchpad mode (off by default): two-finger scrolling turns the view, with Shift it slides, with Ctrl it zooms. Zoom toward the mouse pointer (off by default, as in Blender), invert zoom, and turn speed 25 to 300 %. The wheel now also takes a touchpad's small steps when touchpad mode is off. Alt+drag for touchpads works as before, always.
- **Saving:** auto-save every 2, 5 (default) or 10 minutes, or off: it saves the open project, as Ctrl+S would, when there are unsaved changes and nothing is being dragged or asked; imported OBJ files are not saved over. Undo steps 16 to 1000 (default 256). Recent projects 3 to 20 (default 8). Start window on or off; off opens the most recent project, or the start window if there is none.
- **Export and updates:** default export format and scale for new projects, show the exported file in Explorer afterwards, and the launcher's update check (the same `updates` line the launcher uses).
- **settings.txt:** one "name value" line each; values out of range are brought into range; lines Mixer does not know are kept.

### Decisions made in Stage 10, step 2: the Mixer Launcher (owner approved the design picture)

- **Pages:** Home (version, update card or status, checks for SDL2, graphics and rendering engine, Start Mixer), Welcome (when SDL2 or Mixer is missing: what is needed, Install and start), Advanced (rendering engine list, the update-check setting, versions and folder, and Repair install, which since 0.10.1, at the owner's request, downloads the release of the installed version again and puts every Mixer file back, then SDL2; without a release for that version it uses the newest), and Working (each step with a tick, a real progress bar for downloads, a log behind Show details, Cancel until files are being replaced, Try again and Back after a failure). Enter and Esc do the obvious button. Tab between buttons is not built.
- **Drawing:** the launcher lays out its pages as shapes in `view.rs`, tested on Linux with the real fonts and drawable as SVG for review, and draws them with GDI+ in `gui.rs`, with Mixer's Inter fonts built in. Dark title bar where Windows offers it. Its own orange icon.
- **Rendering engines:** all four are listed. OpenGL 2.1 is the only one built, so the others say "Coming in a later update" and cannot be picked until step 4; then the "may be unsupported" rule applies. The choice is saved as a `renderer` line in settings.txt, which Mixer now keeps when it saves its own settings.
- **Updates:** the launcher asks `api.github.com/repos/Smogi67/mixer/releases/latest` (an "updates on/off" line in settings.txt; on by default). A newer version shows a card with the release notes' first three list items, Update now and Not now. Updating downloads `mixer-<version>-windows.zip` and its `.sha256` from github.com only, checks size and fingerprint, unpacks, refuses any path outside the folder or any file but Mixer's own, checks that `version.txt` matches, writes every file beside its old one first, replaces mixer.exe first (if Mixer is open, Windows refuses and nothing has changed), moves the running launcher aside as `mixer-launcher.old.exe` (deleted on the next start), writes `version.txt` last, then starts Mixer. Projects are never touched.
- **Releases:** when a push to main carries a version in Cargo.toml that has no release yet, GitHub Actions builds, tests, zips the release folder, and publishes it as release `v<version>` with its fingerprint and `dist/release-notes.md`; then it updates an old copy from it. So finishing a stage means raising the version and writing the notes. (Pushing tags from Claude's environment is not allowed, so the workflow makes the tag.) Mixer shows its version in the start window.
- **Graphics too old:** Mixer now says so in plain words instead of an internal error (found in the Actions screenshots, where Windows has only OpenGL 1.1).

### Decisions made in Stage 9, part 4: mirror and array modifiers (owner left the choice to Claude)

- **Where:** in Part mode, with one shape clicked, the Modifiers section of the panel has Mirror and Array buttons, then each modifier's settings in the order they run, a Remove button for each, and Apply.
- **Mirror:** across the middle of the world (X, Y and Z ticked separately), because Mixer has no objects with their own centers. Merge (on by default) welds points within 0.001 of the middle, and walls left between the halves disappear, so half a shape becomes one closed shape. A shape sitting across the middle stays whole instead of vanishing.
- **Array:** 1 to 64 copies in a row. Offset is in shape sizes along X, Y and Z, like Blender's relative offset: 1 puts copies side by side. Merge (off by default) welds copies where they touch.
- **Live:** the copies are drawn without lines or points, and follow every edit, also while dragging. The status bar's triangle count and the Roblox warnings count the copies, because exports (OBJ and glTF) include them. Applying makes the copies real faces.
- **Stored:** each modifier belongs to its shape's group (see Join). Projects save them as `m` lines (`m 3 mirror 1 0 0 0.001`, `m 3 array 2 1 0 0 off`). Undo covers adding, changing, removing and applying; dragging a slider makes one undo step.
- **With other tools:** faces a tool adds to a shape with modifiers join its group. Duplicating a shape copies its modifiers, and Separate gives each piece its own copy. Join keeps the one shape's modifiers for the joined whole, but refuses two shapes that both have modifiers, and Union, Subtract and Intersect refuse shapes with modifiers, with a message to apply or remove them first. Deleting a shape deletes its modifiers. Changing a new shape's size or location in the panel keeps its modifiers and shading.
- **Not built:** Blender's other mirror and array options (mirror object, bisect, clipping, constant offset, fit length, start and end caps, UV options).
- Code: `crates/mesh/src/modifiers.rs`.

### Decisions made in Stage 9, part 2 (owner's choices)

- **Name:** the owner renamed the project **Mixer**: the programs are `mixer.exe` and `mixer-setup.exe` (since Stage 10 `mixer-launcher.exe`), the download is `mixer`, the crates are `mixer`, `mixer-setup` (now `mixer-launcher`) and `mixer-mesh`. The owner renamed the GitHub repository to `Smogi67/mixer`; the old address forwards.
- **Project files:** now `.mix`, starting `mixer 1`. Older `.lem` files still open. Projects also remember their export format, scale and file name (`export`, `scale` and `exportname` lines).
- **Start window:** shows every time Mixer starts, unless it was started with a file. Quick Project starts at once with a cube, saved as "Quick" plus the date and time in Documents > Mixer > Quick Projects. New Project asks for a name, a starting shape, and the export format and scale, and saves to Documents > Mixer > Projects > name > name.mix. A taken name is refused, not overwritten. Recent lists the last 8 projects opened or saved, with how long ago. Open other file, and Mixer folder (opens it in Explorer), are there too. File > Start window goes back to it.
- **Exports:** go to an Exports folder beside the project. Before a project is saved, to Documents > Mixer > Exports.
- **Anti-aliasing:** 4 samples per pixel, on by default, switched in the start window; it changes on the next start, because the window must be made with it. If the graphics driver refuses, Mixer starts without it and the start window says so. With it on, picking reads depth from an off-screen buffer, because reading depth from a multisampled window is not reliable on every driver. Settings live in Documents > Mixer > settings.txt. `MIXER_HOME` moves the Mixer folder, for testing.
- **Opening a project** now frames it with more room around it.

### Decisions made in Stage 8

- **Project files (`.lem`):** plain text in OBJ style, so they can be read and repaired by hand. They keep everything: faces with any number of corners, loose edges and vertices, and exact positions. Selection, shape records and camera are not saved. A newer format version is refused instead of misread. Code: `crates/mesh/src/project.rs`.
- **Open:** reads `.lem` projects, and imports `.obj` files from other programs (turning Y up into Z up). Broken faces are left out and counted. An imported OBJ is never overwritten by Save; Save asks for a new project name. Dropping a file onto the window, or onto the .exe, opens it.
- **Saving safely:** every file is written whole to a temporary file, then renamed, so a failure cannot damage the old file. Unsaved changes show as a dot after the file name, and New, Open and closing the window ask first. Selecting counts as a change, as in Blender.
- **Dialogs:** Windows gets its own Open and Save As dialogs. Elsewhere the app asks for a path in its own box, which is only used in development.
- **glTF export:** a single `.glb` file. Each separate part becomes its own mesh, so Roblox makes one MeshPart per part. It passes the official Khronos glTF validator with no errors or warnings. OBJ stays the default.
- **Shortcuts:** Ctrl+N, Ctrl+O, Ctrl+S, Ctrl+Shift+S and Ctrl+E added. The keyboard button in the top bar shows every shortcut.
- **Release zip:** GitHub Actions makes the `low-end-modeler` download with `low-end-modeler.exe`, `low-end-modeler-setup.exe`, `README.txt` and the font licenses. It also runs setup on Windows without a window, checks the app loads SDL2.dll late, and screenshots both programs into its log.

## 6. Feature scope

### Core

- **Primitives:** cube, cone, cylinder, UV sphere, icosphere, grid, circle
- **Selection:** vertex, edge, and face modes; click, box, circle, and lasso; loop, linked, and more/less
- **Transforms:** move, rotate, scale, mirror, duplicate
- **Vertex tools:** new edge/face from vertices, connect, slide, smooth
- **Edge tools:** bevel, bridge edge loops, subdivide, edge slide, loop cut and slide
- **Face tools:** extrude, inset, poke, triangulate, fill
- **Cleanup:** merge, split, separate, delete and dissolve, normals
- **Viewport:** orbit, pan, zoom, snapping, proportional editing, 3D cursor, pivot points
- **General:** undo/redo, Blender-style shortcuts, save/load, live triangle counter

### Dependency installer (owner's decision, do not change without asking)

The owner decided this on 2026-11-05.

- **Changed by the owner on 2026-11-08:** the installer became the **Mixer Launcher**, which also starts Mixer and updates it from GitHub Releases, asking first. See "Stage 10 plan" and "Decisions made in Stage 10, step 2". The rules below still hold for dependencies.
- **Delivery:** the release is one .zip that holds both the app's .exe and the launcher. Updates replace the files of that same zip, from a GitHub Release.
- **Purpose:** besides starting and updating Mixer, the launcher deals only with dependencies. Shortcuts, file associations and sample files are not part of it.
- **First screen:** a checkbox list of dependencies that shows which are required and which are optional. Required items cannot be unticked.
- **After pressing Continue:** a console-style view shows what is really happening, such as downloading with a progress bar, unzipping, and installing. Every step and every progress value must be real, and errors must be shown.
- **Honest list:** show only what the real .exe needs or can use. Do not add dummy entries. Skip or mark anything already present. If a dependency cannot be installed by a program, such as OpenGL support in the graphics driver, show it as a check with clear advice.
- **Must run first:** the installer itself cannot depend on anything it installs.
- **Downloads:** use official sources over HTTPS and verify each file before installing it.
- **The list (owner's decision, 2026-11-07):** asked whether to keep everything built into the .exe and make setup a system check, or to move SDL2 out so setup downloads it, the owner chose to download SDL2. So the list is: SDL2 (required, downloaded) and a graphics driver with OpenGL 2.1 (required, checked only; setup gives advice, since no program can install drivers). Nothing is optional: the C runtime stays built into the .exe.
- **How it is built (Stage 8, redrawn in Stage 10):** `crates/launcher` (was `crates/installer`), a separate .exe using only Windows itself: GDI+ for the window (Stage 8 used Windows' standard controls), WinHTTP for the download, and OpenGL for the check. SDL2 2.32.10 comes from SDL's GitHub releases, checked against SHA-256 fingerprints pinned in `core.rs` for the zip and for each file in it, then checked again by having Windows load the DLL. A file already in place with the right fingerprint is not downloaded again. A wrong one is replaced. Setup writes only `SDL2.dll` and `README-SDL.txt` (SDL's license notice, which it asks to ship with the DLL), into its own folder.
- **The app without SDL2.dll:** the .exe loads SDL2.dll late, checks for it first, and offers to start setup, instead of Windows showing a missing-DLL error.
- **Updating SDL2:** change the version, address, size and fingerprints in `crates/launcher/src/core.rs`. The test `the_pinned_sdl_fingerprints_match_the_real_download` checks them against a downloaded copy (set `SDL_ZIP`).

### Later

- Spin, screw, solidify, shrink/fatten, grid fill, convex hull
- Modifiers: solidify, bevel, decimate, weld, triangulate, and boolean (mirror and array are done, and the one-time boolean tools are done)
- UV seams, unwrapping, and UV editing, then the bevel's Mark Seams option
- Custom normals, then the bevel's Harden Normals and Face Strength options
- Mesh checks for bad geometry before export
- Feature search (owner's request, Stage 10 step 5): a search box, like Blender's F3 menu, that finds any tool or setting by name, shows its shortcut, and runs it

### Out of scope

Geometry Nodes, sculpting, multiresolution and remesh modifiers, physics, simulation, volumes, realistic rendering, video editing, metaballs, NURBS surfaces, and curves.

## 7. Roblox Studio export

Roblox Studio's Importer accepts `.obj`, `.fbx`, and `.gltf`. Roblox's docs also describe `.glb` import behavior.

- **OBJ:** best for simple single meshes.
- **glTF:** supports multiple meshes, hierarchies, and textures.

Export must:
- Triangulate faces automatically.
- Write correct normals and consistent face winding.
- Let the user set the export scale. Roblox's Importer has a setting for the units a file was modeled in, with Studs as the default.
- Show the triangle count before export.

Check Roblox's current mesh requirements before setting any triangle limit or warning. Do not hard-code a limit from memory.

Docs:
- https://create.roblox.com/docs/studio/importer
- https://create.roblox.com/docs/art/modeling/3d-importer

## 8. Build stages

Finish and test each stage before starting the next. **Aufwand** is the effort setting in the Claude app. See section 9.

| Stage | Work | Aufwand |
|---|---|---|
| 1 | Mesh structure, validation, undo/redo design, unit tests | Extra hoch |
| 2 | Window, OpenGL 2.1 viewport, camera, grid, GitHub Actions build | Hoch |
| 3 | Primitives, object display, OBJ export, test import into Roblox | Hoch |
| 4 | Selection modes and transforms | Hoch |
| 5 | Extrude, inset, merge, delete, fill, subdivide | Hoch |
| 6 | Loop selection, loop cut, edge slide, bridge | Extra hoch |
| 7 | Bevel | Extra hoch |
| 8 | Save/load, glTF export, shortcuts, UI polish, dependency installer, release .zip | Hoch |
| 9 | Later features, one at a time | Depends on the feature |

Small changes, such as a button or a label, only need Mittel. Keep Maximal for bugs that lower settings could not fix, because it uses at least 4x as much of the owner's limits.

## 9. Working with the owner

- **Effort setting:** The owner asked to be told when to change Aufwand. Say which setting to use **before** starting each stage. The AI cannot change it or see the current setting.
- **Testing:** The owner tests on the real laptop and reports back. Do not ask them to do unit testing or debugging by hand. Tell them exactly what to try for each build.
- **Owner's keyboard:** a compact 60% keyboard with no numpad, where F keys and Home need Fn. Every shortcut must work on it, so never make a numpad key the only way to do something.
- **Assumed experience:** Give exact, step-by-step instructions for anything done outside the app, such as GitHub setup. Do not assume programming experience.
- **Recommendations:** The owner wants recommendations based on technical merit, not on their preferences, unless they say otherwise.
- **Writing style:** Keep messages short, and lead with what they need to do or know. They often read on a phone.
- **Choices:** Whenever there is more than one suggestion to choose from, ask with a question prompt (clickable options), not in plain text (owner's rule, 2026-11-08).

## 10. Rules for AI contributors

1. Read this file first, then the current code.
2. Stay within the current stage. Do not start later features early.
3. Do not add out-of-scope features or newer graphics requirements without the owner's approval.
4. Add or update unit tests for every mesh operation.
5. Be honest about what has and has not been compiled or run. Never describe untested code as working.
6. When a bug report comes in, find the cause before changing code.
7. Update sections 11 and 12 at the end of each work session.

## 11. Progress log

| Date | Stage | What changed | Tested on laptop? |
|---|---|---|---|
| 2026-11-05 | Planning | Project spec written | Not applicable |
| 2026-11-05 | 1 | Mesh structure, validation, undo/redo, and 113 automated tests added. All pass on Linux, including a 450,000-step random test. Planted-bug testing: about 350 bugs tried; only 2 uncaught, and neither changes behavior. Not yet compiled for Windows, because the Windows target could not be downloaded in the build environment. Stage 2's GitHub Actions build must confirm this. | No. There is no app to run until Stage 2 |
| 2026-11-05 | Planning | Owner's dependency installer decision recorded in section 6 and added to Stage 8 | Not applicable |
| 2026-11-05 | Planning | Laptop details recorded in sections 2 and 12 | Not applicable |
| 2026-11-05 | 2 | Window, OpenGL 2.1 viewport, camera, grid, and GitHub Actions build added. Run on Linux with software graphics using simulated mouse and keyboard input: orbit, pan, wheel zoom, top view, and resizing all worked, and idle CPU measured 0.00% | Not yet |
| 2026-11-05 | 2 | First Windows build through GitHub Actions passed: all tests pass on Windows and the .exe is 3.01 MB. The first build failed because current CMake rejects the bundled SDL2's old version setting, fixed in the workflow | Not yet, waiting for the owner's test |
| 2026-11-06 | 2 | After the owner's feedback: right drag turns and middle drag slides with no keys held, and F keys added for views | Partly: the first build opened, and the owner has not yet reported the full test |
| 2026-11-06 | 2 | Laptop reports OpenGL 3.1.0, Build 9.17.10.4459, Intel HD Graphics 3000. The owner saw the picture stretch while dragging the window edge, because Windows pauses the main loop during a resize. Window events now draw from an SDL event watch. | Fix not yet confirmed |
| 2026-11-06 | 3 | Seven primitives, flat-shaded display with edges, tool panel, undo and redo buttons and shortcuts, triangle counter, and OBJ export added. Run on Linux with OpenGL limited to 2.1 using simulated clicks: all shapes, undo, redo, and export worked, and idle CPU measured 0.00% | Not yet |
| 2026-11-06 | 3 | The owner reported a crash when changing the Icosphere slider, and that changing values did not affect shapes. Reproduced the crash: the slider and the shape button shared a label, so a shared ID hit an out-of-range mouse button in Dear ImGui. Labels are now unique. Panel values now adjust the newest shape. Also fixed a null draw list on the first frame | Fix not yet confirmed |
| 2026-11-06 | 4 | Vertex, edge, and face selection with click, box, circle, and lasso; all, none, invert, linked, more, and less; move, rotate, and scale with axis lock and snapping; duplicate and mirror; partial graphics updates. Started at the owner's request before the Stage 3 tests were reported, so the Roblox import is still untested. Run on Linux with OpenGL limited to 2.1 using simulated input, and idle CPU measured 0.00% | Not yet |
| 2026-11-06 | 5 | The owner reported that sliders stopped working after selecting, and asked to select parts without dragging. Sliders now follow the selected shape, Part mode and click-only selecting added, and a slightly moved click no longer clears the selection. Extrude, inset, merge, delete, fill, and subdivide added, with a random test of all tools. Run on Linux with OpenGL limited to 2.1 using simulated input, and idle CPU measured 0.00% | Not yet |
| 2026-11-07 | 5 | Interface redesign at the owner's request: new layout, colors, fonts, icons, view gizmo, and a nicer 3D view. Run on Linux with OpenGL limited to 2.1 using simulated input; screenshots sent to the owner. A test clicks and drags over every point of the interface, including inside the popups | Not yet |
| 2026-11-07 | 6 | Edge loops and rings, loop selection by double-click and buttons, loop cut with a preview and slide, edge slide, and bridge. 21 new mesh tests, including a random test that ran each new tool 66 to 249 times and checked exact undo. Run on Linux with OpenGL limited to 2.1 using simulated input: loop cut preview, extra cuts, cutting, sliding, cancelling, double-click loop selection, and G G slide all worked | Not yet |
| 2026-11-07 | 7 | Edge bevel with width and 1 to 16 segments, round profiles, spherical cube corners, and a width limit. 13 new tests, including exact volumes for a flat cube-edge cut and a mitred loop, points lying on the true quarter circle and corner sphere, convexity of a rounded cube, and a random test of 310 bevels on six shapes that checked validity, holes, face direction and exact undo. Run on Linux with OpenGL limited to 2.1 using simulated input: drag, segment change, confirm, undo and cancel all worked | Not yet |
| 2026-11-07 | 8 | Project files, OBJ import, glTF export, File menu, shortcuts sheet, unsaved-changes prompts, and the setup program. 18 new tests, including exact project round trips and glTF read back for volume and winding; sample files passed the Khronos glTF validator. On GitHub's Windows machine, setup downloaded SDL2 in 0.3 seconds, matched both fingerprints, installed it, and Windows loaded it; a second run found it in place. Save, prompts and menus run on Linux with simulated input | Not yet |
| 2026-11-08 | 7 | At the owner's request: Blender-style handles (move arrows, rotate rings, scale handles), turned on and off with the Move, Rotate and Scale buttons, while G, R and S still work with the mouse alone. A Bevel panel after each bevel, like Blender's, with Affect (vertices or edges), Width, Segments, Shape and Clamp overlap; any change redoes the bevel. Vertex bevel added (Ctrl+Shift+B). Left out on purpose: material index, seams, sharp edges, normals options, width type, miter, intersection, custom profile and loop slide, which do not matter for a Roblox mesh. 7 new mesh tests, including an exact cube-corner volume. Run on Linux with OpenGL limited to 2.1 using simulated input: all three handles, axis drags, the view ring, every panel control, and undo worked, and idle CPU with handles showing measured 0.00%. Also fixed: after Ctrl+B, the interface thought Ctrl was still held, so clicking a slider started typing in it | Not yet |
| 2026-11-08 | 9 | Owner's choices: knife, bisect, join and separate, union, subtract and intersect, View mode, and M no longer squashing shapes in Part mode. 24 new mesh tests and 3 app tests, including exact volumes of combined cubes, a hole through a cube, and 150 random combinations that must stay closed (2,000 more ran while writing it). Run on Linux with OpenGL limited to 2.1 using simulated input: knife preview and cut, bisect, union, subtract, join, the M message, and View mode all worked | Not yet |
| 2026-11-08 | 9 | Renamed to Mixer at the owner's request. Start window with Quick Project, New Project, recent projects and settings. Anti-aliasing with a fallback when the driver refuses it. Projects remember their export settings. New tests for project settings, dates, ages, setting files and clicking through the start window. Run on Linux with OpenGL limited to 2.1 using simulated input: quick project, new project with a cylinder and glTF, the File menu's Start window, reopening from Recent, and picking through the off-screen depth buffer all worked. Anti-aliasing itself could not be seen there, because the test display offers no multisampled window; the fallback ran instead | Not yet |
| 2026-11-08 | 9 | Smooth shading at the owner's request: Flat, Smooth and Auto smooth, kept through tools, saved, and exported. 8 new tests, including a cylinder whose side normals point straight out and a smooth bevel whose big faces stay flat. Run on Linux with OpenGL limited to 2.1 using simulated input: a 10-segment bevel looked faceted flat and round with Auto smooth | Not yet |
| 2026-11-08 | 9 | Mirror and array modifiers, chosen by Claude when the owner left the choice open. 12 new mesh tests, including exact volumes of a mirrored box, a closed result from an open half, merged arrays, a centered shape mirrored on all three axes, exact undo, saving, export, and how join, separate, duplicate, delete, extrude and the combine tools treat modifiers; 1 new app test for resizing a new shape with a mirror; the interface test now clicks every modifier control. Run on Linux with OpenGL limited to 2.1 using simulated input: adding mirror and array, ticking an axis, one-step undo, copies following a move while dragging, saving the `m` lines, and Apply all worked. That run found that resizing a new shape lost its mirror, now fixed and tested | Not yet |
| 2026-11-08 | 10 | Step 1, small fixes: each Quick Project in its own folder; Show in folder and Delete beside each recent project, with a question first, moving the project's own folder (or only the file, for an older loose quick project or one elsewhere) to the Recycle Bin; deleting the open project means it is not saved again by mistake; the "Start with a shape" text removed. New tests for project folders and what Delete removes; the start window test clicks the new buttons. Run on Linux with simulated input: a quick project saved in its own folder, the empty view without text, and Delete with its question all worked. The Recycle Bin and Explorer only exist on Windows and are type-checked but not yet run | Not yet |
| 2026-11-08 | 10 | Step 2, Mixer Launcher: the owner approved the design picture; the setup program became the launcher with four custom-drawn pages and updates from GitHub Releases; version 0.10.0. 17 new launcher tests: JSON reading, versions, release parsing, refusing downloads off github.com, fingerprints and sizes, refusing paths outside the folder, replacing files with the launcher moved aside, a failed replace changing nothing, settings lines kept, job steps with stand-in downloads, and the layout of every page fitting the window with nothing overlapping. On GitHub's Windows machine: screenshots of the Welcome, Home (with an update card) and Advanced pages match the design; the Working page finished too fast to catch, so it has only been checked in the Linux preview; a real SDL2 install; release v0.10.0 published; and a real update of a copy pretending to be 0.0.1 from it, which replaced every file, moved the running launcher aside and kept SDL2.dll | Not yet |
| 2026-11-08 | 10 | Version 0.10.1, at the owner's request: Repair install also puts Mixer's own files back from the release of its version. 1 new test repairs a damaged Mixer with stand-in downloads; GitHub Actions repairs a damaged copy for real. Published as an update, so the owner's 0.10.0 launcher offers it | Not yet |
| 2026-11-08 | 10 | Step 3, settings page, version 0.11.0: the Settings window with Display, Mouse, Saving, and Export and updates; interface size; touchpad mode; zoom toward the pointer; auto-save; undo steps; start window off. 5 new tests: settings read back exactly, odd values brought into range with other lines kept, zooming at the pointer keeping the point under it, and clicking all over every settings page. Run on Linux with OpenGL limited to 2.1 using simulated input: every page, switching to 125 % (sharp letters, clicks and the 3D view still lined up, the top bar fitting), and an auto-save two minutes after adding a cone. Touchpad scrolling could not be tried without a touchpad | Not yet |
| 2026-11-08 | 10 | Before step 4, at the owner's request, version 0.12.0: Blender's bevel options (width types, custom profile with six presets and sampling options, miters with spread, Cutoff intersection, loop slide, mark sharp), sharp edges in general, and a shadows setting. 16 new tests: 11 for the bevel options (each width type's measured size, loop slide, the bevel's sharp borders, chamfer and round custom profiles matching the plain ones, every preset closed, both sides of a one-sided profile matching, miters, a cube cut off at all eight corners with every face flat, refusing a bad spread, and 540 combinations of options with exact undo), 4 for custom profiles, and 1 for sharp edges splitting smooth shading and being saved. The interface test now keeps the Bevel panel open so it clicks its controls. Run on Linux with OpenGL limited to 2.1 using simulated input: the panel with each profile type, typing a width and segments, the Steps and Cornice Molding presets, More options, Mark sharp, Cutoff, smooth shading with blue sharp borders, and shadows off (saved as `shadows off`). Miters were checked by tests only | Not yet |
| 2026-11-08 | 10 | Step 4, version 0.13.0: Vulkan, DirectX 12 and OpenGL 3.3 through wgpu beside OpenGL 2.1, with the engine check, recommendations, "May be unsupported" marks and falling back; at the owner's request, a glow on selected and sharp edges (both renderers), "Edge lines in View mode" for shadows off (the owner chose to offer both), and Mixer deleting the leftover old launcher. 11 new tests: engine names, support words and report files read back with sensible recommendations (app and launcher), driver version texts, the uniform layout, the WGSL shaders compiling, and every engine being pickable in both windows. Run on Linux with simulated input: Vulkan (Mesa's software Vulkan) through a whole bevel session with picking, Cutoff, smooth shading and View mode, looking the same as OpenGL 2.1; OpenGL 3.3 (Mesa's OpenGL ES); falling back from DirectX 12; the settings marks. That found two problems, both fixed: the interface's clipping cut the OpenGL 3.3 picture, and long grid lines turned into stray triangles. On GitHub's Windows machine: DirectX 12 started on Windows' software adapter and drew the start window; Vulkan and OpenGL 3.3, missing there, fell back as planned. No newer engine has run on a real graphics card yet | Not yet |
| 2026-11-08 | 10 | Version 0.13.1, at the owner's request: Roblox wording removed from the interface and README; the triangle limit became a setting (the owner chose both neutral wording and a setting). Tests updated for warnings with the limit on and off and for reading the setting. Run on Linux with simulated input: the start window and the new settings section | Not yet |
| 2026-11-08 | 10 | Version 0.13.2, at the owner's request: bevel only follows a held left-click drag; one Edit/View toggle button. Run on Linux with simulated input: after Ctrl+B, moving the mouse left the width at 0; a drag set it, letting go confirmed it and opened the Bevel panel; the toggle switched to View mode and back | Not yet |
| 2026-11-09 | 10 | Version 0.14.0, at the owner's requests: bevel-again fix, docked and floating Add Shapes and Properties panels, cursor held during bevel drags, a theme page for every color. New tests: bevelling again after every preset (2), theme files read back, the docked layout stacking inside the column. Run on Linux with simulated input: the bevel options showing as soon as the drag ends, taking Add Shapes out, moving it, the docking outline, docking it again, the theme page, and a light blue theme loaded from theme.txt | Not yet |
| 2026-11-09 | 10 | Step 5, version 0.15.0: feature search (Ctrl+F). 1 new test (matching and ranking, every name once). Run on Linux with simulated input: Ctrl+F opened the list, typing "sub" left Subdivide and Subtract, Enter subdivided the selected cube | Not yet |
| 2026-11-09 | 10 | Version 0.15.1, the owner's report again (bevelling a bevelled shape still failed): the round corner patch where three bevelled edges meet chose each small triangle's facing from its own shape, and on nearly flat corners, such as a cylinder's or cone's rim with many segments, some came out facing the other way. Now the whole patch turns together. New tests bevel again the way the app does (width 0 with the last settings, then moving the points) after every preset, segment count, intersection and width on one edge, a loop and a whole cube, and after round bevels of every shape on cubes, cylinders, cones and spheres | Not yet |
| 2026-11-09 | 10 | Version 0.16.0, materials (the owner chose color, metal and roughness; per-face assignment; saved in the project; their own panel): the Materials panel, the bevel's Material Index, tools keeping materials, project saving, OBJ+MTL and glTF materials with vertex colors, and both renderers shading metal and roughness. One project-wide list instead of per-part slots (see section 5). 6 new tests: undo, new faces inheriting and the bevel's Material Index, duplicates and booleans, project round trip and old files, both exports, and the app writing the `.mtl` file. Run on Linux with simulated input, on OpenGL 2.1 and Vulkan: adding a gold metal material, selecting it in the list, assigning it to the top face, and undo. Not tried yet: importing the colors into Roblox Studio | Not yet |

## 12. Open questions

- [x] Exact CPU model: i3-2328M (dxdiag showed "not applicable", so it came from Task Manager)
- [x] Graphics chip: Intel HD Graphics 3000
- [x] Windows version: Windows 10 Home 64-bit, so Stage 2 builds a 64-bit .exe
- [x] The graphics chip supports OpenGL 2.1: the laptop reports OpenGL 3.1 on Intel HD Graphics 3000
- [x] Owner has a GitHub account (Smogi67)
- [ ] Test importing an exported OBJ into Roblox Studio (Stage 3 goal; the owner will do this later and asked not to wait for it)
- [x] Roblox's mesh requirements checked 2026-11-06: 20,000 triangles per mesh, closed geometry with real thickness (https://create.roblox.com/docs/en-us/art/modeling/specifications)
- [x] List the real dependencies of the .exe for the installer: done in Stage 8, see section 6
- [x] The mesh crate compiles and its tests pass on Windows (GitHub Actions)
- [x] Approve the picture of the new Mixer Launcher before it is built (Stage 10 step 2): approved 2026-11-08
- [ ] Save the side panels' layout between runs?
- [ ] Does Roblox Studio show exported material colors (glTF materials, vertex colors, or OBJ's .mtl)? To try with the owner's import test
- [ ] Try the newer engines on a PC with a modern graphics card (the laptop only has OpenGL 2.1; GitHub's Windows machine only has DirectX 12 in software)
- [x] With shadows off, should View mode look flat? Owner: let people choose; fully flat by default, edge lines as an option
