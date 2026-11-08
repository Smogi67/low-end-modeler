Mixer
=====

A small 3D modeler for low-end Windows laptops.

FIRST TIME
1. Unzip this whole folder somewhere you can write to, such as Documents.
2. Start mixer-launcher.exe. The first time, it downloads SDL2 (about
   600 KB) from SDL's official site, checks it, puts it in this folder,
   and starts Mixer.

EVERY TIME
- Start Mixer with mixer-launcher.exe. When a new version of Mixer is on
  GitHub, it asks whether to update, then downloads it, checks it, and
  replaces Mixer's files in this folder. Your projects are not touched.
- Starting mixer.exe directly also works, without the update check.
- Advanced (in the launcher): rendering engine, update checks, and
  Repair install, which downloads SDL2 again.

FILES
- Mixer opens with a start window: make a Quick Project, make a named
  project, or open a recent one.
- Projects (.mix) live in Documents > Mixer > Projects, and quick ones in
  Documents > Mixer > Quick Projects, each in a folder of its own with an
  Exports folder (.obj or .glb).
- Settings live in Documents > Mixer > settings.txt.
- mixer.log in this folder records the graphics details and any
  errors. Send it along when reporting a problem.

LICENSES
- Inter font: SIL Open Font License, see licenses/Inter-LICENSE.txt
- Lucide icons: ISC License, see licenses/Lucide-LICENSE.txt
- SDL2: zlib License, see README-SDL.txt (installed by the launcher)
