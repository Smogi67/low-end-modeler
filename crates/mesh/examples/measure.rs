//! Measures memory and speed. Run with: cargo run --release --example measure
//!
//! The numbers are from whichever machine runs this. They are useful for spotting
//! regressions and rough limits, not as a promise about the target laptop.

use mixer_mesh::{Document, Mesh, VertexId};
use std::time::{Duration, Instant};

fn grid(size: usize) -> (Mesh, Vec<VertexId>) {
    let mut mesh = Mesh::new();
    let mut v = Vec::with_capacity((size + 1) * (size + 1));
    for y in 0..=size {
        for x in 0..=size {
            v.push(mesh.add_vertex([x as f32, y as f32, 0.0]).unwrap());
        }
    }
    let at = |x: usize, y: usize| y * (size + 1) + x;
    for y in 0..size {
        for x in 0..size {
            let quad = [
                v[at(x, y)],
                v[at(x + 1, y)],
                v[at(x + 1, y + 1)],
                v[at(x, y + 1)],
            ];
            mesh.add_face(&quad).unwrap();
        }
    }
    (mesh, v)
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn main() {
    println!(
        "quads | MiB mesh | build ms | validate ms | move 1 ms | 1-vertex step B | move all ms | all-vertex step KiB | undo all ms | split edge ms | neighbor lookups/s"
    );
    for size in [10, 50, 100, 200, 500] {
        let start = Instant::now();
        let (mesh, v) = grid(size);
        let build = start.elapsed();
        let memory = mesh.memory_bytes() as f64 / (1024.0 * 1024.0);

        let start = Instant::now();
        mesh.validate().unwrap();
        let validate = start.elapsed();

        let mut doc = Document::new(mesh);

        let start = Instant::now();
        doc.edit("one", |m| m.set_vertex_position(v[0], [5.0; 3]))
            .unwrap();
        let move_one = start.elapsed();
        let one_step = doc.history_bytes();

        let start = Instant::now();
        doc.edit("all", |m| {
            v.iter()
                .try_for_each(|&vertex| m.set_vertex_position(vertex, [1.0; 3]))
        })
        .unwrap();
        let move_all = start.elapsed();
        let all_step = (doc.history_bytes() - one_step) as f64 / 1024.0;

        let start = Instant::now();
        doc.undo();
        let undo_all = start.elapsed();

        let edge = doc.mesh().edge_between(v[0], v[1]).unwrap().unwrap();
        let start = Instant::now();
        doc.edit("split", |m| m.split_edge(edge, 0.5).map(|_| ()))
            .unwrap();
        let split = start.elapsed();

        let start = Instant::now();
        let mut lookups = 0u64;
        let mut sum = 0usize;
        for _ in 0..20 {
            for &vertex in &v {
                sum += doc.mesh().vertex_faces(vertex).unwrap().count();
                lookups += 1;
            }
        }
        let rate = lookups as f64 / start.elapsed().as_secs_f64();
        assert!(sum > 0);

        println!(
            "{:>5} | {:>8.2} | {:>8.1} | {:>11.2} | {:>9.3} | {:>15} | {:>11.2} | {:>19.1} | {:>11.2} | {:>13.3} | {:>12.0}",
            size * size,
            memory,
            millis(build),
            millis(validate),
            millis(move_one),
            one_step,
            millis(move_all),
            all_step,
            millis(undo_all),
            millis(split),
            rate
        );
    }
}
