Mixer
=====

A small 3D modeler for low-end Windows laptops, made for Roblox Studio.

FIRST TIME
1. Unzip this whole folder somewhere you can write to, such as Documents.
2. Run mixer-setup.exe once. It downloads SDL2 (about 600 KB)
   from SDL's official site, checks it, and puts it in this folder.
   It also checks that the graphics driver supports OpenGL 2.1.
3. Start mixer.exe.

FILES
- Mixer opens with a start window: make a Quick Project, make a named
  project, or open a recent one.
- Projects (.mix) live in Documents > Mixer > Projects, and quick ones in
  Documents > Mixer > Quick Projects. Each project's exports for Roblox
  (.obj or .glb) go into an Exports folder next to it.
- mixer.log in this folder records the graphics details and any
  errors. Send it along when reporting a problem.

LICENSES
- Inter font: SIL Open Font License, see licenses/Inter-LICENSE.txt
- Lucide icons: ISC License, see licenses/Lucide-LICENSE.txt
- SDL2: zlib License, see README-SDL.txt (installed by setup)
