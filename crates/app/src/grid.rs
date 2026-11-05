//! Builds the ground grid's lines. No graphics code, so it is tested without a window.

/// Position (x, y, z) then color (r, g, b), for each line end.
pub const FLOATS_PER_VERTEX: usize = 6;

const EXTENT: i32 = 50;
const MAJOR_EVERY: i32 = 10;
const MINOR: [f32; 3] = [0.30, 0.30, 0.30];
const MAJOR: [f32; 3] = [0.42, 0.42, 0.42];
/// The X axis is red and the Y axis green, as in Blender.
const X_AXIS: [f32; 3] = [0.80, 0.25, 0.28];
const Y_AXIS: [f32; 3] = [0.45, 0.70, 0.20];

fn color_for(line: i32, axis_color: [f32; 3]) -> [f32; 3] {
    if line == 0 {
        axis_color
    } else if line % MAJOR_EVERY == 0 {
        MAJOR
    } else {
        MINOR
    }
}

/// A flat grid on the Z = 0 plane, one unit per square.
pub fn grid_vertices() -> Vec<f32> {
    let lines = (2 * EXTENT + 1) as usize;
    let mut out = Vec::with_capacity(lines * 2 * 2 * FLOATS_PER_VERTEX);
    let limit = EXTENT as f32;
    for line in -EXTENT..=EXTENT {
        let at = line as f32;
        // Runs along X at this Y. The line through Y = 0 is the X axis.
        let color = color_for(line, X_AXIS);
        for x in [-limit, limit] {
            out.extend_from_slice(&[x, at, 0.0]);
            out.extend_from_slice(&color);
        }
        // Runs along Y at this X. The line through X = 0 is the Y axis.
        let color = color_for(line, Y_AXIS);
        for y in [-limit, limit] {
            out.extend_from_slice(&[at, y, 0.0]);
            out.extend_from_slice(&color);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertices() -> Vec<[f32; 6]> {
        grid_vertices()
            .chunks_exact(FLOATS_PER_VERTEX)
            .map(|v| v.try_into().unwrap())
            .collect()
    }

    #[test]
    fn has_two_ends_for_each_of_404_half_lines() {
        assert_eq!(grid_vertices().len(), 404 * FLOATS_PER_VERTEX);
    }

    #[test]
    fn lies_flat_within_its_extent() {
        for v in vertices() {
            assert_eq!(v[2], 0.0);
            assert!(v[0].abs() <= 50.0 && v[1].abs() <= 50.0);
        }
    }

    #[test]
    fn each_line_is_straight_and_one_color() {
        for line in vertices().chunks_exact(2) {
            let (a, b) = (line[0], line[1]);
            assert_eq!(a[3..], b[3..]);
            assert!((a[0] == b[0]) != (a[1] == b[1]), "{a:?} {b:?}");
            assert_eq!((a[0] - b[0]).abs() + (a[1] - b[1]).abs(), 100.0);
        }
    }

    #[test]
    fn axes_are_colored_along_the_right_directions() {
        let lines: Vec<_> = vertices().chunks_exact(2).map(|l| [l[0], l[1]]).collect();
        let x_axis: Vec<_> = lines.iter().filter(|l| l[0][3..] == X_AXIS).collect();
        let y_axis: Vec<_> = lines.iter().filter(|l| l[0][3..] == Y_AXIS).collect();
        assert_eq!((x_axis.len(), y_axis.len()), (1, 1));
        assert!(x_axis[0][0][1] == 0.0 && x_axis[0][1][1] == 0.0);
        assert!(y_axis[0][0][0] == 0.0 && y_axis[0][1][0] == 0.0);
    }

    #[test]
    fn major_lines_appear_every_ten_units() {
        assert_eq!(color_for(10, X_AXIS), MAJOR);
        assert_eq!(color_for(-20, X_AXIS), MAJOR);
        assert_eq!(color_for(7, X_AXIS), MINOR);
        assert_eq!(color_for(0, Y_AXIS), Y_AXIS);
    }
}
