# Low-End 3D Modeler: Project Specification

Read this whole file before writing code. It records the decisions made so far, the scope, and how to work with the project owner.

Status: **Stage 4 code written and working on Linux with OpenGL limited to 2.1. The owner's laptop tests of Stages 3 and 4 and the Roblox import are still needed. See section 11.**
Next step: **After those tests pass, Stage 5: extrude, inset, merge, delete, fill, subdivide.**

## 1. Goal

Build a small, Blender-inspired desktop 3D modeler for very low-end Windows laptops. Its job is to create low-poly models and export them for use in games, mainly Roblox Studio.

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

The app is in `crates/app`. The code is on GitHub: https://github.com/Smogi67/low-end-modeler. Each push to `main` builds the .exe with GitHub Actions.

- **Rendering:** GLSL 1.20 shaders and vertex buffers only, which are both part of OpenGL 2.1. All OpenGL calls are in `renderer.rs`.
- **Idle use:** the main loop sleeps until an event arrives, and draws only if that event changed something.
- **Axes:** Z is up, as in Blender. Roblox uses Y up, so Stage 3's export must convert this.
- **Camera controls (owner's choice, replaces Blender's here):** right drag turns the view and middle drag (holding the wheel) slides it, with no keys held. The wheel zooms, and Ctrl + middle drag also zooms. Plain left stays free for selecting in Stage 4. Alt + left drag turns the view on touchpads, and Alt + Shift + left drag slides it.
- **View keys:** F1, F3, and F7 or numpad 1, 3, and 7 give front, right, and top views (add Ctrl for the opposite side). F5 or numpad 5 switches to orthographic. Numpad 2, 4, 6, and 8 turn the view, and Home resets it.
- **Linking:** on Windows, SDL2 and the C runtime are built into the .exe. Review this when designing the dependency installer (section 6).
- **Grid:** it lies on the ground, except in level views along an axis (such as front and right), where it stands upright, as in Blender.
- **Dear ImGui:** not added yet, because Stage 2 needs no UI. Check the bindings work with OpenGL 2.1 when the first UI is added.
- **Log file:** `low-end-modeler.log` beside the .exe records the OpenGL details and any errors. The title bar shows the OpenGL version too.

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
- **Mouse:** left click selects, left drag selects with the chosen tool (box, circle, or lasso), Shift adds, and Ctrl removes. A click on nothing clears the selection. This follows the owner's wish to avoid extra keys for common actions.
- **Hidden parts:** depth is read from the graphics card once per selection, so only visible parts get selected. There is no select-through mode yet.
- **Transforms:** G, R, and S or panel buttons start a drag around the selection's center. X, Y, or Z locks an axis, Ctrl snaps (1 unit, 15 degrees, or 0.1 scale), a left click confirms, and a right click or Esc cancels. Duplicate is one undo step with its move, so cancelling removes the copy.
- **Mirror:** across the selection's center. Fully selected faces get their winding reversed, so they keep facing outward.
- **Partial updates:** during a transform only the affected faces, edges, and points are rewritten and sent (section 4, rule 3). Other changes, including selection, still rebuild everything.
- **Scheduling gaps:** proportional editing, the 3D cursor, other pivot points, and typing exact values have no stage yet. Add them to Stage 8 or 9 unless the owner says otherwise. Loop selection stays in Stage 6.

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

- **Delivery:** the release is one .zip that holds both the app's .exe and the installer. The installer never downloads or installs the app.
- **Purpose:** the installer deals only with dependencies. Shortcuts, file associations, sample files, and app updates are not part of it.
- **First screen:** a checkbox list of dependencies that shows which are required and which are optional. Required items cannot be unticked.
- **After pressing Continue:** a console-style view shows what is really happening, such as downloading with a progress bar, unzipping, and installing. Every step and every progress value must be real, and errors must be shown.
- **Honest list:** show only what the real .exe needs or can use. Do not add dummy entries. Skip or mark anything already present. If a dependency cannot be installed by a program, such as OpenGL support in the graphics driver, show it as a check with clear advice.
- **Must run first:** the installer itself cannot depend on anything it installs.
- **Downloads:** use official sources over HTTPS and verify each file before installing it.
- **List not final:** check the real dependencies of the .exe once the Stage 2 build exists. Linking everything into the .exe would leave the installer little to do, so raise that choice with the owner rather than deciding it alone.
- **Before the installer exists:** test builds must run on the owner's laptop without it.

### Later

- Knife and bisect
- Spin, screw, solidify, shrink/fatten, grid fill, convex hull
- Modifiers: mirror and array first, then solidify, bevel, decimate, weld, triangulate, and boolean last
- UV seams, unwrapping, and UV editing
- Simple materials
- Mesh checks for bad geometry before export

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

## 12. Open questions

- [x] Exact CPU model: i3-2328M (dxdiag showed "not applicable", so it came from Task Manager)
- [x] Graphics chip: Intel HD Graphics 3000
- [x] Windows version: Windows 10 Home 64-bit, so Stage 2 builds a 64-bit .exe
- [x] The graphics chip supports OpenGL 2.1: the laptop reports OpenGL 3.1 on Intel HD Graphics 3000
- [x] Owner has a GitHub account (Smogi67)
- [ ] Test importing an exported OBJ into Roblox Studio (Stage 3 goal, not yet reported)
- [x] Roblox's mesh requirements checked 2026-11-06: 20,000 triangles per mesh, closed geometry with real thickness (https://create.roblox.com/docs/en-us/art/modeling/specifications)
- [ ] List the real dependencies of the .exe for the installer (once the Stage 2 build exists)
- [x] The mesh crate compiles and its tests pass on Windows (GitHub Actions)
