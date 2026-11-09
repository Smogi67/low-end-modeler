//! Builds the ground grid's lines. No graphics code, so it is tested without a window.

/// Position (x, y, z) then color (r, g, b), for each line end.
pub const FLOATS_PER_VERTEX: usize = 6;

const EXTENT: i32 = 50;
const MAJOR_EVERY: i32 = 10;
// The colors come from the theme (palette.rs): by default X red, Y green and Z blue,
// as in Blender.
use crate::palette::{self, GRID_MAJOR, GRID_MINOR, X_AXIS, Y_AXIS, Z_AXIS};

/// Which plane the grid lies in. It stays on the ground, except in level views
/// along an axis, where a flat grid would be seen edge-on as a single line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridPlane {
    Ground,
    /// Upright, facing the front and back views.
    FrontBack,
    /// Upright, facing the right and left views.
    Side,
}

impl GridPlane {
    pub const ALL: [GridPlane; 3] = [GridPlane::Ground, GridPlane::FrontBack, GridPlane::Side];

    fn position(self, u: f32, v: f32) -> [f32; 3] {
        match self {
            GridPlane::Ground => [u, v, 0.0],
            GridPlane::FrontBack => [u, 0.0, v],
            GridPlane::Side => [0.0, u, v],
        }
    }

    fn axis_colors(self) -> ([f32; 3], [f32; 3]) {
        match self {
            GridPlane::Ground => (palette::rgb(X_AXIS), palette::rgb(Y_AXIS)),
            GridPlane::FrontBack => (palette::rgb(X_AXIS), palette::rgb(Z_AXIS)),
            GridPlane::Side => (palette::rgb(Y_AXIS), palette::rgb(Z_AXIS)),
        }
    }
}

fn color_for(line: i32, axis_color: [f32; 3]) -> [f32; 3] {
    if line == 0 {
        axis_color
    } else if line % MAJOR_EVERY == 0 {
        palette::rgb(GRID_MAJOR)
    } else {
        palette::rgb(GRID_MINOR)
    }
}

/// A flat grid in the given plane, one unit per square.
pub fn grid_vertices(plane: GridPlane) -> Vec<f32> {
    let lines = (2 * EXTENT + 1) as usize;
    let mut out = Vec::with_capacity(lines * 2 * 2 * FLOATS_PER_VERTEX);
    let limit = EXTENT as f32;
    let (u_color, v_color) = plane.axis_colors();
    for line in -EXTENT..=EXTENT {
        let at = line as f32;
        // Runs along the plane's first axis. The line through zero is that axis.
        let color = color_for(line, u_color);
        for u in [-limit, limit] {
            out.extend_from_slice(&plane.position(u, at));
            out.extend_from_slice(&color);
        }
        // Runs along the plane's second axis.
        let color = color_for(line, v_color);
        for v in [-limit, limit] {
            out.extend_from_slice(&plane.position(at, v));
            out.extend_from_slice(&color);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertices(plane: GridPlane) -> Vec<[f32; 6]> {
        grid_vertices(plane)
            .chunks_exact(FLOATS_PER_VERTEX)
            .map(|v| v.try_into().unwrap())
            .collect()
    }

    #[test]
    fn every_plane_has_404_line_ends() {
        for plane in GridPlane::ALL {
            assert_eq!(grid_vertices(plane).len(), 404 * FLOATS_PER_VERTEX);
        }
    }

    #[test]
    fn each_plane_is_flat_on_its_own_axis_and_within_its_extent() {
        for (plane, flat_axis) in [
            (GridPlane::Ground, 2),
            (GridPlane::FrontBack, 1),
            (GridPlane::Side, 0),
        ] {
            for v in vertices(plane) {
                assert_eq!(v[flat_axis], 0.0);
                assert!(v[..3].iter().all(|c| c.abs() <= 50.0));
            }
        }
    }

    #[test]
    fn each_line_is_straight_and_one_color() {
        for plane in GridPlane::ALL {
            for line in vertices(plane).chunks_exact(2) {
                let (a, b) = (line[0], line[1]);
                assert_eq!(a[3..], b[3..]);
                let changed = (0..3).filter(|&i| a[i] != b[i]).count();
                assert_eq!(changed, 1, "{a:?} {b:?}");
            }
        }
    }

    /// Each colored line must run along the world axis its color stands for.
    #[test]
    fn axes_are_colored_along_the_right_directions() {
        for (plane, axes) in [
            (GridPlane::Ground, [(X_AXIS, 0), (Y_AXIS, 1)]),
            (GridPlane::FrontBack, [(X_AXIS, 0), (Z_AXIS, 2)]),
            (GridPlane::Side, [(Y_AXIS, 1), (Z_AXIS, 2)]),
        ] {
            let all = vertices(plane);
            for (color, axis) in axes {
                let color = palette::rgb(color);
                let lines: Vec<_> = all.chunks_exact(2).filter(|l| l[0][3..] == color).collect();
                assert_eq!(lines.len(), 1, "{plane:?}");
                for other in (0..3).filter(|&i| i != axis) {
                    assert_eq!(lines[0][0][other], 0.0);
                    assert_eq!(lines[0][1][other], 0.0);
                }
                assert_eq!((lines[0][0][axis] - lines[0][1][axis]).abs(), 100.0);
            }
        }
    }

    #[test]
    fn major_lines_appear_every_ten_units() {
        let x = palette::rgb(X_AXIS);
        let y = palette::rgb(Y_AXIS);
        assert_eq!(color_for(10, x), palette::rgb(GRID_MAJOR));
        assert_eq!(color_for(-20, x), palette::rgb(GRID_MAJOR));
        assert_eq!(color_for(7, x), palette::rgb(GRID_MINOR));
        assert_eq!(color_for(0, y), y);
    }
}
