# Low-End 3D Modeler: Project Specification

Read this whole file before writing code. It records the decisions made so far, the scope, and how to work with the project owner.

Status: **Stage 2 code written and working on Linux with software graphics. The Windows build and a test on the owner's laptop are still needed. See section 11.**
Next step: **Finish Stage 2 once the owner's laptop test passes, then Stage 3.**

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
| GPU | Integrated Intel, reported as "Intel(R) HD Graphics Family", driver 9.17.10.4459, Direct3D feature level 10_1. Likely HD Graphics 3000, based on the CPU model |
| Screen | 1920 x 1080 |

OpenGL 2.1 support is expected for this chip but not yet confirmed on the laptop. The first Stage 2 build must report the real OpenGL version. Do not assume a graphics feature newer than OpenGL 2.1 is available.

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
- **Camera controls:** middle drag orbits, Shift + middle drag pans, Ctrl + middle drag zooms, and the wheel zooms. Alt + left drag stands in for the middle button on touchpads. Numpad 1, 3, and 7 give front, right, and top views (add Ctrl for the opposite side), 5 switches to orthographic, and 2, 4, 6, 8 turn the view. Home resets it.
- **Linking:** on Windows, SDL2 and the C runtime are built into the .exe. Review this when designing the dependency installer (section 6).
- **Dear ImGui:** not added yet, because Stage 2 needs no UI. Check the bindings work with OpenGL 2.1 when the first UI is added.
- **Log file:** `low-end-modeler.log` beside the .exe records the OpenGL details and any errors. The title bar shows the OpenGL version too.

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

## 12. Open questions

- [x] Exact CPU model: i3-2328M (dxdiag showed "not applicable", so it came from Task Manager)
- [x] Graphics chip as reported: Intel(R) HD Graphics Family
- [x] Windows version: Windows 10 Home 64-bit, so Stage 2 builds a 64-bit .exe
- [ ] Confirm the graphics chip supports OpenGL 2.1 (first Stage 2 build on the laptop)
- [x] Owner has a GitHub account (Smogi67)
- [ ] Check Roblox's current mesh requirements before Stage 3
- [ ] List the real dependencies of the .exe for the installer (once the Stage 2 build exists)
- [x] The mesh crate compiles and its tests pass on Windows (GitHub Actions)
