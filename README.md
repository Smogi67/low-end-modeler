# Low-End 3D Modeler

A small Blender-inspired modeler for very low-end Windows laptops. Read `SPEC.md` first: it holds the decisions, scope, and current status.

- `crates/mesh`: the editable mesh, validation, and undo/redo. It has no graphics or UI code and no dependencies.

Run the tests with `cargo test`. Run a longer random test with:

```
MESH_FUZZ_SEEDS=300 MESH_FUZZ_STEPS=1500 cargo test --release --test randomized
```

Measure memory and speed with `cargo run --release --example measure`.
