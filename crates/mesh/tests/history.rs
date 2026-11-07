//! Undo, redo, cancelled edits and the memory cap.

mod common;
use common::{cube, edge, grid};
use modeler_mesh::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_STEPS, Document, HistoryLimits, Mesh, MeshError,
};

fn doc_with_cube() -> (
    Document,
    [modeler_mesh::VertexId; 8],
    [modeler_mesh::FaceId; 6],
) {
    let (mesh, v, f) = cube();
    (Document::new(mesh), v, f)
}

/// Every kind of edit, undone and redone, must restore the exact state each time.
#[test]
fn every_edit_can_be_undone_and_redone() {
    type Edit = fn(&mut Mesh, &[modeler_mesh::VertexId; 8], &[modeler_mesh::FaceId; 6]);
    let edits: Vec<(&'static str, Edit)> = vec![
        ("add vertex", |m, _, _| {
            m.add_vertex([9.0; 3]).unwrap();
        }),
        ("move", |m, v, _| {
            m.set_vertex_position(v[0], [9.0; 3]).unwrap()
        }),
        ("add edge", |m, v, _| {
            m.add_edge(v[0], v[6]).unwrap();
        }),
        ("add face", |m, v, _| {
            m.add_face(&[v[0], v[2], v[6]]).unwrap();
        }),
        ("remove face", |m, _, f| m.remove_face(f[0]).unwrap()),
        ("remove edge", |m, v, _| {
            m.remove_edge(edge(m, v[0], v[1])).unwrap()
        }),
        ("remove vertex", |m, v, _| m.remove_vertex(v[0]).unwrap()),
        ("split edge", |m, v, _| {
            m.split_edge(edge(m, v[0], v[1]), 0.5).unwrap();
        }),
        ("split face", |m, v, f| {
            m.split_face(f[1], v[4], v[6]).unwrap();
        }),
        ("join", |m, v, _| {
            m.join_faces(edge(m, v[0], v[1])).unwrap();
        }),
        ("flip", |m, _, f| m.flip_face(f[0]).unwrap()),
    ];
    for (label, run) in edits {
        let (mut doc, v, f) = doc_with_cube();
        let before = doc.mesh().clone();
        doc.edit(label, |m| {
            run(m, &v, &f);
            Ok(())
        })
        .unwrap();
        let after = doc.mesh().clone();
        assert_ne!(before, after, "{label} should change something");

        for _ in 0..3 {
            assert_eq!(doc.undo(), Some(label));
            assert_eq!(*doc.mesh(), before, "undo of {label}");
            doc.mesh().validate().unwrap();
            assert_eq!(doc.redo(), Some(label));
            assert_eq!(*doc.mesh(), after, "redo of {label}");
            doc.mesh().validate().unwrap();
        }
    }
}

#[test]
fn undo_and_redo_follow_order_across_many_steps() {
    let (mut doc, v, f) = doc_with_cube();
    let mut states = vec![doc.mesh().clone()];
    let vertex = v[0];
    doc.edit("one", |m| m.set_vertex_position(vertex, [5.0; 3]))
        .unwrap();
    states.push(doc.mesh().clone());
    doc.edit("two", |m| m.remove_face(f[0])).unwrap();
    states.push(doc.mesh().clone());
    doc.edit("three", |m| {
        m.split_edge(edge(m, v[6], v[7]), 0.5).map(|_| ())
    })
    .unwrap();
    states.push(doc.mesh().clone());

    assert_eq!(doc.undo_count(), 3);
    assert_eq!(doc.redo_count(), 0);
    assert_eq!(doc.undo_label(), Some("three"));
    assert_eq!(doc.redo_label(), None);

    assert_eq!(doc.undo(), Some("three"));
    assert_eq!(*doc.mesh(), states[2]);
    assert_eq!(doc.undo_label(), Some("two"));
    assert_eq!(doc.redo_label(), Some("three"));
    assert_eq!(doc.undo(), Some("two"));
    assert_eq!(*doc.mesh(), states[1]);
    assert_eq!(doc.undo(), Some("one"));
    assert_eq!(*doc.mesh(), states[0]);
    assert_eq!(doc.undo(), None, "nothing is left to undo");
    assert_eq!(doc.undo_count(), 0);
    assert_eq!(doc.redo_count(), 3);

    assert_eq!(doc.redo(), Some("one"));
    assert_eq!(*doc.mesh(), states[1]);
    assert_eq!(doc.redo(), Some("two"));
    assert_eq!(*doc.mesh(), states[2]);
    assert_eq!(doc.redo(), Some("three"));
    assert_eq!(*doc.mesh(), states[3]);
    assert_eq!(doc.redo(), None, "nothing is left to redo");
}

#[test]
fn a_new_edit_clears_redo_and_its_memory() {
    let (mut doc, v, _) = doc_with_cube();
    let vertex = v[0];
    doc.edit("first", |m| m.set_vertex_position(vertex, [1.0; 3]))
        .unwrap();
    let one_step = doc.history_bytes();
    doc.edit("second", |m| m.set_vertex_position(vertex, [2.0; 3]))
        .unwrap();
    doc.undo();
    assert_eq!(doc.redo_count(), 1);
    doc.edit("third", |m| m.set_vertex_position(vertex, [3.0; 3]))
        .unwrap();
    assert_eq!(doc.redo_count(), 0);
    assert_eq!(doc.redo(), None);
    assert_eq!(doc.history_bytes(), one_step * 2);
}

#[test]
fn undone_ids_go_stale_and_redo_brings_them_back() {
    let (mut doc, _, _) = doc_with_cube();
    let new = doc.edit("add", |m| m.add_vertex([7.0; 3])).unwrap();
    assert!(doc.mesh().vertex(new).is_some());
    doc.undo();
    assert!(doc.mesh().vertex(new).is_none());
    doc.redo();
    assert_eq!(doc.mesh().vertex(new).unwrap().position(), [7.0; 3]);
}

#[test]
fn removed_ids_come_back_on_undo() {
    let (mut doc, v, f) = doc_with_cube();
    doc.edit("remove", |m| m.remove_vertex(v[0])).unwrap();
    assert!(doc.mesh().vertex(v[0]).is_none());
    assert!(doc.mesh().face(f[0]).is_none());
    doc.undo();
    assert_eq!(doc.mesh().vertex(v[0]).unwrap().position(), [0.0; 3]);
    assert!(doc.mesh().face(f[0]).is_some());
}

/// The specific danger the spec names: an old ID silently pointing at something
/// else, here through a slot reused after undo.
#[test]
fn an_id_from_an_undone_edit_never_matches_a_later_element() {
    let (mut doc, _, _) = doc_with_cube();
    let undone = doc.edit("add", |m| m.add_vertex([1.0; 3])).unwrap();
    doc.undo();
    let later = doc.edit("add again", |m| m.add_vertex([2.0; 3])).unwrap();
    assert_eq!(
        undone.index(),
        later.index(),
        "this test needs the slot to be reused"
    );
    assert_ne!(undone, later);
    assert!(doc.mesh().vertex(undone).is_none());
    assert_eq!(doc.mesh().vertex(later).unwrap().position(), [2.0; 3]);
}

#[test]
fn a_failed_edit_is_fully_reverted() {
    let (mut doc, v, f) = doc_with_cube();
    let before = doc.mesh().clone();
    let result = doc.edit("half finished", |m| {
        m.remove_vertex(v[0])?;
        m.add_vertex([1.0; 3])?;
        m.remove_face(f[1])?;
        m.add_edge(v[0], v[1])?; // v[0] is gone, so this fails.
        Ok(())
    });
    assert_eq!(result, Err(MeshError::StaleVertex(v[0])));
    assert_eq!(*doc.mesh(), before);
    doc.mesh().validate().unwrap();
    assert_eq!(doc.undo_count(), 0);
    assert!(!doc.edit_in_progress());
}

#[test]
fn a_failed_edit_keeps_redo_available() {
    let (mut doc, v, _) = doc_with_cube();
    let vertex = v[0];
    doc.edit("move", |m| m.set_vertex_position(vertex, [1.0; 3]))
        .unwrap();
    doc.undo();
    let result: Result<(), _> = doc.edit("fails", |m| {
        m.set_vertex_position(vertex, [2.0; 3])?;
        Err(MeshError::InvalidFactor)
    });
    assert!(result.is_err());
    assert_eq!(doc.redo(), Some("move"));
    assert_eq!(doc.mesh().vertex(vertex).unwrap().position(), [1.0; 3]);
}

#[test]
fn an_edit_that_changes_nothing_records_no_step() {
    let (mut doc, v, _) = doc_with_cube();
    let vertex = v[0];
    doc.edit("real", |m| m.set_vertex_position(vertex, [1.0; 3]))
        .unwrap();
    doc.undo();

    doc.edit("nothing", |_| Ok(())).unwrap();
    doc.edit("back to the start", |m| {
        m.set_vertex_position(vertex, [9.0; 3])?;
        m.set_vertex_position(vertex, [0.0; 3])
    })
    .unwrap();
    assert_eq!(doc.undo_count(), 0);
    assert_eq!(doc.redo_count(), 1, "a no-op must not throw redo away");
    assert!(doc.history_bytes() > 0);
}

#[test]
fn a_drag_stays_open_across_frames_then_commits() {
    let (mut doc, v, _) = doc_with_cube();
    assert!(doc.active_mesh().is_none());
    doc.begin_edit("drag").unwrap();
    assert!(doc.edit_in_progress());
    for frame in 1..=100 {
        let mesh = doc.active_mesh().unwrap();
        mesh.set_vertex_position(v[0], [frame as f32, 0.0, 0.0])
            .unwrap();
    }
    assert_eq!(doc.commit_edit(), Ok(true));
    assert!(!doc.edit_in_progress());
    assert_eq!(doc.undo_count(), 1);
    assert_eq!(
        doc.mesh().vertex(v[0]).unwrap().position(),
        [100.0, 0.0, 0.0]
    );
    doc.undo();
    assert_eq!(doc.mesh().vertex(v[0]).unwrap().position(), [0.0; 3]);
}

#[test]
fn a_drag_can_be_cancelled() {
    let (mut doc, v, _) = doc_with_cube();
    let before = doc.mesh().clone();
    doc.begin_edit("drag").unwrap();
    let mesh = doc.active_mesh().unwrap();
    mesh.set_vertex_position(v[0], [9.0; 3]).unwrap();
    let created = mesh.add_vertex([8.0; 3]).unwrap();
    doc.cancel_edit().unwrap();
    assert_eq!(*doc.mesh(), before);
    assert_eq!(doc.undo_count(), 0);
    assert!(doc.mesh().vertex(created).is_none());
    let later = doc.edit("add", |m| m.add_vertex([1.0; 3])).unwrap();
    assert_eq!(later.index(), created.index());
    assert!(doc.mesh().vertex(created).is_none());
}

#[test]
fn an_open_edit_blocks_other_history_actions() {
    let (mut doc, v, _) = doc_with_cube();
    let vertex = v[0];
    doc.edit("first", |m| m.set_vertex_position(vertex, [1.0; 3]))
        .unwrap();
    doc.edit("second", |m| m.set_vertex_position(vertex, [2.0; 3]))
        .unwrap();
    doc.undo();
    doc.begin_edit("drag").unwrap();
    assert_eq!(doc.begin_edit("another"), Err(MeshError::EditInProgress));
    assert_eq!(
        doc.edit("another", |_| Ok(())),
        Err(MeshError::EditInProgress)
    );
    assert_eq!(doc.undo(), None);
    assert_eq!(doc.redo(), None);
    assert_eq!(doc.clear_history(), Err(MeshError::EditInProgress));
    assert_eq!((doc.undo_count(), doc.redo_count()), (1, 1));
    doc.cancel_edit().unwrap();
    assert_eq!(doc.undo(), Some("first"));
}

#[test]
fn commit_or_cancel_without_an_open_edit_is_refused() {
    let (mut doc, _, _) = doc_with_cube();
    assert_eq!(doc.commit_edit(), Err(MeshError::NoEditInProgress));
    assert_eq!(doc.cancel_edit(), Err(MeshError::NoEditInProgress));
}

#[test]
fn committing_a_no_op_reports_false() {
    let (mut doc, _, _) = doc_with_cube();
    doc.begin_edit("nothing").unwrap();
    assert_eq!(doc.commit_edit(), Ok(false));
    assert_eq!(doc.undo_count(), 0);
}

#[test]
fn into_mesh_cancels_an_open_edit() {
    let (mut doc, v, _) = doc_with_cube();
    let before = doc.mesh().clone();
    doc.begin_edit("drag").unwrap();
    doc.active_mesh()
        .unwrap()
        .set_vertex_position(v[0], [9.0; 3])
        .unwrap();
    let mut mesh = doc.into_mesh();
    assert_eq!(mesh, before);
    mesh.validate().unwrap();
    // Recording has stopped, so the mesh can be wrapped in a new document.
    mesh.add_vertex([1.0; 3]).unwrap();
    let mut doc = Document::new(mesh);
    doc.edit("again", |m| m.add_vertex([2.0; 3])).unwrap();
}

#[test]
fn default_limits() {
    let doc = Document::new(Mesh::new());
    let limits = doc.limits();
    assert_eq!(limits, HistoryLimits::default());
    assert_eq!(limits.max_bytes, DEFAULT_MAX_BYTES);
    assert_eq!(limits.max_steps, DEFAULT_MAX_STEPS);
    assert_eq!(DEFAULT_MAX_BYTES, 67_108_864, "64 MiB");
    assert_eq!(DEFAULT_MAX_STEPS, 256);

    let custom = HistoryLimits {
        max_bytes: 123,
        max_steps: 4,
    };
    assert_eq!(Document::with_limits(Mesh::new(), custom).limits(), custom);
}

#[test]
fn step_limit_drops_the_oldest_steps() {
    let (mesh, v, _) = cube();
    let limits = HistoryLimits {
        max_bytes: usize::MAX,
        max_steps: 3,
    };
    let mut doc = Document::with_limits(mesh, limits);
    let vertex = v[0];
    for i in 1..=5 {
        doc.edit("move", |m| m.set_vertex_position(vertex, [i as f32; 3]))
            .unwrap();
    }
    assert_eq!(doc.undo_count(), 3);
    while doc.undo().is_some() {}
    assert_eq!(doc.mesh().vertex(vertex).unwrap().position(), [2.0; 3]);
}

#[test]
fn memory_cap_drops_the_oldest_steps_and_is_respected() {
    let (mesh, v, _) = cube();
    let vertex = v[0];

    // Measure the cost of one step, then allow room for about three of them.
    let mut probe = Document::new(mesh.clone());
    probe
        .edit("move", |m| m.set_vertex_position(vertex, [1.0; 3]))
        .unwrap();
    let step = probe.history_bytes();
    assert!(step > 0);

    let limits = HistoryLimits {
        max_bytes: step * 3 + step / 2,
        max_steps: usize::MAX,
    };
    let mut doc = Document::with_limits(mesh, limits);
    for i in 1..=20 {
        doc.edit("move", |m| m.set_vertex_position(vertex, [i as f32; 3]))
            .unwrap();
        assert!(doc.history_bytes() <= limits.max_bytes);
    }
    assert_eq!(doc.undo_count(), 3);
    assert_eq!(doc.history_bytes(), step * 3);
    while doc.undo().is_some() {
        assert!(doc.history_bytes() <= limits.max_bytes);
    }
    assert_eq!(doc.mesh().vertex(vertex).unwrap().position(), [17.0; 3]);
    assert!(doc.history_bytes() <= limits.max_bytes);
    while doc.redo().is_some() {
        assert!(doc.history_bytes() <= limits.max_bytes);
    }
    assert_eq!(doc.mesh().vertex(vertex).unwrap().position(), [20.0; 3]);
}

#[test]
fn memory_is_measured_in_bytes_not_only_in_steps() {
    // A small limit keeps few large steps, or many small ones.
    let (mesh, v) = grid(30, 30);
    let budget = 64 * 1024;
    let limits = HistoryLimits {
        max_bytes: budget,
        max_steps: usize::MAX,
    };

    let mut small = Document::with_limits(mesh.clone(), limits);
    for i in 1..=500 {
        small
            .edit("small", |m| m.set_vertex_position(v[0], [i as f32; 3]))
            .unwrap();
    }
    let mut large = Document::with_limits(mesh, limits);
    let all = v.clone();
    for i in 1..=50 {
        large
            .edit("large", |m| {
                all.iter()
                    .try_for_each(|&vertex| m.set_vertex_position(vertex, [i as f32; 3]))
            })
            .unwrap();
    }
    assert!(small.history_bytes() <= budget && large.history_bytes() <= budget);
    assert!(small.undo_count() > large.undo_count() * 10);
}

#[test]
fn the_newest_step_is_kept_even_if_it_exceeds_the_cap() {
    let (mesh, v, _) = cube();
    let limits = HistoryLimits {
        max_bytes: 1,
        max_steps: 0,
    };
    let mut doc = Document::with_limits(mesh, limits);
    let vertex = v[0];
    doc.edit("first", |m| m.set_vertex_position(vertex, [1.0; 3]))
        .unwrap();
    doc.edit("second", |m| m.set_vertex_position(vertex, [2.0; 3]))
        .unwrap();
    assert_eq!(doc.undo_count(), 1);
    assert_eq!(doc.undo(), Some("second"));
    assert_eq!(doc.mesh().vertex(vertex).unwrap().position(), [1.0; 3]);
    assert_eq!(doc.undo(), None);
}

#[test]
fn memory_accounting_returns_to_zero() {
    let (mut doc, v, f) = doc_with_cube();
    assert_eq!(doc.history_bytes(), 0);
    doc.edit("a", |m| m.remove_face(f[0])).unwrap();
    doc.edit("b", |m| m.remove_vertex(v[6])).unwrap();
    doc.undo();
    assert!(doc.history_bytes() > 0);
    doc.clear_history().unwrap();
    assert_eq!(doc.history_bytes(), 0);
    assert_eq!((doc.undo_count(), doc.redo_count()), (0, 0));
    assert_eq!(doc.undo(), None);
    assert_eq!(doc.redo(), None);
}

#[test]
fn lowering_the_limits_drops_undo_steps_then_far_redo_steps() {
    let (mut doc, v, _) = doc_with_cube();
    let vertex = v[0];
    for i in 1..=6 {
        doc.edit("move", |m| m.set_vertex_position(vertex, [i as f32; 3]))
            .unwrap();
    }
    let step = doc.history_bytes() / 6;
    doc.undo();
    doc.undo();
    assert_eq!((doc.undo_count(), doc.redo_count()), (4, 2));

    // Keeps room for three steps: undo is cut first, down to one.
    doc.set_limits(HistoryLimits {
        max_bytes: step * 3,
        max_steps: usize::MAX,
    });
    assert_eq!((doc.undo_count(), doc.redo_count()), (1, 2));
    assert!(doc.history_bytes() <= step * 3);

    // Room for two: the redo step farthest from now goes first. Next redo still works.
    doc.set_limits(HistoryLimits {
        max_bytes: step * 2,
        max_steps: usize::MAX,
    });
    assert_eq!((doc.undo_count(), doc.redo_count()), (1, 1));
    assert_eq!(doc.mesh().vertex(vertex).unwrap().position(), [4.0; 3]);
    doc.redo();
    assert_eq!(doc.mesh().vertex(vertex).unwrap().position(), [5.0; 3]);
    doc.undo();

    // Almost no room at all: one step survives.
    doc.set_limits(HistoryLimits {
        max_bytes: 1,
        max_steps: usize::MAX,
    });
    assert_eq!(doc.undo_count() + doc.redo_count(), 1);

    // A lower step limit also applies straight away.
    let (mut doc, _, _) = doc_with_cube();
    for i in 1..=5 {
        doc.edit("move", |m| m.set_vertex_position(vertex, [i as f32; 3]))
            .unwrap();
    }
    doc.set_limits(HistoryLimits {
        max_bytes: usize::MAX,
        max_steps: 2,
    });
    assert_eq!(doc.undo_count(), 2);
    assert_eq!(doc.history_bytes(), doc.undo_count() * step);
}

#[test]
fn memory_used_follows_the_edit_size_not_the_mesh_size() {
    let (mesh, v) = grid(100, 100);
    let mut doc = Document::new(mesh);
    doc.edit("one vertex", |m| m.set_vertex_position(v[0], [1.0; 3]))
        .unwrap();
    assert!(
        doc.history_bytes() < 2048,
        "moving one vertex in a 10,000-face mesh used {} bytes",
        doc.history_bytes()
    );
}

/// Errors are for showing to a person, so each one must have a readable message.
#[test]
fn errors_have_readable_messages() {
    let (mut mesh, v, f) = cube();
    let e = edge(&mesh, v[0], v[1]);
    mesh.remove_vertex(v[7]).unwrap();
    let errors = [
        MeshError::StaleVertex(v[0]),
        MeshError::StaleEdge(e),
        MeshError::StaleFace(f[0]),
        MeshError::NonFinitePosition,
        MeshError::InvalidFactor,
        MeshError::SameVertex(v[0]),
        MeshError::EdgeExists(e),
        MeshError::FaceTooSmall { count: 2 },
        MeshError::RepeatedVertex(v[0]),
        MeshError::FaceExists(f[0]),
        MeshError::VertexNotInFace {
            vertex: v[0],
            face: f[0],
        },
        MeshError::CornersAdjacent { face: f[0] },
        MeshError::NotTwoFaces { edge: e, faces: 3 },
        MeshError::InconsistentWinding(e),
        MeshError::JoinWouldRepeatVertex(v[0]),
        MeshError::EditInProgress,
        MeshError::NoEditInProgress,
        MeshError::InvalidPrimitive,
        MeshError::InvalidScale,
        MeshError::WrongSelectMode,
        MeshError::InvalidAxis,
        MeshError::NothingSelected,
        MeshError::NoFacesSelected,
        MeshError::CannotFill,
        MeshError::InvalidDistance,
        MeshError::IdsExhausted,
    ];
    let messages: Vec<String> = errors.iter().map(|error| error.to_string()).collect();
    for message in &messages {
        assert!(message.len() > 10, "too short: {message:?}");
    }
    let unique: std::collections::HashSet<_> = messages.iter().collect();
    assert_eq!(unique.len(), messages.len());
    assert!(
        MeshError::StaleVertex(v[0])
            .to_string()
            .contains(&v[0].to_string())
    );
}

/// Exactly at the cap is allowed: only going over it drops anything.
#[test]
fn history_may_fill_the_cap_exactly() {
    let (mut doc, v, _) = doc_with_cube();
    let vertex = v[0];
    for i in 1..=4 {
        doc.edit("move", |m| m.set_vertex_position(vertex, [i as f32; 3]))
            .unwrap();
    }
    let step = doc.history_bytes() / 4;

    doc.set_limits(HistoryLimits {
        max_bytes: step * 4,
        max_steps: usize::MAX,
    });
    assert_eq!(doc.undo_count(), 4);

    doc.undo();
    doc.undo();
    doc.undo();
    assert_eq!((doc.undo_count(), doc.redo_count()), (1, 3));
    doc.set_limits(HistoryLimits {
        max_bytes: step * 3,
        max_steps: usize::MAX,
    });
    assert_eq!((doc.undo_count(), doc.redo_count()), (1, 2));
    assert_eq!(doc.history_bytes(), step * 3);
    doc.set_limits(HistoryLimits {
        max_bytes: step * 2,
        max_steps: usize::MAX,
    });
    assert_eq!((doc.undo_count(), doc.redo_count()), (1, 1));
    assert_eq!(doc.history_bytes(), step * 2);
}

/// The only remaining step must survive even when it is a redo step.
#[test]
fn a_lone_redo_step_survives_a_tiny_cap() {
    let (mut doc, v, _) = doc_with_cube();
    let vertex = v[0];
    doc.edit("move", |m| m.set_vertex_position(vertex, [1.0; 3]))
        .unwrap();
    doc.undo();
    doc.set_limits(HistoryLimits {
        max_bytes: 1,
        max_steps: usize::MAX,
    });
    assert_eq!((doc.undo_count(), doc.redo_count()), (0, 1));
    assert_eq!(doc.redo(), Some("move"));
}

#[test]
fn the_version_tells_whether_the_mesh_is_back_where_it_was() {
    let mut doc = Document::with_limits(
        Mesh::new(),
        HistoryLimits {
            max_bytes: usize::MAX,
            max_steps: 2,
        },
    );
    let start = doc.version();
    doc.edit("a", |m| m.add_vertex([0.0; 3]).map(|_| ()))
        .unwrap();
    let one = doc.version();
    assert_ne!(one, start);
    // An edit that changes nothing keeps the version.
    doc.edit("nothing", |_| Ok(())).unwrap();
    assert_eq!(doc.version(), one);
    doc.undo();
    assert_eq!(doc.version(), start);
    doc.redo();
    assert_eq!(doc.version(), one);
    // A new edit after undoing gets a new version, even though the history is as long.
    doc.undo();
    doc.edit("b", |m| m.add_vertex([1.0; 3]).map(|_| ()))
        .unwrap();
    assert_ne!(doc.version(), one);
    assert_ne!(doc.version(), start);
    // Dropping old steps must not make the start look reached again.
    let saved = doc.version();
    for i in 0..4 {
        doc.edit("more", |m| m.add_vertex([i as f32; 3]).map(|_| ()))
            .unwrap();
    }
    while doc.undo().is_some() {}
    assert_ne!(doc.version(), start);
    assert_ne!(doc.version(), saved);
    // A cancelled edit leaves the version alone.
    let before = doc.version();
    doc.begin_edit("drag").unwrap();
    doc.active_mesh().unwrap().add_vertex([5.0; 3]).unwrap();
    doc.cancel_edit().unwrap();
    assert_eq!(doc.version(), before);
}
